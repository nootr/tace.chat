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

    // P2P Configuration
    let bind_p2p_host = env::var("BIND_P2P_HOST")
        .or_else(|_| env::var("BIND_HOST"))
        .unwrap_or_else(|_| "0.0.0.0".to_string());
    let bind_p2p_port: u16 = env::var("BIND_P2P_PORT")
        .or_else(|_| env::var("NODE_PORT"))
        .unwrap_or_else(|_| "6512".to_string())
        .parse()
        .expect("BIND_P2P_PORT must be a valid integer");
    let advertise_p2p_host = env::var("ADVERTISE_P2P_HOST")
        .or_else(|_| env::var("ADVERTISE_HOST"))
        .ok()
        .filter(|s| !s.is_empty())
        .expect("ADVERTISE_P2P_HOST must be set (no default)");
    let advertise_p2p_port: u16 = env::var("ADVERTISE_P2P_PORT")
        .unwrap_or_else(|_| bind_p2p_port.to_string())
        .parse()
        .expect("ADVERTISE_P2P_PORT must be a valid integer");

    // API Configuration
    let bind_api_host = env::var("BIND_API_HOST")
        .unwrap_or_else(|_| "0.0.0.0".to_string());
    let bind_api_port: u16 = env::var("BIND_API_PORT")
        .or_else(|_| env::var("API_PORT"))
        .unwrap_or_else(|_| "80".to_string())
        .parse()
        .expect("BIND_API_PORT must be a valid integer");
    let advertise_api_host = env::var("ADVERTISE_API_HOST")
        .or_else(|_| env::var("API_HOST"))
        .unwrap_or_else(|_| advertise_p2p_host.clone());
    let advertise_api_port: u16 = env::var("ADVERTISE_API_PORT")
        .unwrap_or_else(|_| "443".to_string())
        .parse()
        .expect("ADVERTISE_API_PORT must be a valid integer");
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

    let bind_p2p_address = format!("{}:{}", bind_p2p_host, bind_p2p_port);
    let advertise_p2p_address = format!("{}:{}", advertise_p2p_host, advertise_p2p_port);
    let advertise_api_address = format!("{}:{}", advertise_api_host, advertise_api_port);

    let node =
        Arc::new(ChordNode::new(advertise_p2p_address, advertise_api_address, Arc::new(RealNetworkClient)).await);

    let node_for_api = node.clone();
    let bind_api_host_clone = bind_api_host.clone();
    tokio::spawn(async move {
        run(&bind_api_host_clone, bind_api_port, node_for_api)
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

    node.start(&bind_p2p_address).await;
}
