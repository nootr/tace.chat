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
    env_logger::init();

    let bind_host = env::var("BIND_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let advertise_host = env::var("ADVERTISE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let node_port: u16 = env::var("NODE_PORT")
        .unwrap_or_else(|_| "6512".to_string())
        .parse()
        .expect("BIND_PORT must be a valid integer");
    let api_host = env::var("API_HOST").unwrap_or_else(|_| advertise_host.clone());
    let api_port: u16 = env::var("API_PORT")
        .unwrap_or_else(|_| "6345".to_string())
        .parse()
        .expect("BIND_PORT must be a valid integer");
    let is_bootstrap = env::var("IS_BOOTSTRAP")
        .map(|v| v == "true")
        .unwrap_or(false);
    let bootstrap_address = if is_bootstrap {
        None
    } else {
        Some(match env::var("BOOTSTRAP_ADDRESS") {
            Ok(addr) => addr,
            _ => "bootstrap.tace.chat:6512".to_string(),
        })
    };
    let stabilization_interval: u64 = env::var("STABILIZATION_INTERVAL")
        .unwrap_or_else(|_| "5".to_string())
        .parse()
        .expect("STABILIZATION_INTERVAL must be a valid integer");

    let bind_address = format!("{}:{}", bind_host, node_port);
    let advertise_address = format!("{}:{}", advertise_host, node_port);
    let api_address = format!("{}:{}", api_host, api_port);

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
            node.share_and_update_metrics().await;
            tokio::time::sleep(tokio::time::Duration::from_secs(stabilization_interval)).await;
        }
    });

    node.start(&bind_address).await;
}
