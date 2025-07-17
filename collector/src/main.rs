use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use log::error;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::time::{self, Duration};

mod api;
mod db;
mod metrics_fetcher;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let bootstrap_node_api_url = std::env::var("BOOTSTRAP_NODE_API_URL")
        .unwrap_or_else(|_| "http://bootstrap.tace.chat:6345".to_string());
    let collector_bind_address =
        std::env::var("COLLECTOR_BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8000".to_string());
    let metrics_fetch_interval_secs: u64 = std::env::var("METRICS_FETCH_INTERVAL_SECS")
        .unwrap_or_else(|_| "60".to_string())
        .parse()?;

    let db_path =
        std::env::var("SQLITE_DB_PATH").unwrap_or_else(|_| "./metrics.sqlite".to_string());
    let conn = db::init_db(&db_path)?;
    let db_conn = Arc::new(tokio::sync::Mutex::new(conn));

    // Spawn task to periodically fetch and store metrics
    let db_conn_clone = db_conn.clone();
    let bootstrap_node_api_url_clone = bootstrap_node_api_url.clone();
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(metrics_fetch_interval_secs));
        interval.tick().await; // Initial tick to run immediately
        loop {
            metrics_fetcher::fetch_and_store_metrics(
                bootstrap_node_api_url_clone.clone(),
                db_conn_clone.clone(),
            )
            .await;
            interval.tick().await;
        }
    });

    // Start HTTP server to serve metrics
    let addr: std::net::SocketAddr = collector_bind_address.parse()?;
    let listener = TcpListener::bind(addr).await?;

    let db_conn_for_service = db_conn.clone();
    loop {
        let (tcp, _) = listener.accept().await?;
        let io = TokioIo::new(tcp);
        let db_conn_clone = db_conn_for_service.clone();
        tokio::task::spawn(async move {
            if let Err(err) = hyper::server::conn::http1::Builder::new()
                .serve_connection(
                    io,
                    service_fn(move |req| api::metrics_api_handler(db_conn_clone.clone(), req)),
                )
                .await
            {
                error!("Error serving collector API connection: {:?}", err);
            }
        });
    }
}
