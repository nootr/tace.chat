use std::convert::Infallible;
use std::net::SocketAddr;

use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{
    body::{Body, Bytes},
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpListener;

#[derive(Deserialize, Debug)]
struct MessagePayload {
    to: String,
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
}

fn format_response(
    status: StatusCode,
    body: impl Into<Bytes>,
) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(body.into()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
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

async fn message_handler(
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let body_bytes = match req.collect().await {
        Ok(body) => body.to_bytes(),
        Err(e) => {
            eprintln!("Error reading request body: {}", e);
            return Ok(format_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Failed to read request body" }).to_string(),
            ));
        }
    };

    let message_payload: MessagePayload = match serde_json::from_slice(&body_bytes) {
        Ok(payload) => payload,
        Err(e) => {
            eprintln!("Failed to deserialize message payload: {}", e);
            return Ok(format_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Invalid JSON payload" }).to_string(),
            ));
        }
    };

    println!("Received message: {:?}", message_payload);

    let response_body = json!({ "status": "message received" }).to_string();
    Ok(format_response(StatusCode::OK, response_body))
}

async fn handler(req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/ping") => Ok(ping(req)),
        (&Method::POST, "/message") => message_handler(req).await,
        _ => Ok(not_found()),
    }
}

pub async fn run(port: u16) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let listener = TcpListener::bind(addr).await?;

    println!("API is listening on http://{}", addr);
    loop {
        let (tcp, _) = listener.accept().await?;
        let io = TokioIo::new(tcp);
        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(handler))
                .await
            {
                println!("Error serving connection: {:?}", err);
            }
        });
    }
}
