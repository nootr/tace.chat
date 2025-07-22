use http_body_util::BodyExt;
use hyper::Uri;
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use log::{debug, error};
use rusqlite::Connection;
use std::sync::Arc;
use tace_lib::metrics::NetworkMetrics;

use crate::db;

pub async fn fetch_and_store_metrics(
    bootstrap_node_api_url: String,
    db_conn: Arc<tokio::sync::Mutex<Connection>>,
) {
    let uri: Uri = match format!("{}/metrics", bootstrap_node_api_url).parse() {
        Ok(uri) => uri,
        Err(e) => {
            error!("Failed to parse URI: {}", e);
            return;
        }
    };

    // Create HTTPS client that supports both HTTP and HTTPS
    let https = HttpsConnector::new();
    let client: Client<HttpsConnector<HttpConnector>, http_body_util::Full<bytes::Bytes>> =
        Client::builder(TokioExecutor::new()).build(https);

    debug!("Fetching metrics from {}", uri);

    match client.get(uri).await {
        Ok(res) => {
            if res.status().is_success() {
                let body_bytes = match res.into_body().collect().await {
                    Ok(collected) => collected.to_bytes(),
                    Err(e) => {
                        error!("Failed to read response body: {}", e);
                        return;
                    }
                };
                match serde_json::from_slice::<NetworkMetrics>(&body_bytes) {
                    Ok(metrics) => {
                        debug!("Successfully fetched metrics: {:?}", metrics);
                        let conn = db_conn.lock().await;
                        if let Err(e) = db::insert_metrics(&conn, &metrics) {
                            error!("Failed to insert metrics into DB: {}", e);
                        }
                    }
                    Err(e) => error!("Failed to deserialize metrics: {}", e),
                }
            } else {
                error!("Failed to fetch metrics: HTTP status {}", res.status());
            }
        }
        Err(e) => error!("Failed to connect to bootstrap node: {}", e),
    }
}
