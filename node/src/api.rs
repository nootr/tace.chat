use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{
    body::{Body, Bytes},
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use log::{debug, error, info};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha1::{Digest, Sha1};
use tace_lib::dht_messages::{DhtMessage, NodeId};
use tace_lib::keys;
use tokio::net::TcpListener;
use tokio::sync::OnceCell;

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

#[derive(Serialize, Deserialize, Debug)]
struct PollRequestPayload {
    public_key: String,
    nonce: Vec<u8>,
    signature: Vec<u8>,
}

// Store challenges with a timestamp to prevent them from living forever
struct Challenge {
    nonce: Vec<u8>,
    timestamp: Instant,
}

type ChallengeMap = Arc<Mutex<HashMap<String, Challenge>>>;

static CHALLENGES: OnceCell<ChallengeMap> = OnceCell::const_new();
const CHALLENGE_EXPIRATION: Duration = Duration::from_secs(60);

async fn get_challenges() -> &'static ChallengeMap {
    CHALLENGES
        .get_or_init(|| async { Arc::new(Mutex::new(HashMap::new())) })
        .await
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
        hyper::header::HeaderValue::from_static("GET, POST, OPTIONS"),
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

fn ping_handler(_req: Request<impl Body>) -> Response<Full<Bytes>> {
    let response_body = json!({ "message": "pong" }).to_string();
    format_response(StatusCode::OK, response_body)
}

async fn connect_handler<T: NetworkClient>(
    node: Arc<ChordNode<T>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    const MAX_ATTEMPTS: u8 = 10;
    for _ in 0..MAX_ATTEMPTS {
        // Generate a random key
        let mut buf = [0u8; 20];
        OsRng.fill_bytes(&mut buf);

        let mut hasher = Sha1::new();
        hasher.update(buf);
        let key = hasher.finalize().into();

        // Find the node responsible for this key
        let responsible_node = node.find_successor(key).await;

        // If it's ourself, we are reachable
        if responsible_node.id == node.info.id {
            let response_body = json!({ "node": responsible_node.api_address }).to_string();
            return Ok(format_response(StatusCode::OK, response_body));
        }

        // Ping the node to check if it's alive
        match node
            .network_client
            .call_node(&responsible_node.address, DhtMessage::Ping)
            .await
        {
            Ok(DhtMessage::Pong) => {
                // Node is alive, return its address
                let response_body = json!({ "node": responsible_node.api_address }).to_string();
                return Ok(format_response(StatusCode::OK, response_body));
            }
            _ => {
                // Node is not reachable, try another one
                debug!(
                    "Node {} at {} is not reachable, trying another one.",
                    hex::encode(responsible_node.id),
                    responsible_node.address
                );
                continue;
            }
        }
    }

    // If we reach here, we failed to find a reachable node
    error!(
        "Failed to find a reachable node after {} attempts.",
        MAX_ATTEMPTS
    );
    Ok(format_response(
        StatusCode::SERVICE_UNAVAILABLE,
        json!({ "error": "Could not find a reachable node in the network" }).to_string(),
    ))
}

async fn get_challenge_handler<B>(req: Request<B>) -> Result<Response<Full<Bytes>>, Infallible> {
    let public_key = req
        .uri()
        .query()
        .and_then(|q| q.split('&').find(|p| p.starts_with("public_key=")))
        .and_then(|p| p.split('=').nth(1))
        .map(|s| s.to_string());

    let Some(public_key) = public_key else {
        return Ok(format_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing public_key parameter" }).to_string(),
        ));
    };

    let challenges_map = get_challenges().await;
    let mut challenges = challenges_map.lock().unwrap();

    // Clean up expired challenges
    challenges.retain(|_, c| c.timestamp.elapsed() < CHALLENGE_EXPIRATION);

    // Generate a new challenge
    let mut nonce = vec![0u8; 32];
    OsRng.fill_bytes(&mut nonce);
    let challenge = Challenge {
        nonce: nonce.clone(),
        timestamp: Instant::now(),
    };
    challenges.insert(public_key, challenge);

    info!("Issued challenge for public key");
    let response_body = json!({ "nonce": hex::encode(nonce) }).to_string();
    Ok(format_response(StatusCode::OK, response_body))
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

async fn poll_handler<T: NetworkClient, B>(
    req: Request<B>,
    node: Arc<ChordNode<T>>,
) -> Result<Response<Full<Bytes>>, Infallible>
where
    B: Body + Send + 'static,
    B::Error: std::error::Error + Send + Sync + 'static,
{
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

    let poll_payload: PollRequestPayload = match serde_json::from_slice(&body_bytes) {
        Ok(payload) => payload,
        Err(e) => {
            error!("Failed to deserialize poll payload: {}", e);
            return Ok(format_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Invalid JSON payload" }).to_string(),
            ));
        }
    };

    // Verify the challenge
    let is_valid = {
        let challenges_map = get_challenges().await;
        let mut challenges = challenges_map.lock().unwrap();
        if let Some(challenge) = challenges.get(&poll_payload.public_key) {
            if challenge.nonce == poll_payload.nonce {
                if keys::verify(
                    &poll_payload.public_key,
                    &poll_payload.nonce,
                    &poll_payload.signature,
                ) {
                    // Valid signature, remove challenge and proceed
                    challenges.remove(&poll_payload.public_key);
                    true
                } else {
                    false // Invalid signature
                }
            } else {
                false // Nonce mismatch
            }
        } else {
            false // No challenge found
        }
    };

    if !is_valid {
        return Ok(format_response(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "Invalid challenge response" }).to_string(),
        ));
    }

    let key = generate_key(&poll_payload.public_key);

    // Find the node responsible for this key
    let responsible_node = node.find_successor(key).await;

    let messages_data = if responsible_node.id == node.info.id {
        // This node is responsible, retrieve locally
        node.retrieve(key).await
    } else {
        match node
            .network_client
            .call_node(&responsible_node.address, DhtMessage::Retrieve { key })
            .await
        {
            Ok(DhtMessage::Retrieved { key: _, value }) => value,
            _ => None,
        }
    };

    let messages = if let Some(data_vec) = messages_data {
        // Deserialize the stored messages
        data_vec
            .into_iter()
            .filter_map(|data| {
                bincode::deserialize::<StoredMessage>(&data)
                    .map(|stored_msg| PollMessage {
                        from: stored_msg.from,
                        ciphertext: stored_msg.ciphertext,
                        nonce: stored_msg.nonce,
                        timestamp: stored_msg.timestamp,
                    })
                    .ok()
            })
            .collect()
    } else {
        vec![]
    };

    let response_body = json!({ "messages": messages }).to_string();
    Ok(format_response(StatusCode::OK, response_body))
}

