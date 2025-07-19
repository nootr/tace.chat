use bytes::Bytes;
use http_body_util::Full;
use hyper::{Method, Request, Response, StatusCode};
use log::error;
use rusqlite::Connection;
use serde_json::json;
use std::sync::Arc;

use crate::db;

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

pub async fn metrics_api_handler(
    db_conn: Arc<tokio::sync::Mutex<Connection>>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    if req.method() == Method::OPTIONS {
        return Ok(format_response(StatusCode::OK, ""));
    }

    let conn = db_conn.lock().await;
    match db::get_latest_metrics(&conn) {
        Ok(metrics) => match serde_json::to_string(&metrics) {
            Ok(json_response) => Ok(format_response(StatusCode::OK, json_response)),
            Err(e) => {
                error!("Failed to serialize metrics to JSON: {}", e);
                Ok(format_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({ "error": "Failed to serialize metrics" }).to_string(),
                ))
            }
        },
        Err(e) => {
            error!("Failed to retrieve metrics from DB: {}", e);
            Ok(format_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": "Internal server error" }).to_string(),
            ))
        }
    }
}
