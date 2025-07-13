mod api;
mod network_client;
mod node;

use api::run;
use network_client::RealNetworkClient;
use node::ChordNode;
use std::env;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Initialize logger
    env_logger::init();

    let advertise_address =
        env::var("NODE_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8000".to_string());
    let (advertise_host, advertise_port) = advertise_address
        .split_once(':')
        .expect("NODE_ADDRESS must be in the format HOST:PORT");
    let bind_address =
        env::var("BIND_ADDRESS").unwrap_or_else(|_| format!("0.0.0.0:{}", advertise_port));
    let bootstrap_address = env::var("BOOTSTRAP_ADDRESS").ok();
    let api_port: u16 = env::var("API_PORT")
        .unwrap_or_else(|_| "8001".to_string())
        .parse()
        .expect("API_PORT must be a valid u16");
    let api_address = format!("{}:{}", advertise_host, api_port);

    let node =
        Arc::new(ChordNode::new(advertise_address, api_address, Arc::new(RealNetworkClient)).await);

    let node_for_api = node.clone();
    tokio::spawn(async move {
        run(api_port, node_for_api)
            .await
            .expect("Failed to run API server");
    });

    node.join(bootstrap_address).await;

    let node_clone_for_tasks = node.clone();

    tokio::spawn(async move {
        let node = node_clone_for_tasks;

        loop {
            node.stabilize().await;
            node.fix_fingers().await;
            node.check_predecessor().await;
            if log::log_enabled!(log::Level::Debug) {
                node.debug_ring_state(); // Debug ring formation
            }
            node.estimate_network_size().await;
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }
    });

    node.start(&bind_address).await;
}
