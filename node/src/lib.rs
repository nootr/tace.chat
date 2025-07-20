//! Tace DHT Node Library
//! 
//! This library provides the core functionality for DHT (Distributed Hash Table) nodes
//! implementing the Chord protocol. It can be used both as a standalone binary and
//! as a library for integration testing.

pub mod api;
pub mod network_client;
pub mod node;

// Re-export main types for public API
pub use network_client::{NetworkClient, RealNetworkClient};
pub use node::{ChordNode, NodeInfo};

// Re-export from lib crate for convenience
pub use tace_lib::dht_messages::{DhtMessage, NodeId};
pub use tace_lib::metrics::NetworkMetrics;

/// Configuration for a ChordNode
#[derive(Debug, Clone)]
pub struct Config {
    pub p2p_address: String,
    pub api_address: String,
    pub node_id: Option<NodeId>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            p2p_address: "127.0.0.1:8000".to_string(),
            api_address: "127.0.0.1:9000".to_string(),
            node_id: None,
        }
    }
}

impl<T: NetworkClient> ChordNode<T> {
    /// Create a new ChordNode with a custom network client (useful for testing)
    pub async fn new_with_client(
        node_id: NodeId,
        p2p_address: &str,
        api_address: &str,
        network_client: T,
    ) -> Self {
        // Use the existing constructor with generated or provided node ID
        let mut node = Self::new(
            p2p_address.to_string(),
            api_address.to_string(),
            std::sync::Arc::new(network_client),
        ).await;
        
        // Override the generated ID with the provided one
        node.info.id = node_id;
        node
    }

    /// Create a ChordNode from config with the real network client
    pub async fn from_config(config: Config) -> ChordNode<RealNetworkClient> {
        let node = ChordNode::new(
            config.p2p_address,
            config.api_address,
            std::sync::Arc::new(RealNetworkClient),
        ).await;

        if let Some(node_id) = config.node_id {
            // Override generated ID if provided
            let mut node = node;
            node.info.id = node_id;
            node
        } else {
            node
        }
    }

    /// Join an existing network or create a new one
    pub async fn join_network(&self, bootstrap_address: Option<&str>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.join(bootstrap_address.map(|s| s.to_string())).await;
        Ok(())
    }

    /// Create a new network (this node becomes the first node)
    pub async fn create_network(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.join(None).await;
        Ok(())
    }

    /// Store data in the DHT
    pub async fn store_data(&self, key: NodeId, value: Vec<u8>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.store(key, value).await;
        Ok(())
    }

    /// Retrieve data from the DHT
    pub async fn retrieve_data(&self, key: NodeId) -> Result<Option<Vec<Vec<u8>>>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.retrieve(key).await)
    }

    /// Find successor of a given ID (wrapper with error handling)
    pub async fn find_successor_safe(&self, id: NodeId) -> Result<Option<(NodeId, String, String)>, Box<dyn std::error::Error + Send + Sync>> {
        let successor = self.find_successor(id).await;
        Ok(Some((successor.id, successor.address, successor.api_address)))
    }

    /// Get this node's successor
    pub async fn get_successor(&self) -> Option<(NodeId, String, String)> {
        if let Some(guard) = self.safe_lock_public(&self.successor) {
            let successor = guard.clone();
            Some((successor.id, successor.address, successor.api_address))
        } else {
            None
        }
    }

    /// Get this node's predecessor  
    pub async fn get_predecessor(&self) -> Option<(NodeId, String, String)> {
        if let Some(guard) = self.safe_lock_public(&self.predecessor) {
            guard.as_ref().map(|pred| (pred.id, pred.address.clone(), pred.api_address.clone()))
        } else {
            None
        }
    }

    /// Get network metrics for this node
    pub async fn get_metrics(&self) -> NetworkMetrics {
        if let Some(guard) = self.safe_lock_public(&self.metrics) {
            guard.clone()
        } else {
            NetworkMetrics::default()
        }
    }


    /// Safe lock helper method - expose for testing (calls internal method)
    pub fn safe_lock_public<'a, U>(&self, mutex: &'a std::sync::Mutex<U>) -> Option<std::sync::MutexGuard<'a, U>> {
        match mutex.lock() {
            Ok(guard) => Some(guard),
            Err(_) => None,
        }
    }
}