use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{
    body::{Body, Bytes},
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha1::{Digest, Sha1};
use tace_lib::dht_messages::{DhtMessage, NodeId};
use tokio::net::TcpListener;

use crate::{network_client::NetworkClient, ChordNode};

#[derive(Deserialize, Debug)]
struct MessagePayload {
    to: String,
    from: String, // Sender's public key
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
}

#[derive(Serialize, Debug)]
struct PollMessage {
    from: String,
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    timestamp: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct StoredMessage {
    from: String,
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    timestamp: u64,
}

fn generate_key(public_key: &str) -> NodeId {
    let mut hasher = Sha1::new();
    hasher.update(public_key.as_bytes());
    hasher.finalize().into()
}

fn format_response(status: StatusCode, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(body.into()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        hyper::header::HeaderValue::from_static("*"),
    );
    response.headers_mut().insert(
        hyper::header::ACCESS_CONTROL_ALLOW_HEADERS,
        hyper::header::HeaderValue::from_static("content-type"),
    );
    response.headers_mut().insert(
        hyper::header::ACCESS_CONTROL_ALLOW_METHODS,
        hyper::header::HeaderValue::from_static("POST, OPTIONS"),
    );
    response
}

fn not_found() -> Response<Full<Bytes>> {
    let mut response = format_response(
        StatusCode::NOT_FOUND,
        json!({ "error": "Not Found" }).to_string(),
    );
    *response.status_mut() = StatusCode::NOT_FOUND;
    response
}

fn ping(_req: Request<impl Body>) -> Response<Full<Bytes>> {
    let response_body = json!({ "message": "pong" }).to_string();
    format_response(StatusCode::OK, response_body)
}

async fn message_handler<T: NetworkClient>(
    req: Request<hyper::body::Incoming>,
    node: Arc<ChordNode<T>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let body_bytes = match req.collect().await {
        Ok(body) => body.to_bytes(),
        Err(e) => {
            error!("Error reading request body: {}", e);
            return Ok(format_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Failed to read request body" }).to_string(),
            ));
        }
    };

    let message_payload: MessagePayload = match serde_json::from_slice(&body_bytes) {
        Ok(payload) => payload,
        Err(e) => {
            error!("Failed to deserialize message payload: {}", e);
            return Ok(format_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Invalid JSON payload" }).to_string(),
            ));
        }
    };

    debug!("Received message: {:?}", message_payload);

    // Generate key from recipient's public key
    let key = generate_key(&message_payload.to);

    // Create message to store in DHT
    let stored_message = StoredMessage {
        from: message_payload.from, // Store sender's public key
        ciphertext: message_payload.ciphertext,
        nonce: message_payload.nonce,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };

    let serialized = bincode::serialize(&stored_message).unwrap();

    // Find the node responsible for this key
    let responsible_node = node.find_successor(key).await;

    if responsible_node.id == node.info.id {
        // This node is responsible, store locally
        node.store(key, serialized).await;
    } else {
        // Send to responsible node
        match node
            .network_client
            .call_node(
                &responsible_node.address,
                DhtMessage::Store {
                    key,
                    value: serialized,
                },
            )
            .await
        {
            Ok(_) => debug!(
                "Message stored on node {}",
                hex::encode(responsible_node.id)
            ),
            Err(e) => {
                error!("Failed to store message on remote node: {}", e);
                return Ok(format_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({ "error": "Failed to store message" }).to_string(),
                ));
            }
        }
    }

    let response_body = json!({ "status": "message received" }).to_string();
    Ok(format_response(StatusCode::OK, response_body))
}

async fn poll_handler<T: NetworkClient>(
    req: Request<hyper::body::Incoming>,
    node: Arc<ChordNode<T>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    // Extract public key from query parameters
    let uri = req.uri();
    let query = uri.query().unwrap_or("");
    let params: Vec<(&str, &str)> = query
        .split('&')
        .filter_map(|s| {
            let mut parts = s.split('=');
            Some((parts.next()?, parts.next()?))
        })
        .collect();

    let public_key = params
        .iter()
        .find(|(k, _)| k == &"public_key")
        .map(|(_, v)| v);

    if public_key.is_none() {
        return Ok(format_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing public_key parameter" }).to_string(),
        ));
    }

    let key = generate_key(public_key.unwrap());

    // Find the node responsible for this key
    let responsible_node = node.find_successor(key).await;

    let messages_data = if responsible_node.id == node.info.id {
        // This node is responsible, retrieve locally
        node.retrieve(key).await
    } else {
        // Request from responsible node
        match node
            .network_client
            .call_node(&responsible_node.address, DhtMessage::Retrieve { key })
            .await
        {
            Ok(DhtMessage::Retrieved { key: _, value }) => value,
            _ => None,
        }
    };

    let messages = if let Some(data) = messages_data {
        // Deserialize the stored message
        match bincode::deserialize::<StoredMessage>(&data) {
            Ok(stored_msg) => vec![PollMessage {
                from: stored_msg.from,
                ciphertext: stored_msg.ciphertext,
                nonce: stored_msg.nonce,
                timestamp: stored_msg.timestamp,
            }],
            Err(_) => vec![],
        }
    } else {
        vec![]
    };

    let response_body = json!({ "messages": messages }).to_string();
    Ok(format_response(StatusCode::OK, response_body))
}

async fn handler<T: NetworkClient + Send + Sync + 'static>(
    req: Request<hyper::body::Incoming>,
    node: Arc<ChordNode<T>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/ping") => Ok(ping(req)),
        (&Method::POST, "/message") => message_handler(req, node).await,
        (&Method::GET, "/poll") => poll_handler(req, node).await,
        (&Method::OPTIONS, _) => Ok(format_response(StatusCode::OK, "")),
        _ => Ok(not_found()),
    }
}

pub async fn run<T: NetworkClient + Send + Sync + 'static>(
    port: u16,
    node: Arc<ChordNode<T>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let listener = TcpListener::bind(addr).await?;

    info!("API is listening on http://{}", addr);
    loop {
        let (tcp, _) = listener.accept().await?;
        let io = TokioIo::new(tcp);
        let node_clone = node.clone();
        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(move |req| handler(req, node_clone.clone())))
                .await
            {
                error!("Error serving connection: {:?}", err);
            }
        });
    }
}
