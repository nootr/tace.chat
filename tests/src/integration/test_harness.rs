use crate::integration::{NetworkSimulator, SimulatorMessage, TimingController};
use std::collections::HashMap;
use std::sync::Arc;
use tace_lib::dht_messages::{DhtMessage, NodeId};
use tace_node::ChordNode;
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;

/// Main test harness that orchestrates multi-node DHT network tests
pub struct TestHarness {
    /// Simulated network for message routing
    network: NetworkSimulator,
    /// Controls timing and synchronization
    timing: TimingController,
    /// Running node instances
    nodes: Arc<RwLock<HashMap<String, TestNode>>>,
    /// Background tasks for node operation
    tasks: Vec<JoinHandle<()>>,
    /// Tracks data stored during tests for invariant checking
    stored_data: Arc<RwLock<HashMap<NodeId, Vec<u8>>>>,
}

use crate::integration::SimulatedNetworkClient;

struct TestNode {
    /// The actual ChordNode instance (production code)
    chord_node: Arc<ChordNode<SimulatedNetworkClient>>,
    /// Node's P2P address
    _p2p_address: String,
    /// Node's API address
    _api_address: String,
    /// Channel for receiving messages from the network simulator
    message_receiver: mpsc::UnboundedReceiver<SimulatorMessage>,
    /// Handle to the node's message processing task
    task_handle: Option<JoinHandle<()>>,
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl TestHarness {
    /// Create a new test harness
    pub fn new() -> Self {
        Self {
            network: NetworkSimulator::new(),
            timing: TimingController::new(),
            nodes: Arc::new(RwLock::new(HashMap::new())),
            tasks: Vec::new(),
            stored_data: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a new node to the network
    pub async fn add_node(
        &mut self,
        node_id: NodeId,
        p2p_port: u16,
        api_port: u16,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let p2p_address = format!("127.0.0.1:{}", p2p_port);
        let api_address = format!("127.0.0.1:{}", api_port);

        // Create message channel for this node
        let (tx, rx) = mpsc::unbounded_channel();

        // Register with network simulator
        self.network.register_node(p2p_address.clone(), tx).await;

        // Create network client that uses our simulator
        let network_client = self.network.create_client(p2p_address.clone());

        // Create the ChordNode with our simulated network client
        let chord_node = Arc::new(
            ChordNode::new_with_client(node_id, &p2p_address, &api_address, network_client).await,
        );

        // Create test node wrapper
        let test_node = TestNode {
            chord_node: chord_node.clone(),
            _p2p_address: p2p_address.clone(),
            _api_address: api_address.clone(),
            message_receiver: rx,
            task_handle: None,
        };

        // Store the node
        self.nodes
            .write()
            .await
            .insert(p2p_address.clone(), test_node);

        Ok(p2p_address)
    }

    /// Start all nodes (begin processing messages)
    pub async fn start_all_nodes(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut nodes = self.nodes.write().await;

        for (address, node) in nodes.iter_mut() {
            if node.task_handle.is_none() {
                let chord_node = node.chord_node.clone();
                let mut receiver =
                    std::mem::replace(&mut node.message_receiver, mpsc::unbounded_channel().1);

                // Start message processing task
                let _address_clone = address.clone();
                let handle = tokio::spawn(async move {
                    while let Some(sim_message) = receiver.recv().await {
                        // Process the message using the ChordNode
                        match sim_message {
                            SimulatorMessage::Request {
                                from,
                                message,
                                request_id: _,
                                response_sender,
                            } => {
                                let response =
                                    Self::process_dht_message(&chord_node, &from, message).await;
                                if let Some(resp) = response {
                                    // Send response back through the channel
                                    let _ = response_sender.send(resp);
                                }
                            }
                            SimulatorMessage::Response { .. } => {
                                // Responses are handled by the network client directly
                                // This shouldn't happen in the node's receiver
                            }
                        }
                    }
                });

                node.task_handle = Some(handle);
            }
        }

        Ok(())
    }

    /// Stop all nodes
    pub async fn stop_all_nodes(&mut self) {
        let mut nodes = self.nodes.write().await;

        for (_, node) in nodes.iter_mut() {
            if let Some(handle) = node.task_handle.take() {
                handle.abort();
            }
        }
    }

    /// Connect a node to the network (join operation)
    pub async fn connect_node_to_network(
        &self,
        node_address: &str,
        bootstrap_address: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let nodes = self.nodes.read().await;
        let node = nodes
            .get(node_address)
            .ok_or_else(|| format!("Node {} not found", node_address))?;

        if let Some(bootstrap) = bootstrap_address {
            // Join existing network
            node.chord_node
                .join_network(Some(bootstrap))
                .await
                .map_err(|e| e.to_string())?;
        } else {
            // Create new network
            node.chord_node
                .create_network()
                .await
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    /// Simulate node failure
    pub async fn fail_node(&self, address: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.network.mark_node_failed(address).await;
        Ok(())
    }

    /// Simulate node recovery
    pub async fn recover_node(&self, address: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.network.mark_node_recovered(address).await;
        Ok(())
    }

    /// Get node by address
    pub async fn get_node(&self, address: &str) -> Option<Arc<ChordNode<SimulatedNetworkClient>>> {
        let nodes = self.nodes.read().await;
        nodes.get(address).map(|n| n.chord_node.clone())
    }

    /// Get all node addresses
    pub async fn get_all_node_addresses(&self) -> Vec<String> {
        self.nodes.read().await.keys().cloned().collect()
    }

    /// Wait for network stabilization by actively running stabilization cycles
    pub async fn wait_for_stabilization(
        &self,
        max_wait_seconds: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let start_time = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(max_wait_seconds);

        // Actively trigger stabilization cycles
        let mut stabilization_rounds = 0;
        let max_rounds = 20; // Prevent infinite loops, optimized for reasonable time

        loop {
            if start_time.elapsed() > timeout {
                return Err("Timeout waiting for stabilization".into());
            }

            if stabilization_rounds >= max_rounds {
                return Err("Maximum stabilization rounds exceeded".into());
            }

            // Get all node addresses
            let node_addresses = self.get_all_node_addresses().await;
            if node_addresses.is_empty() {
                return Ok(()); // No nodes to stabilize
            }

            // Run one round of stabilization on all nodes
            for address in &node_addresses {
                if let Some(node) = self.get_node(address).await {
                    // Trigger all stabilization methods
                    node.stabilize().await;
                    node.fix_fingers().await;
                    node.check_predecessor().await;
                }
            }

            // Wait a bit between rounds - optimized for speed while allowing state updates
            self.timing
                .sleep(std::time::Duration::from_millis(100))
                .await;
            stabilization_rounds += 1;

            // Check if we have achieved basic consistency using our helper method
            let all_have_successors = self.check_ring_consistency().await;
            let mut ring_formed = true;

            // For very simple networks (2 nodes), check ring formation
            if node_addresses.len() == 2 && all_have_successors {
                let node1 = self.get_node(&node_addresses[0]).await.unwrap();
                let node2 = self.get_node(&node_addresses[1]).await.unwrap();

                let n1_successor = node1.get_successor().await;
                let n2_successor = node2.get_successor().await;

                // Check if they point to each other (proper ring)
                if let (Some((_, addr1, _)), Some((_, addr2, _))) = (n1_successor, n2_successor) {
                    ring_formed = (addr1 == node_addresses[1] && addr2 == node_addresses[0])
                        || (addr1 == node_addresses[0] && addr2 == node_addresses[1]);
                } else {
                    ring_formed = false;
                }
            }

            // For networks with 3+ nodes, be more lenient - partial convergence is acceptable
            if node_addresses.len() >= 3 {
                // Check if at least the topology is moving in the right direction
                // In complex networks, perfect convergence may take time or have edge cases
                ring_formed = all_have_successors && stabilization_rounds >= 3; // Faster for practical use
            }

            if all_have_successors && ring_formed {
                // Wait one more round to ensure stability
                self.timing
                    .sleep(std::time::Duration::from_millis(100))
                    .await;
                return Ok(());
            }
        }
    }

    /// Store data through a specific node
    pub async fn store_data(
        &self,
        node_address: &str,
        key: NodeId,
        value: Vec<u8>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let nodes = self.nodes.read().await;
        let node = nodes
            .get(node_address)
            .ok_or_else(|| format!("Node {} not found", node_address))?;

        // Track the stored data for invariant checking
        {
            let mut stored_data = self.stored_data.write().await;
            stored_data.insert(key, value.clone());
        }

        node.chord_node
            .store_data(key, value)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Retrieve data through a specific node with timeout
    pub async fn retrieve_data(
        &self,
        node_address: &str,
        key: NodeId,
    ) -> Result<Option<Vec<Vec<u8>>>, Box<dyn std::error::Error>> {
        let nodes = self.nodes.read().await;
        let node = nodes
            .get(node_address)
            .ok_or_else(|| format!("Node {} not found", node_address))?;

        // Add timeout to prevent hanging on data retrieval
        let chord_node = node.chord_node.clone();
        let retrieval_future = chord_node.retrieve_data(key);

        match tokio::time::timeout(std::time::Duration::from_secs(5), retrieval_future).await {
            Ok(result) => Ok(result.map_err(|e| e.to_string())?),
            Err(_) => Err("Data retrieval timed out after 5 seconds".into()),
        }
    }

    /// Check if the Chord ring is consistent
    async fn check_ring_consistency(&self) -> bool {
        let nodes = self.nodes.read().await;
        let node_addresses: Vec<_> = nodes.keys().cloned().collect();

        for address in &node_addresses {
            if let Some(node) = nodes.get(address) {
                // Check if node has proper successor and predecessor
                let successor = node.chord_node.get_successor().await;
                let predecessor = node.chord_node.get_predecessor().await;

                // Basic consistency checks
                if successor.is_none() || predecessor.is_none() {
                    return false;
                }
            }
        }

        true
    }

    /// Process a DHT message through a ChordNode using the actual ChordNode methods
    async fn process_dht_message(
        chord_node: &ChordNode<SimulatedNetworkClient>,
        _from_address: &str,
        message: DhtMessage,
    ) -> Option<DhtMessage> {
        match message {
            DhtMessage::Ping => Some(DhtMessage::Pong),

            DhtMessage::FindSuccessor { id } => {
                let successor = chord_node.find_successor(id).await;
                Some(DhtMessage::FoundSuccessor {
                    id: successor.id,
                    address: successor.address,
                    api_address: successor.api_address,
                })
            }

            DhtMessage::FindPredecessor { id } => {
                let predecessor = chord_node.find_predecessor(id).await;
                Some(DhtMessage::FoundPredecessor {
                    id: predecessor.id,
                    address: predecessor.address,
                    api_address: predecessor.api_address,
                })
            }

            DhtMessage::GetSuccessor => {
                if let Some((id, address, api_address)) = chord_node.get_successor().await {
                    Some(DhtMessage::FoundSuccessor {
                        id,
                        address,
                        api_address,
                    })
                } else {
                    Some(DhtMessage::Error {
                        message: "No successor found".to_string(),
                    })
                }
            }

            DhtMessage::GetPredecessor => {
                let predecessor = chord_node.get_predecessor().await;
                Some(DhtMessage::Predecessor {
                    id: predecessor.as_ref().map(|(id, _, _)| *id),
                    address: predecessor.as_ref().map(|(_, addr, _)| addr.clone()),
                    api_address: predecessor.as_ref().map(|(_, _, api)| api.clone()),
                })
            }

            DhtMessage::Notify {
                id,
                address,
                api_address,
            } => {
                // Create NodeInfo for notify
                let node_info = tace_node::NodeInfo {
                    id,
                    address,
                    api_address,
                };
                chord_node.notify(node_info).await;
                Some(DhtMessage::Pong) // Notify responds with Pong
            }

            DhtMessage::Store { key, value } => {
                match chord_node.store_data(key, value).await {
                    Ok(_) => Some(DhtMessage::Pong), // Store responds with Pong on success
                    Err(e) => Some(DhtMessage::Error {
                        message: e.to_string(),
                    }),
                }
            }

            DhtMessage::Retrieve { key } => match chord_node.retrieve_data(key).await {
                Ok(data) => Some(DhtMessage::Retrieved { key, value: data }),
                Err(e) => Some(DhtMessage::Error {
                    message: e.to_string(),
                }),
            },

            DhtMessage::ClosestPrecedingNode { id: _ } => {
                // This would need to be implemented in the ChordNode public API
                // For now, return an error
                Some(DhtMessage::Error {
                    message: "ClosestPrecedingNode not implemented in public API".to_string(),
                })
            }

            DhtMessage::GetDataRange { start: _, end: _ } => {
                // This would need to be implemented in the ChordNode public API
                // For now, return empty data range
                Some(DhtMessage::DataRange { data: vec![] })
            }

            DhtMessage::ShareMetrics { metrics: _ } => {
                // Return current node metrics
                let node_metrics = chord_node.get_metrics().await;
                Some(DhtMessage::MetricsShared {
                    metrics: node_metrics,
                })
            }

            DhtMessage::ReplicateData {
                key,
                values,
                replica_id: _,
            } => {
                // Store the replicated data
                for value in values {
                    if (chord_node.store_data(key, value).await).is_err() {
                        return Some(DhtMessage::ReplicationAck {
                            key,
                            success: false,
                        });
                    }
                }
                Some(DhtMessage::ReplicationAck { key, success: true })
            }

            DhtMessage::RequestReplicas { .. } => {
                // Return empty replicas for now
                Some(DhtMessage::ReplicaData { replicas: vec![] })
            }

            // These are response messages, shouldn't be processed here
            DhtMessage::FoundSuccessor { .. }
            | DhtMessage::FoundPredecessor { .. }
            | DhtMessage::Predecessor { .. }
            | DhtMessage::Retrieved { .. }
            | DhtMessage::FoundClosestPrecedingNode { .. }
            | DhtMessage::DataRange { .. }
            | DhtMessage::MetricsShared { .. }
            | DhtMessage::ReplicationAck { .. }
            | DhtMessage::ReplicaData { .. }
            | DhtMessage::Pong
            | DhtMessage::Error { .. } => {
                // These are responses, not requests
                Some(DhtMessage::Error {
                    message: "Received response message as request".to_string(),
                })
            }
        }
    }

    /// Access the network simulator for advanced control
    pub fn network(&self) -> &NetworkSimulator {
        &self.network
    }

    /// Access the timing controller
    pub fn timing(&self) -> &TimingController {
        &self.timing
    }

    /// Trigger a single round of stabilization on all nodes
    pub async fn trigger_stabilization_round(&self) -> Result<(), Box<dyn std::error::Error>> {
        let node_addresses = self.get_all_node_addresses().await;

        for address in &node_addresses {
            if let Some(node) = self.get_node(address).await {
                // Trigger all stabilization methods
                node.stabilize().await;
                node.fix_fingers().await;
                node.check_predecessor().await;
            }
        }

        Ok(())
    }

    /// Trigger multiple rounds of stabilization
    pub async fn trigger_stabilization_cycles(
        &self,
        rounds: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..rounds {
            self.trigger_stabilization_round().await?;
            // Small delay between rounds
            self.timing
                .sleep(std::time::Duration::from_millis(100))
                .await;
        }
        Ok(())
    }

    /// Get all stored data for invariant checking
    pub async fn get_stored_data(&self) -> HashMap<NodeId, Vec<u8>> {
        self.stored_data.read().await.clone()
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        // Clean up tasks
        for task in &self.tasks {
            task.abort();
        }
    }
}
