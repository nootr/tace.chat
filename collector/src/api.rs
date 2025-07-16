use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode};
use log::error;
use rusqlite::Connection;
use std::sync::Arc;

use crate::db;

pub async fn metrics_api_handler(
    db_conn: Arc<tokio::sync::Mutex<Connection>>,
    _req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let conn = db_conn.lock().await;
    match db::get_latest_metrics(&conn) {
        Ok(metrics) => {
            let json_response = serde_json::to_string(&metrics).unwrap();
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(hyper::header::CONTENT_TYPE, "application/json")
                .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*") // Add CORS header
                .body(Full::new(Bytes::from(json_response)))
                .unwrap())
        }
        Err(e) => {
            error!("Failed to retrieve metrics from DB: {}", e);
            Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from("Internal server error")))
                .unwrap())
        }
    }
}
