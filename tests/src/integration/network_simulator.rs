use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tace_lib::dht_messages::DhtMessage;
use tace_node::NetworkClient;
use tokio::sync::{mpsc, RwLock};

/// Request ID for correlating requests and responses
pub type RequestId = u64;

/// Simulates network communication between nodes in the test environment
/// Routes DHT messages between nodes without actual TCP connections
#[derive(Clone)]
pub struct NetworkSimulator {
    /// Maps node addresses to message channels
    nodes: Arc<RwLock<HashMap<String, mpsc::UnboundedSender<SimulatorMessage>>>>,
    /// Controls message delivery timing and failures
    delivery_controller: Arc<RwLock<DeliveryController>>,
    /// Pending requests waiting for responses
    pending_requests: Arc<RwLock<HashMap<RequestId, mpsc::UnboundedSender<DhtMessage>>>>,
    /// Request ID counter
    next_request_id: Arc<AtomicU64>,
}

/// Message sent through the simulator
#[derive(Debug, Clone)]
pub enum SimulatorMessage {
    /// Request message with correlation ID
    Request {
        from: String,
        message: DhtMessage,
        request_id: RequestId,
        response_sender: mpsc::UnboundedSender<DhtMessage>,
    },
    /// Response message
    Response {
        message: DhtMessage,
        request_id: RequestId,
    },
}

#[derive(Default)]
struct DeliveryController {
    /// Addresses that should fail message delivery
    failed_nodes: std::collections::HashSet<String>,
    /// Simulated network latency in milliseconds
    latency_ms: u64,
    /// Drop rate for messages (0.0 to 1.0)
    drop_rate: f64,
}

impl Default for NetworkSimulator {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkSimulator {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            delivery_controller: Arc::new(RwLock::new(DeliveryController::default())),
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            next_request_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Register a node with the simulator
    pub async fn register_node(
        &self,
        address: String,
        sender: mpsc::UnboundedSender<SimulatorMessage>,
    ) {
        self.nodes.write().await.insert(address, sender);
    }

    /// Remove a node from the simulator (simulates node failure)
    pub async fn unregister_node(&self, address: &str) {
        self.nodes.write().await.remove(address);
    }

    /// Mark a node as failed (messages will not be delivered)
    pub async fn mark_node_failed(&self, address: &str) {
        self.delivery_controller
            .write()
            .await
            .failed_nodes
            .insert(address.to_string());
    }

    /// Mark a node as recovered
    pub async fn mark_node_recovered(&self, address: &str) {
        self.delivery_controller
            .write()
            .await
            .failed_nodes
            .remove(address);
    }

    /// Set simulated network latency
    pub async fn set_latency(&self, latency_ms: u64) {
        self.delivery_controller.write().await.latency_ms = latency_ms;
    }

    /// Set message drop rate (0.0 = no drops, 1.0 = drop all)
    pub async fn set_drop_rate(&self, drop_rate: f64) {
        self.delivery_controller.write().await.drop_rate = drop_rate.clamp(0.0, 1.0);
    }

    /// Get list of registered node addresses
    pub async fn get_registered_nodes(&self) -> Vec<String> {
        self.nodes.read().await.keys().cloned().collect()
    }

    /// Check if a node is registered
    pub async fn is_node_registered(&self, address: &str) -> bool {
        self.nodes.read().await.contains_key(address)
    }

    /// Create a NetworkClient for a specific node
    pub fn create_client(&self, from_address: String) -> SimulatedNetworkClient {
        SimulatedNetworkClient {
            simulator: self.clone(),
            from_address,
        }
    }
}

/// NetworkClient implementation that uses the simulator for message delivery
#[derive(Clone)]
pub struct SimulatedNetworkClient {
    simulator: NetworkSimulator,
    from_address: String,
}

#[async_trait]
impl NetworkClient for SimulatedNetworkClient {
    async fn call_node(
        &self,
        address: &str,
        message: DhtMessage,
    ) -> Result<DhtMessage, Box<dyn std::error::Error + Send + Sync>> {
        let controller = self.simulator.delivery_controller.read().await;

        // Check if target node is marked as failed
        if controller.failed_nodes.contains(address) {
            return Err("Node is marked as failed".into());
        }

        // Simulate message drop
        if controller.drop_rate > 0.0 && rand::random::<f64>() < controller.drop_rate {
            return Err("Message dropped by simulator".into());
        }

        // Apply simulated latency
        if controller.latency_ms > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(controller.latency_ms)).await;
        }
        drop(controller);

        // Get the target node's message channel
        let nodes = self.simulator.nodes.read().await;
        let sender = nodes
            .get(address)
            .ok_or_else(|| format!("Node {} not registered", address))?
            .clone();
        drop(nodes);

        // Generate unique request ID
        let request_id = self
            .simulator
            .next_request_id
            .fetch_add(1, Ordering::SeqCst);

        // Create response channel for this request
        let (response_tx, mut response_rx) = mpsc::unbounded_channel::<DhtMessage>();

        // Register this request as pending
        {
            let mut pending = self.simulator.pending_requests.write().await;
            pending.insert(request_id, response_tx.clone());
        }

        // Send the request to the target node
        let sim_message = SimulatorMessage::Request {
            from: self.from_address.clone(),
            message: message.clone(),
            request_id,
            response_sender: response_tx,
        };

        sender
            .send(sim_message)
            .map_err(|_| "Failed to send message to node")?;

        // Wait for response with timeout
        let timeout_duration = tokio::time::Duration::from_secs(5);
        match tokio::time::timeout(timeout_duration, response_rx.recv()).await {
            Ok(Some(response)) => {
                // Clean up the pending request
                self.simulator
                    .pending_requests
                    .write()
                    .await
                    .remove(&request_id);
                Ok(response)
            }
            Ok(None) => {
                self.simulator
                    .pending_requests
                    .write()
                    .await
                    .remove(&request_id);
                Err("Response channel closed".into())
            }
            Err(_) => {
                self.simulator
                    .pending_requests
                    .write()
                    .await
                    .remove(&request_id);
                Err("Request timeout".into())
            }
        }
    }
}

impl SimulatedNetworkClient {
    /// Send a response to a pending request (helper method)
    pub async fn send_response(
        &self,
        request_id: RequestId,
        response: DhtMessage,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let pending = self.simulator.pending_requests.read().await;
        if let Some(sender) = pending.get(&request_id) {
            sender
                .send(response)
                .map_err(|_| "Failed to send response")?;
            Ok(())
        } else {
            Err("Request ID not found or already completed".into())
        }
    }
}
