mod api;
mod network_client;
mod node;

use api::run;
use network_client::RealNetworkClient;
use node::ChordNode;
use std::env;
use std::sync::Arc;
use tokio::signal;

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
        .unwrap_or_else(|_| {
            eprintln!("Error: BIND_P2P_PORT must be a valid integer, using default 6512");
            6512
        });
    let advertise_p2p_host = env::var("ADVERTISE_P2P_HOST")
        .or_else(|_| env::var("ADVERTISE_HOST"))
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            eprintln!("Error: ADVERTISE_P2P_HOST must be set, using localhost as fallback");
            "localhost".to_string()
        });
    let advertise_p2p_port: u16 = env::var("ADVERTISE_P2P_PORT")
        .unwrap_or_else(|_| bind_p2p_port.to_string())
        .parse()
        .unwrap_or(bind_p2p_port);

    // API Configuration
    let bind_api_host = env::var("BIND_API_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let bind_api_port: u16 = env::var("BIND_API_PORT")
        .or_else(|_| env::var("API_PORT"))
        .unwrap_or_else(|_| "80".to_string())
        .parse()
        .unwrap_or_else(|_| {
            eprintln!("Error: BIND_API_PORT must be a valid integer, using default 80");
            80
        });
    let advertise_api_host = env::var("ADVERTISE_API_HOST")
        .or_else(|_| env::var("API_HOST"))
        .unwrap_or_else(|_| advertise_p2p_host.clone());
    let advertise_api_port: u16 = env::var("ADVERTISE_API_PORT")
        .unwrap_or_else(|_| "443".to_string())
        .parse()
        .unwrap_or_else(|_| {
            eprintln!("Error: ADVERTISE_API_PORT must be a valid integer, using default 443");
            443
        });
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
        .unwrap_or_else(|_| {
            eprintln!("Error: STABILIZATION_INTERVAL must be a valid integer, using default 5");
            5
        });

    let bind_p2p_address = format!("{}:{}", bind_p2p_host, bind_p2p_port);
    let advertise_p2p_address = format!("{}:{}", advertise_p2p_host, advertise_p2p_port);
    let advertise_api_address = format!("{}:{}", advertise_api_host, advertise_api_port);

    let node = Arc::new(
        ChordNode::new(
            advertise_p2p_address,
            advertise_api_address,
            Arc::new(RealNetworkClient),
        )
        .await,
    );

    let node_for_api = node.clone();
    let bind_api_host_clone = bind_api_host.clone();
    tokio::spawn(async move {
        if let Err(e) = run(&bind_api_host_clone, bind_api_port, node_for_api).await {
            log::error!("Failed to run API server: {}", e);
            std::process::exit(1);
        }
    });

    node.join(bootstrap_address).await;

    let node_clone_for_tasks = node.clone();
    let node_clone_for_shutdown = node.clone();

    // Spawn stabilization task
    let stabilization_handle = tokio::spawn(async move {
        let node = node_clone_for_tasks;

        loop {
            // Check for shutdown signal
            if node
                .shutdown_signal
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                log::info!("Stabilization task shutting down");
                break;
            }

            node.stabilize().await;
            node.fix_fingers().await;
            node.check_predecessor().await;
            if log::log_enabled!(log::Level::Debug) {
                node.debug_ring_state(); // Debug ring formation
            }
            node.share_and_update_metrics().await;
            node.check_and_rebalance().await;
            tokio::time::sleep(tokio::time::Duration::from_secs(stabilization_interval)).await;
        }
    });

    // Spawn signal handler
    let signal_handle = tokio::spawn(async move {
        let mut sigint = match signal::unix::signal(signal::unix::SignalKind::interrupt()) {
            Ok(signal) => signal,
            Err(e) => {
                log::error!("Failed to register SIGINT handler: {}", e);
                return;
            }
        };
        let mut sigterm = match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(e) => {
                log::error!("Failed to register SIGTERM handler: {}", e);
                return;
            }
        };

        tokio::select! {
            _ = sigint.recv() => {
                log::info!("Received SIGINT (Ctrl+C), initiating graceful shutdown");
            }
            _ = sigterm.recv() => {
                log::info!("Received SIGTERM, initiating graceful shutdown");
            }
        }

        node_clone_for_shutdown.shutdown();
    });

    // Start the P2P server (this will block until shutdown)
    let server_handle = tokio::spawn(async move {
        node.start(&bind_p2p_address).await;
    });

    // Wait for either the server to finish or shutdown signal
    tokio::select! {
        _ = server_handle => {
            log::info!("P2P server stopped");
        }
        _ = signal_handle => {
            log::info!("Shutdown signal received");
        }
    }

    // Wait for stabilization task to finish
    if let Err(e) = stabilization_handle.await {
        log::error!("Error waiting for stabilization task: {}", e);
    }

    log::info!("Node shutdown complete");
}