async fn metrics_handler<T: NetworkClient>(
    node: Arc<ChordNode<T>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let metrics = node.metrics.lock().unwrap().clone();
    let response_body = serde_json::to_string(&metrics).unwrap();
    Ok(format_response(StatusCode::OK, response_body))
}

async fn handler<T: NetworkClient + Send + Sync + 'static>(
    req: Request<hyper::body::Incoming>,
    node: Arc<ChordNode<T>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/ping") => Ok(ping_handler(req)),
        (&Method::GET, "/connect") => connect_handler(node.clone()).await,
        (&Method::GET, "/poll/challenge") => get_challenge_handler(req).await,
        (&Method::POST, "/message") => message_handler(req, node.clone()).await,
        (&Method::POST, "/poll") => poll_handler(req, node.clone()).await,
        (&Method::GET, "/metrics") => metrics_handler(node.clone()).await,
        (&Method::OPTIONS, _) => {
            let mut response = format_response(StatusCode::OK, "");
            response.headers_mut().insert(
                hyper::header::ACCESS_CONTROL_MAX_AGE,
                hyper::header::HeaderValue::from_static("86400"),
            );
            Ok(response)
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_client::MockNetworkClient;
    use http_body_util::BodyExt;
    use serde_json::Value;

    #[tokio::test]
    async fn test_connect_handler_basic() {
        let mock_client = MockNetworkClient::new();
        let node = Arc::new(ChordNode::new_for_test(
            "127.0.0.1:8000".to_string(),
            Arc::new(mock_client),
        ));

        let response = connect_handler(node).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["node"], "127.0.0.1:8000");
    }

    #[tokio::test]
    async fn test_get_challenge_handler() {
        let keypair = keys::generate_keypair();
        let req = Request::builder()
            .uri(format!("/poll/challenge?public_key={}", keypair.public_key))
            .body(http_body_util::Empty::<Bytes>::new())
            .unwrap();

        let response = get_challenge_handler(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&body).unwrap();
        let nonce_hex = v["nonce"].as_str().unwrap();
        assert_eq!(nonce_hex.len(), 64); // 32 bytes = 64 hex chars
    }

    #[tokio::test]
    async fn test_poll_handler_success() {
        let keypair = keys::generate_keypair();
        let mock_client = MockNetworkClient::new();
        let node = Arc::new(ChordNode::new_for_test(
            "0.0.0.0:0".to_string(),
            Arc::new(mock_client),
        ));

        // 1. Get a challenge
        let challenges_map = get_challenges().await;
        let nonce = {
            let mut challenges = challenges_map.lock().unwrap();
            let mut nonce = vec![0u8; 32];
            OsRng.fill_bytes(&mut nonce);
            challenges.insert(
                keypair.public_key.clone(),
                Challenge {
                    nonce: nonce.clone(),
                    timestamp: Instant::now(),
                },
            );
            nonce
        };

        // 2. Sign the challenge
        let signature = keys::sign(&keypair.private_key, &nonce).unwrap();

        // 3. Create the poll request
        let payload = PollRequestPayload {
            public_key: keypair.public_key.clone(),
            nonce,
            signature,
        };
        let req = Request::builder()
            .method(Method::POST)
            .uri("/poll")
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(
                serde_json::to_vec(&payload).unwrap(),
            )))
            .unwrap();

        // 4. Call the handler
        let response = poll_handler(req, node).await.unwrap();

        // 5. Assert the response is OK
        assert_eq!(response.status(), StatusCode::OK);

        // 6. Assert the challenge was consumed
        let challenges = challenges_map.lock().unwrap();
        assert!(!challenges.contains_key(&keypair.public_key));
    }

    #[tokio::test]
    async fn test_poll_handler_invalid_signature() {
        let keypair = keys::generate_keypair();
        let mock_client = MockNetworkClient::new();
        let node = Arc::new(ChordNode::new_for_test(
            "0.0.0.0:0".to_string(),
            Arc::new(mock_client),
        ));

        // 1. Get a challenge
        let challenges_map = get_challenges().await;
        let nonce = {
            let mut challenges = challenges_map.lock().unwrap();
            let mut nonce = vec![0u8; 32];
            OsRng.fill_bytes(&mut nonce);
            challenges.insert(
                keypair.public_key.clone(),
                Challenge {
                    nonce: nonce.clone(),
                    timestamp: Instant::now(),
                },
            );
            nonce
        };

        // 2. Sign a DIFFERENT message
        let bad_signature = keys::sign(&keypair.private_key, b"wrong message").unwrap();

        // 3. Create the poll request
        let payload = PollRequestPayload {
            public_key: keypair.public_key.clone(),
            nonce,
            signature: bad_signature,
        };
        let req = Request::builder()
            .method(Method::POST)
            .uri("/poll")
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(
                serde_json::to_vec(&payload).unwrap(),
            )))
            .unwrap();

        // 4. Call the handler
        let response = poll_handler(req, node).await.unwrap();

        // 5. Assert the response is UNAUTHORIZED
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
