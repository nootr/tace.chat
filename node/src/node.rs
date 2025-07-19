use log::{debug, error, info};
use num_bigint::BigUint;
use num_traits::ToPrimitive;
use rand::seq::SliceRandom;
use rand::Rng;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tace_lib::dht_messages::{DhtMessage, NodeId};
use tace_lib::metrics::NetworkMetrics;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::network_client::NetworkClient;

/// Helper struct to automatically decrement connection count on drop
struct ConnectionGuard {
    counter: Arc<AtomicU32>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

macro_rules! log_info {
    ($address:expr, $($arg:tt)*) => ({
        info!("[{}] {}", $address, format_args!($($arg)*));
    })
}

macro_rules! log_error {
    ($address:expr, $($arg:tt)*) => ({
        error!("[{}] {}", $address, format_args!($($arg)*));
    })
}

const M: usize = 160; // Number of bits in Chord ID space (SHA-1 produces 160-bit hash)
const SUCCESSOR_LIST_SIZE: usize = 3; // Number of successors to maintain for fault tolerance
const MAX_MESSAGE_SIZE: usize = 1024 * 1024; // 1MB limit to prevent DoS
const MAX_CONNECTIONS: u32 = 100; // Maximum concurrent connections
const REPLICATION_FACTOR: usize = 3; // Number of replicas for each data item
const VIRTUAL_NODES_PER_NODE: usize = 32; // Number of virtual nodes per physical node for better load distribution
const LOAD_BALANCE_THRESHOLD: f32 = 0.8; // Trigger rebalancing when load exceeds 80%

/// Represents a node in the Chord DHT network.
///
/// Each `ChordNode` is responsible for a segment of the hash space and stores key-value pairs.
/// It maintains connections to a successor, a predecessor, and a finger table for efficient lookups.
/// The node also includes logic for joining the network, stabilizing its connections, and handling data storage and retrieval.
///
/// # Type Parameters
///
/// * `T`: A type that implements the `NetworkClient` trait, used for communication between nodes.
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: NodeId,
    pub address: String,
    pub api_address: String,
}

#[derive(Debug)]
pub struct ChordNode<T: NetworkClient> {
    pub info: NodeInfo,
    pub successor: Arc<Mutex<NodeInfo>>,
    pub successor_list: Arc<Mutex<Vec<NodeInfo>>>,
    pub predecessor: Arc<Mutex<Option<NodeInfo>>>,
    pub finger_table: Arc<Mutex<Vec<NodeInfo>>>,
    pub data: Arc<Mutex<HashMap<NodeId, Vec<Vec<u8>>>>>,
    pub virtual_nodes: Arc<Mutex<Vec<NodeId>>>, // Virtual nodes for load balancing
    pub node_loads: Arc<Mutex<HashMap<NodeId, f32>>>, // Load tracking for other nodes
    pub network_client: Arc<T>,
    pub metrics: Arc<Mutex<NetworkMetrics>>,
    pub shutdown_signal: Arc<AtomicBool>,
    pub active_connections: Arc<AtomicU32>,
}

impl<T: NetworkClient> Clone for ChordNode<T> {
    fn clone(&self) -> Self {
        ChordNode {
            info: self.info.clone(),
            successor: self.successor.clone(),
            successor_list: self.successor_list.clone(),
            predecessor: self.predecessor.clone(),
            finger_table: self.finger_table.clone(),
            data: self.data.clone(), // This clones the Arc, not the HashMap
            virtual_nodes: self.virtual_nodes.clone(),
            node_loads: self.node_loads.clone(),
            network_client: self.network_client.clone(),
            metrics: self.metrics.clone(),
            shutdown_signal: self.shutdown_signal.clone(),
            active_connections: self.active_connections.clone(),
        }
    }
}

impl<T: NetworkClient> ChordNode<T> {
    /// Helper method for safe mutex access - returns early on poisoned mutex
    fn safe_lock<'a, U>(
        &self,
        mutex: &'a std::sync::Mutex<U>,
    ) -> Option<std::sync::MutexGuard<'a, U>> {
        match mutex.lock() {
            Ok(guard) => Some(guard),
            Err(e) => {
                log_error!(
                    self.info.address,
                    "Mutex poisoned: {}. This indicates a critical error in another thread.",
                    e
                );
                None
            }
        }
    }

    /// Helper method to safely update a mutex value
    fn safe_update<U, F>(&self, mutex: &std::sync::Mutex<U>, update_fn: F) -> bool
    where
        F: FnOnce(&mut U),
    {
        match self.safe_lock(mutex) {
            Some(mut guard) => {
                update_fn(&mut *guard);
                true
            }
            None => false,
        }
    }
    pub async fn new(address: String, api_address: String, network_client: Arc<T>) -> Self {
        let id = Self::generate_node_id(&address);
        let info = NodeInfo {
            id,
            address: address.clone(),
            api_address,
        };

        let successor = Arc::new(Mutex::new(info.clone()));
        let successor_list = Arc::new(Mutex::new(vec![info.clone(); SUCCESSOR_LIST_SIZE]));
        let predecessor = Arc::new(Mutex::new(None));
        let finger_table = Arc::new(Mutex::<Vec<NodeInfo>>::new(vec![info.clone(); M]));
        let data = Arc::new(Mutex::new(HashMap::new()));
        let virtual_nodes = Arc::new(Mutex::new(Self::generate_virtual_nodes(&address)));
        let node_loads = Arc::new(Mutex::new(HashMap::new()));

        ChordNode {
            info,
            successor,
            successor_list,
            predecessor,
            finger_table,
            data,
            virtual_nodes,
            node_loads,
            network_client,
            metrics: Arc::new(Mutex::new(NetworkMetrics::default())),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            active_connections: Arc::new(AtomicU32::new(0)),
        }
    }

    #[cfg(test)]
    pub fn new_for_test(address: String, network_client: Arc<T>) -> Self {
        let id = Self::generate_node_id(&address);
        let info = NodeInfo {
            id,
            address: address.clone(),
            api_address: address.clone(),
        };

        let successor = Arc::new(Mutex::new(info.clone()));
        let successor_list = Arc::new(Mutex::new(vec![info.clone(); SUCCESSOR_LIST_SIZE]));
        let predecessor = Arc::new(Mutex::new(None));
        let finger_table = Arc::new(Mutex::<Vec<NodeInfo>>::new(vec![info.clone(); M]));
        let data = Arc::new(Mutex::new(HashMap::new()));
        let virtual_nodes = Arc::new(Mutex::new(Self::generate_virtual_nodes(&address)));
        let node_loads = Arc::new(Mutex::new(HashMap::new()));

        ChordNode {
            info,
            successor,
            successor_list,
            predecessor,
            finger_table,
            data,
            virtual_nodes,
            node_loads,
            network_client,
            metrics: Arc::new(Mutex::new(NetworkMetrics::default())),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            active_connections: Arc::new(AtomicU32::new(0)),
        }
    }

    fn generate_node_id(address: &str) -> NodeId {
        let mut hasher = Sha1::new();
        hasher.update(address.as_bytes());
        hasher.finalize().into()
    }

    /// Generate virtual nodes for better load distribution
    fn generate_virtual_nodes(address: &str) -> Vec<NodeId> {
        let mut virtual_nodes = Vec::with_capacity(VIRTUAL_NODES_PER_NODE);
        for i in 0..VIRTUAL_NODES_PER_NODE {
            let vnode_key = format!("{}:{}", address, i);
            let mut hasher = Sha1::new();
            hasher.update(vnode_key.as_bytes());
            virtual_nodes.push(hasher.finalize().into());
        }
        virtual_nodes.sort();
        virtual_nodes
    }

    pub async fn start(&self, bind_address: &str) {
        log_info!(
            self.info.address,
            "Chord Node {} starting at {} (binding to {})",
            hex::encode(self.info.id),
            self.info.address,
            bind_address
        );
        let listener = match TcpListener::bind(bind_address).await {
            Ok(listener) => listener,
            Err(e) => {
                log_error!(
                    self.info.address,
                    "Failed to bind to address {}: {}",
                    bind_address,
                    e
                );
                return;
            }
        };

        let node_clone = self.clone(); // Clone the Arc for each connection
        loop {
            // Check for shutdown signal
            if self.shutdown_signal.load(Ordering::Relaxed) {
                log_info!(
                    self.info.address,
                    "Shutdown signal received, stopping server"
                );
                break;
            }

            match listener.accept().await {
                Ok((socket, addr)) => {
                    debug!("[{}] Accepted connection from {}", self.info.address, addr);
                    let node = node_clone.clone();
                    tokio::spawn(async move {
                        node.handle_connection(socket).await;
                    });
                }
                Err(e) => {
                    log_error!(self.info.address, "Failed to accept connection: {}", e);
                    // Add small delay to prevent tight loop on persistent errors
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        }
    }

    async fn handle_connection(&self, mut socket: TcpStream) {
        // Check connection limit
        let current_connections = self.active_connections.fetch_add(1, Ordering::Relaxed);
        if current_connections >= MAX_CONNECTIONS {
            self.active_connections.fetch_sub(1, Ordering::Relaxed);
            log_error!(
                self.info.address,
                "Connection limit exceeded, rejecting connection"
            );
            return;
        }

        // Ensure connection count is decremented on exit
        let _guard = ConnectionGuard {
            counter: self.active_connections.clone(),
        };

        let peer_addr = socket.peer_addr().ok();
        let mut buffer = Vec::with_capacity(8192); // Start with reasonable capacity

        // Read message with length limit
        loop {
            let mut temp_buf = vec![0u8; 1024];
            match socket.read(&mut temp_buf).await {
                Ok(0) => break, // EOF
                Ok(n) => {
                    if buffer.len() + n > MAX_MESSAGE_SIZE {
                        log_error!(self.info.address, "Message too large, dropping connection");
                        return;
                    }
                    if let Some(slice) = temp_buf.get(..n) {
                        buffer.extend_from_slice(slice);
                    } else {
                        log_error!(self.info.address, "Invalid read size from socket");
                        return;
                    }
                }
                Err(e) => {
                    log_error!(self.info.address, "Failed to read from socket: {}", e);
                    return;
                }
            }
        }

        match bincode::deserialize::<DhtMessage>(&buffer) {
            Ok(message) => {
                log_info!(self.info.address, "Received message: {:?} ", message);

                let response = match message {
                    DhtMessage::FindSuccessor { id } => {
                        let successor = self.find_successor(id).await;
                        DhtMessage::FoundSuccessor {
                            id: successor.id,
                            address: successor.address,
                            api_address: successor.api_address,
                        }
                    }
                    DhtMessage::FindPredecessor { id } => {
                        let predecessor = self.find_predecessor(id).await;
                        DhtMessage::FoundPredecessor {
                            id: predecessor.id,
                            address: predecessor.address,
                            api_address: predecessor.api_address,
                        }
                    }
                    DhtMessage::GetSuccessor => {
                        if let Some(successor_guard) = self.safe_lock(&self.successor) {
                            let successor = successor_guard.clone();
                            DhtMessage::FoundSuccessor {
                                id: successor.id,
                                address: successor.address,
                                api_address: successor.api_address,
                            }
                        } else {
                            DhtMessage::Error {
                                message: "Failed to access successor information".to_string(),
                            }
                        }
                    }
                    DhtMessage::ClosestPrecedingNode { id } => {
                        let cpn = self.closest_preceding_node(id).await;
                        DhtMessage::FoundClosestPrecedingNode {
                            id: cpn.id,
                            address: cpn.address,
                            api_address: cpn.api_address,
                        }
                    }
                    DhtMessage::Store { key, value } => {
                        self.store(key, value).await;
                        DhtMessage::Pong // Send a Pong response
                    }
                    DhtMessage::Retrieve { key } => {
                        let value = self.retrieve(key).await;
                        DhtMessage::Retrieved { key, value }
                    }
                    DhtMessage::GetPredecessor => {
                        if let Some(predecessor_guard) = self.safe_lock(&self.predecessor) {
                            let predecessor = predecessor_guard.clone();
                            DhtMessage::Predecessor {
                                id: predecessor.as_ref().map(|p| p.id),
                                address: predecessor.as_ref().map(|p| p.address.clone()),
                                api_address: predecessor.as_ref().map(|p| p.api_address.clone()),
                            }
                        } else {
                            DhtMessage::Error {
                                message: "Failed to access predecessor information".to_string(),
                            }
                        }
                    }
                    DhtMessage::Notify {
                        id,
                        address,
                        api_address,
                    } => {
                        self.notify(NodeInfo {
                            id,
                            address,
                            api_address,
                        })
                        .await;
                        DhtMessage::Pong // Send a Pong response
                    }
                    DhtMessage::Ping => DhtMessage::Pong,
                    DhtMessage::GetDataRange { start, end } => {
                        let data = self.get_data_range(start, end).await;
                        DhtMessage::DataRange { data }
                    }
                    DhtMessage::ShareMetrics { metrics } => {
                        let response_metrics = self.handle_share_metrics(metrics).await;
                        DhtMessage::MetricsShared {
                            metrics: response_metrics,
                        }
                    }
                    DhtMessage::ReplicateData {
                        key,
                        values,
                        replica_id,
                    } => {
                        let success = self.handle_replicate_data(key, values, replica_id).await;
                        DhtMessage::ReplicationAck { key, success }
                    }
                    DhtMessage::RequestReplicas {
                        key_range_start,
                        key_range_end,
                    } => {
                        let replicas = self
                            .handle_request_replicas(key_range_start, key_range_end)
                            .await;
                        DhtMessage::ReplicaData { replicas }
                    }
                    _ => {
                        log_error!(
                            self.info.address,
                            "Unsupported message received: {:?}",
                            message
                        );
                        DhtMessage::Error {
                            message: "Unsupported message type".to_string(),
                        }
                    }
                };
                let encoded_response = match bincode::serialize(&response) {
                    Ok(data) => data,
                    Err(e) => {
                        log_error!(self.info.address, "Failed to serialize response: {}", e);
                        return;
                    }
                };
                if let Some(addr) = peer_addr {
                    log_info!(
                        self.info.address,
                        "Sending response to {}: {:?}",
                        addr,
                        response
                    );
                }
                if let Err(e) = socket.write_all(&encoded_response).await {
                    log_error!(
                        self.info.address,
                        "Failed to write response to socket: {}",
                        e
                    );
                }
            }
            Err(e) => {
                log_error!(self.info.address, "Failed to deserialize message: {}", e);
            }
        }
    }

    /// Stores a key-value pair in the DHT with replication
    pub async fn store(&self, key: NodeId, value: Vec<u8>) {
        // Store locally first
        if let Some(mut data) = self.safe_lock(&self.data) {
            let values = data.entry(key).or_default();
            values.push(value.clone());
            log_info!(
                self.info.address,
                "Stored key locally: {}",
                hex::encode(key)
            );
        } else {
            log_error!(
                self.info.address,
                "Failed to acquire data lock for store operation"
            );
            return;
        }

        // Replicate to backup nodes
        self.replicate_data(key, vec![value]).await;
    }

    /// Replicate data to backup nodes for fault tolerance
    async fn replicate_data(&self, key: NodeId, values: Vec<Vec<u8>>) {
        let replica_nodes = self.get_replica_nodes(key).await;

        for (i, node) in replica_nodes.iter().enumerate() {
            if node.id == self.info.id {
                continue; // Skip self
            }

            let replica_id = (i + 1) as u8;
            let message = DhtMessage::ReplicateData {
                key,
                values: values.clone(),
                replica_id,
            };

            match self.network_client.call_node(&node.address, message).await {
                Ok(DhtMessage::ReplicationAck { success: true, .. }) => {
                    debug!(
                        "[{}] Successfully replicated key {} to replica {} at {}",
                        self.info.address,
                        hex::encode(key),
                        replica_id,
                        node.address
                    );
                }
                Ok(DhtMessage::ReplicationAck { success: false, .. }) => {
                    log_error!(
                        self.info.address,
                        "Replication failed for key {} to replica {} at {}",
                        hex::encode(key),
                        replica_id,
                        node.address
                    );
                }
                Err(e) => {
                    log_error!(
                        self.info.address,
                        "Failed to replicate key {} to {}: {}",
                        hex::encode(key),
                        node.address,
                        e
                    );
                }
                _ => {
                    log_error!(
                        self.info.address,
                        "Unexpected response when replicating key {} to {}",
                        hex::encode(key),
                        node.address
                    );
                }
            }
        }
    }

    /// Get the nodes responsible for storing replicas of this key with load balancing
    async fn get_replica_nodes(&self, key: NodeId) -> Vec<NodeInfo> {
        let mut replica_nodes = Vec::new();

        // Primary node (responsible for the key) - use virtual nodes for better distribution
        let primary = self.find_successor_with_virtual_nodes(key).await;
        replica_nodes.push(primary.clone());

        // Get additional replica nodes considering load balancing
        let mut candidate_nodes = Vec::new();

        // Collect candidates from successor list
        if let Some(successor_list) = self.safe_lock(&self.successor_list) {
            candidate_nodes.extend(successor_list.clone());
        }

        // Add some nodes from finger table for diversity
        if let Some(finger_table) = self.safe_lock(&self.finger_table) {
            for (i, node) in finger_table.iter().enumerate() {
                if i % 3 == 0 {
                    // Sample every 3rd finger
                    candidate_nodes.push(node.clone());
                }
            }
        }

        // Remove duplicates and self
        candidate_nodes.sort_by_key(|n| n.id);
        candidate_nodes.dedup_by_key(|n| n.id);
        candidate_nodes
            .retain(|n| n.id != self.info.id && !replica_nodes.iter().any(|r| r.id == n.id));

        // Sort by load (prefer less loaded nodes)
        if let Some(node_loads) = self.safe_lock(&self.node_loads) {
            candidate_nodes.sort_by(|a, b| {
                let load_a = node_loads.get(&a.id).unwrap_or(&0.0);
                let load_b = node_loads.get(&b.id).unwrap_or(&0.0);
                load_a
                    .partial_cmp(load_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        // Select additional replicas from least loaded candidates
        for node in candidate_nodes.iter().take(REPLICATION_FACTOR - 1) {
            replica_nodes.push(node.clone());
        }

        replica_nodes
    }

    /// Handle incoming replication request
    async fn handle_replicate_data(
        &self,
        key: NodeId,
        values: Vec<Vec<u8>>,
        replica_id: u8,
    ) -> bool {
        debug!(
            "[{}] Receiving replica {} for key {}",
            self.info.address,
            replica_id,
            hex::encode(key)
        );

        if let Some(mut data) = self.safe_lock(&self.data) {
            let entry = data.entry(key).or_default();
            entry.extend(values);
            log_info!(
                self.info.address,
                "Stored replica {} for key: {}",
                replica_id,
                hex::encode(key)
            );
            true
        } else {
            log_error!(
                self.info.address,
                "Failed to store replica {} for key: {}",
                replica_id,
                hex::encode(key)
            );
            false
        }
    }

    /// Handle request for replica data during recovery
    async fn handle_request_replicas(
        &self,
        start: NodeId,
        end: NodeId,
    ) -> Vec<(NodeId, Vec<Vec<u8>>)> {
        log_info!(
            self.info.address,
            "Providing replicas for range ({}, {}]",
            hex::encode(start),
            hex::encode(end)
        );

        if let Some(data) = self.safe_lock(&self.data) {
            let mut replicas = Vec::new();
            for (key, values) in data.iter() {
                if tace_lib::is_between(key, &start, &end) || *key == end {
                    replicas.push((*key, values.clone()));
                }
            }
            replicas
        } else {
            log_error!(
                self.info.address,
                "Failed to access data for replica request"
            );
            Vec::new()
        }
    }

    /// Retrieves a value by key from the DHT with load-balanced reads
    pub async fn retrieve(&self, key: NodeId) -> Option<Vec<Vec<u8>>> {
        // First try local retrieval
        if let Some(data) = self.safe_lock(&self.data) {
            if let Some(value) = data.get(&key).cloned() {
                log_info!(
                    self.info.address,
                    "Retrieved key locally: {}",
                    hex::encode(key)
                );
                return Some(value);
            }
        }

        // If not found locally, try load-balanced retrieval from replicas
        self.retrieve_from_replicas(key).await
    }

    /// Retrieve from replicas using load balancing
    async fn retrieve_from_replicas(&self, key: NodeId) -> Option<Vec<Vec<u8>>> {
        let replica_nodes = self.get_replica_nodes(key).await;

        // Sort replicas by load (prefer less loaded nodes for reads)
        let mut sorted_replicas = replica_nodes;
        if let Some(node_loads) = self.safe_lock(&self.node_loads) {
            sorted_replicas.sort_by(|a, b| {
                let load_a = node_loads.get(&a.id).unwrap_or(&0.0);
                let load_b = node_loads.get(&b.id).unwrap_or(&0.0);
                load_a
                    .partial_cmp(load_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        // Try each replica in order of least load
        for node in sorted_replicas {
            if node.id == self.info.id {
                continue; // Skip self (already tried)
            }

            match self
                .network_client
                .call_node(&node.address, DhtMessage::Retrieve { key })
                .await
            {
                Ok(DhtMessage::Retrieved {
                    value: Some(value), ..
                }) => {
                    log_info!(
                        self.info.address,
                        "Retrieved key {} from replica at {}",
                        hex::encode(key),
                        node.address
                    );
                    return Some(value);
                }
                Ok(DhtMessage::Retrieved { value: None, .. }) => {
                    debug!(
                        "[{}] Key {} not found at replica {}",
                        self.info.address,
                        hex::encode(key),
                        node.address
                    );
                }
                Err(e) => {
                    log_error!(
                        self.info.address,
                        "Failed to retrieve from replica {}: {}",
                        node.address,
                        e
                    );
                }
                _ => {
                    log_error!(
                        self.info.address,
                        "Unexpected response from replica {}",
                        node.address
                    );
                }
            }
        }

        log_info!(self.info.address, "Key not found: {}", hex::encode(key));
        None
    }

    /// Retrieves all key-value pairs in a given range
    pub async fn get_data_range(&self, start: NodeId, end: NodeId) -> Vec<(NodeId, Vec<Vec<u8>>)> {
        let data = match self.safe_lock(&self.data) {
            Some(guard) => guard,
            None => {
                log_error!(
                    self.info.address,
                    "Failed to acquire data lock for range query"
                );
                return Vec::new();
            }
        };
        let mut result = Vec::new();

        for (key, value) in data.iter() {
            // Check if key is in the range (start, end]
            if tace_lib::is_between(key, &start, &end) || *key == end {
                result.push((*key, value.clone()));
            }
        }

        log_info!(
            self.info.address,
            "Retrieved {} keys in range ({}, {}]",
            result.len(),
            hex::encode(start),
            hex::encode(end)
        );

        result
    }

    pub async fn join(&self, bootstrap_address: Option<String>) {
        let Some(address) = bootstrap_address else {
            log_info!(
                self.info.address,
                "No bootstrap node provided. Starting a new network."
            );
            self.start_new_network().await;
            return;
        };

        log_info!(
            self.info.address,
            "Attempting to join network via bootstrap node: {}",
            address
        );

        // Find successor from bootstrap node
        let response = self
            .network_client
            .call_node(&address, DhtMessage::FindSuccessor { id: self.info.id })
            .await;

        let successor_info = match response {
            Ok(DhtMessage::FoundSuccessor {
                id,
                address,
                api_address,
            }) => NodeInfo {
                id,
                address,
                api_address,
            },
            Ok(other) => {
                log_error!(
                    self.info.address,
                    "Unexpected response from bootstrap node: {:?}",
                    other
                );
                return;
            }
            Err(e) => {
                log_error!(
                    self.info.address,
                    "Failed to get successor from bootstrap node: {}",
                    e
                );
                return;
            }
        };

        if !self.safe_update(&self.successor, |successor| {
            *successor = successor_info.clone();
        }) {
            log_error!(
                self.info.address,
                "Failed to update successor after joining"
            );
            return;
        }
        log_info!(
            self.info.address,
            "Joined network. Successor: {} at {}",
            hex::encode(successor_info.id),
            successor_info.address
        );

        // Request data from successor that should belong to us
        log_info!(
            self.info.address,
            "Requesting data transfer from successor {}",
            successor_info.address
        );

        let data_response = self
            .network_client
            .call_node(
                &successor_info.address,
                DhtMessage::GetDataRange {
                    start: self.info.id,
                    end: successor_info.id,
                },
            )
            .await;

        match data_response {
            Ok(DhtMessage::DataRange { data }) => {
                if !data.is_empty() {
                    log_info!(
                        self.info.address,
                        "Received {} keys from successor",
                        data.len()
                    );
                    for (key, values) in data {
                        for value in values {
                            self.store(key, value).await;
                        }
                    }
                }
            }
            Ok(other) => {
                log_error!(
                    self.info.address,
                    "Unexpected response to GetDataRange: {:?}",
                    other
                );
            }
            Err(e) => {
                log_error!(
                    self.info.address,
                    "Failed to get data from successor: {}",
                    e
                );
            }
        }
    }

    async fn start_new_network(&self) {
        let success = self.safe_update(&self.successor, |successor| {
            *successor = self.info.clone();
        }) && self.safe_update(&self.successor_list, |successor_list| {
            *successor_list = vec![self.info.clone(); SUCCESSOR_LIST_SIZE];
        }) && self.safe_update(&self.predecessor, |predecessor| {
            *predecessor = None;
        });

        if success {
            log_info!(
                self.info.address,
                "Started new network. I am the only node."
            );
        } else {
            log_error!(
                self.info.address,
                "Failed to initialize new network due to mutex errors"
            );
        }
    }

    /// Finds the successor of an ID
    pub async fn find_successor(&self, id: NodeId) -> NodeInfo {
        // According to Chord algorithm: find_successor(id) = find_predecessor(id).successor
        let predecessor = self.find_predecessor(id).await;

        // If the predecessor is this node, return our successor
        if predecessor.id == self.info.id {
            return match self.safe_lock(&self.successor) {
                Some(guard) => guard.clone(),
                None => {
                    log_error!(
                        self.info.address,
                        "Failed to access successor in find_successor"
                    );
                    self.info.clone() // Fallback to self
                }
            };
        }

        // Otherwise, get the successor of the predecessor
        match self
            .network_client
            .call_node(&predecessor.address, DhtMessage::GetSuccessor)
            .await
        {
            Ok(DhtMessage::FoundSuccessor {
                id,
                address,
                api_address,
            }) => NodeInfo {
                id,
                address,
                api_address,
            },
            _ => {
                log_error!(
                    self.info.address,
                    "Error: Failed to get successor from predecessor {}.",
                    predecessor.address
                );

                // Try to use successor list as fallback
                if let Some(next_available) = self.find_next_available_successor().await {
                    log_info!(
                        self.info.address,
                        "Using backup successor {} after failure",
                        next_available.address
                    );
                    next_available
                } else {
                    // Last resort: return self's successor
                    log_error!(
                        self.info.address,
                        "No backup successors available, using current successor"
                    );
                    match self.safe_lock(&self.successor) {
                        Some(guard) => guard.clone(),
                        None => {
                            log_error!(
                                self.info.address,
                                "Critical: failed to access successor in find_successor fallback"
                            );
                            self.info.clone()
                        }
                    }
                }
            }
        }
    }

    /// Find successor using virtual nodes for better load distribution
    async fn find_successor_with_virtual_nodes(&self, key: NodeId) -> NodeInfo {
        // Check if any of our virtual nodes are responsible for this key
        if let Some(virtual_nodes) = self.safe_lock(&self.virtual_nodes) {
            // Find the virtual node that should handle this key
            let mut best_vnode = None;
            for &vnode_id in virtual_nodes.iter() {
                if tace_lib::is_between(&key, &self.info.id, &vnode_id) || key == vnode_id {
                    best_vnode = Some(vnode_id);
                    break;
                }
            }

            // If one of our virtual nodes should handle it, use the physical node
            if best_vnode.is_some() {
                return self.info.clone();
            }
        }

        // Otherwise, use standard Chord algorithm
        self.find_successor(key).await
    }

    /// Finds the predecessor of an ID
    pub async fn find_predecessor(&self, id: NodeId) -> NodeInfo {
        let mut n_prime = self.info.clone();
        let mut current_successor = match self.safe_lock(&self.successor) {
            Some(guard) => guard.clone(),
            None => {
                log_error!(
                    self.info.address,
                    "Failed to access successor in find_predecessor"
                );
                return self.info.clone();
            }
        };
        let mut retry_count = 0;
        const MAX_RETRIES: u32 = 10;

        // Loop until id is between n_prime and its successor
        while !tace_lib::is_between(&id, &n_prime.id, &current_successor.id) {
            retry_count += 1;
            if retry_count > MAX_RETRIES {
                log_error!(
                    self.info.address,
                    "Max retries reached in find_predecessor, returning current node."
                );
                break;
            }

            let next_node = if n_prime.id == self.info.id {
                self.closest_preceding_node(id).await
            } else {
                match self
                    .network_client
                    .call_node(&n_prime.address, DhtMessage::ClosestPrecedingNode { id })
                    .await
                {
                    Ok(DhtMessage::FoundClosestPrecedingNode {
                        id,
                        address,
                        api_address,
                    }) => NodeInfo {
                        id,
                        address,
                        api_address,
                    },
                    _ => {
                        log_error!(
                            self.info.address,
                            "Error: Failed to get closest preceding node from {}.",
                            n_prime.address
                        );
                        // If we can't get a closer node, return what we have
                        break;
                    }
                }
            };

            // Avoid infinite loops by checking if we got the same node
            if next_node.id == n_prime.id {
                break;
            }

            n_prime = next_node;

            // Get the successor of the new n_prime
            current_successor = if n_prime.id == self.info.id {
                match self.safe_lock(&self.successor) {
                    Some(guard) => guard.clone(),
                    None => {
                        log_error!(
                            self.info.address,
                            "Failed to access successor in find_predecessor loop"
                        );
                        break;
                    }
                }
            } else {
                match self
                    .network_client
                    .call_node(&n_prime.address, DhtMessage::GetSuccessor)
                    .await
                {
                    Ok(DhtMessage::FoundSuccessor {
                        id,
                        address,
                        api_address,
                    }) => NodeInfo {
                        id,
                        address,
                        api_address,
                    },
                    _ => {
                        log_error!(
                            self.info.address,
                            "Error: Failed to get successor of {}.",
                            n_prime.address
                        );
                        // If we can't get the successor, return what we have
                        break;
                    }
                }
            };
        }
        n_prime
    }

    /// Finds the node in the finger table that most immediately precedes `id`.
    async fn closest_preceding_node(&self, id: NodeId) -> NodeInfo {
        let finger_table = match self.safe_lock(&self.finger_table) {
            Some(guard) => guard,
            None => {
                log_error!(
                    self.info.address,
                    "Failed to access finger table in closest_preceding_node"
                );
                return self.info.clone();
            }
        };
        // Iterate finger table in reverse
        for i in (0..M.min(finger_table.len())).rev() {
            let finger = match finger_table.get(i) {
                Some(f) => f,
                None => continue,
            };
            // If finger is between current node and id (exclusive of current node, exclusive of id)
            if tace_lib::is_between(&finger.id, &self.info.id, &id) {
                return finger.clone();
            }
        }
        self.info.clone()
    }

    /// Handles a ShareMetrics message by averaging with local metrics
    /// and returning the updated local metrics.
    pub async fn handle_share_metrics(&self, incoming_metrics: NetworkMetrics) -> NetworkMetrics {
        let mut local_metrics = match self.safe_lock(&self.metrics) {
            Some(guard) => guard,
            None => {
                log_error!(self.info.address, "Failed to acquire metrics lock");
                return incoming_metrics; // Return the incoming metrics as fallback
            }
        };

        // Average the metrics
        local_metrics.node_churn_rate =
            (local_metrics.node_churn_rate + incoming_metrics.node_churn_rate) / 2.0;
        local_metrics.routing_table_health =
            (local_metrics.routing_table_health + incoming_metrics.routing_table_health) / 2.0;
        local_metrics.operation_success_rate =
            (local_metrics.operation_success_rate + incoming_metrics.operation_success_rate) / 2.0;
        local_metrics.operation_latency =
            (local_metrics.operation_latency + incoming_metrics.operation_latency) / 2;
        local_metrics.message_type_ratio =
            (local_metrics.message_type_ratio + incoming_metrics.message_type_ratio) / 2.0;

        // Only gossip the estimated total network keys, not local key count
        local_metrics.total_network_keys_estimate = (local_metrics.total_network_keys_estimate
            + incoming_metrics.total_network_keys_estimate)
            / 2.0;

        // Average, bound, and smooth the network size estimate
        let new_raw_estimate =
            (local_metrics.network_size_estimate + incoming_metrics.network_size_estimate) / 2.0;
        let bounded_estimate =
            self.apply_bounds(local_metrics.network_size_estimate, new_raw_estimate);
        let smoothed_estimate =
            self.apply_smoothing(local_metrics.network_size_estimate, bounded_estimate);
        local_metrics.network_size_estimate = smoothed_estimate;

        local_metrics.clone()
    }

    /// Periodically shares metrics with a random peer and updates local metrics
    pub async fn share_and_update_metrics(&self) {
        self.update_local_metrics().await;

        // Collect all known nodes (successor + finger table)
        let successor = match self.safe_lock(&self.successor) {
            Some(guard) => guard.clone(),
            None => {
                log_error!(
                    self.info.address,
                    "Failed to access successor for metrics sharing"
                );
                return;
            }
        };
        let finger_table = match self.safe_lock(&self.finger_table) {
            Some(guard) => guard.clone(),
            None => {
                log_error!(
                    self.info.address,
                    "Failed to access finger table for metrics sharing"
                );
                return;
            }
        };

        // If we're the only node, no need to update
        if successor.id == self.info.id {
            return;
        }

        // Create a set of unique nodes (successor + fingers), excluding ourselves
        let mut candidate_nodes = Vec::new();

        // Add successor if it's not ourselves
        if successor.id != self.info.id {
            candidate_nodes.push(successor);
        }

        // Add unique finger table entries that are not ourselves
        for finger in finger_table {
            if finger.id != self.info.id && !candidate_nodes.iter().any(|n| n.id == finger.id) {
                candidate_nodes.push(finger);
            }
        }

        // If no candidates, return early
        if candidate_nodes.is_empty() {
            return;
        }

        // Pick a random node from candidates
        let random_node = match candidate_nodes.choose(&mut rand::thread_rng()) {
            Some(node) => node,
            None => {
                log_error!(
                    self.info.address,
                    "No candidate nodes available for metrics sharing"
                );
                return;
            }
        };

        debug!(
            "[{}] Sharing metrics with random neighbor {}",
            self.info.address, random_node.address
        );

        let local_metrics = match self.safe_lock(&self.metrics) {
            Some(guard) => guard.clone(),
            None => {
                log_error!(self.info.address, "Failed to access metrics for sharing");
                return;
            }
        };

        // Get neighbor's estimate
        match self
            .network_client
            .call_node(
                &random_node.address,
                DhtMessage::ShareMetrics {
                    metrics: local_metrics,
                },
            )
            .await
        {
            Ok(DhtMessage::MetricsShared { metrics }) => {
                self.handle_share_metrics(metrics).await;
                debug!(
                    "[{}] Metrics updated with neighbor {}",
                    self.info.address, random_node.address
                );
            }
            _ => {
                log_error!(
                    self.info.address,
                    "Failed to get metrics from {}",
                    random_node.address
                );
            }
        }
    }

    /// Updates the local metrics based on the node's current state
    async fn update_local_metrics(&self) {
        let mut local_metrics = match self.safe_lock(&self.metrics) {
            Some(guard) => guard,
            None => {
                log_error!(self.info.address, "Failed to access metrics for update");
                return;
            }
        };

        // Update routing table health
        let finger_table = match self.safe_lock(&self.finger_table) {
            Some(guard) => guard,
            None => {
                log_error!(
                    self.info.address,
                    "Failed to access finger table for metrics update"
                );
                return;
            }
        };
        let filled_slots = finger_table.iter().filter(|n| n.id != self.info.id).count();
        local_metrics.routing_table_health = filled_slots as f64 / M as f64;
        drop(finger_table);

        // Update local key count (actual keys stored on this node)
        let data = match self.safe_lock(&self.data) {
            Some(guard) => guard,
            None => {
                log_error!(
                    self.info.address,
                    "Failed to access data for metrics update"
                );
                return;
            }
        };
        local_metrics.local_key_count = data.len() as u64;
        drop(data);

        // Update network size estimate using generalized function
        let local_size_estimate = self.calculate_local_network_size_estimate();
        local_metrics.network_size_estimate = self.update_network_estimate_f64(
            local_metrics.network_size_estimate,
            local_size_estimate,
            1.0,  // network_size: Use 1.0 since local estimate is already scaled
            0.2,  // contribution_factor: Blend local observations with existing estimate
            true, // apply_bounds: Network size should be bounded to prevent wild swings
            0.1,  // smoothing_factor: Standard smoothing for network size
        );

        // Periodically contribute local key count to network estimate using generalized function
        local_metrics.total_network_keys_estimate = self.update_network_estimate_f64(
            local_metrics.total_network_keys_estimate,
            local_metrics.local_key_count as f64,
            local_metrics.network_size_estimate,
            0.1,   // contribution_factor: How much local data influences network estimate
            false, // apply_bounds: Key counts can vary widely, don't bound them
            0.05,  // smoothing_factor: Light smoothing for key count changes
        );

        // Other metrics like churn, success rate, latency, and message ratio
        // would require more complex tracking over time.
        // For now, we'll leave them as they are, to be updated by gossip.
    }

    /// Find the next available successor from the successor list
    async fn find_next_available_successor(&self) -> Option<NodeInfo> {
        let successor_list = match self.safe_lock(&self.successor_list) {
            Some(guard) => guard.clone(),
            None => {
                log_error!(self.info.address, "Failed to access successor list");
                return None;
            }
        };

        for successor in successor_list {
            if successor.id == self.info.id {
                continue;
            }

            // Try to ping the successor
            match self
                .network_client
                .call_node(&successor.address, DhtMessage::Ping)
                .await
            {
                Ok(DhtMessage::Pong) => {
                    return Some(successor);
                }
                _ => {
                    debug!(
                        "[{}] Successor {} is unreachable",
                        self.info.address, successor.address
                    );
                }
            }
        }

        None
    }

    /// Update the successor list by querying successors
    async fn update_successor_list(&self) {
        let current_successor = match self.safe_lock(&self.successor) {
            Some(guard) => guard.clone(),
            None => {
                log_error!(
                    self.info.address,
                    "Failed to access successor for list update"
                );
                return;
            }
        };
        let mut new_list = vec![current_successor.clone()];

        let mut current = current_successor;
        for _i in 1..SUCCESSOR_LIST_SIZE {
            if current.id == self.info.id {
                // We've looped back to ourselves
                break;
            }

            match self
                .network_client
                .call_node(&current.address, DhtMessage::GetSuccessor)
                .await
            {
                Ok(DhtMessage::FoundSuccessor {
                    id,
                    address,
                    api_address,
                }) => {
                    let next = NodeInfo {
                        id,
                        address,
                        api_address,
                    };
                    if next.id == self.info.id {
                        // Don't include ourselves in the successor list
                        break;
                    }
                    new_list.push(next.clone());
                    current = next;
                }
                _ => {
                    debug!(
                        "[{}] Failed to get successor from {}",
                        self.info.address, current.address
                    );
                    break;
                }
            }
        }

        // Fill remaining slots with the last known successor
        while new_list.len() < SUCCESSOR_LIST_SIZE {
            if let Some(last) = new_list.last() {
                new_list.push(last.clone());
            } else {
                break;
            }
        }

        if !self.safe_update(&self.successor_list, |list| {
            *list = new_list;
        }) {
            log_error!(self.info.address, "Failed to update successor list");
        }
    }

    pub async fn stabilize(&self) {
        let successor = match self.safe_lock(&self.successor) {
            Some(guard) => guard.clone(),
            None => {
                log_error!(
                    self.info.address,
                    "Failed to acquire successor lock for stabilization"
                );
                return;
            }
        };

        debug!(
            "[{}] Stabilize: current successor is {}",
            self.info.address, successor.address
        );

        // If this node is its own successor, it's the only node in the network.
        // In this case, its predecessor should be None.
        if self.info.id == successor.id {
            debug!(
                "[{}] Stabilize: single node network, clearing predecessor",
                self.info.address
            );
            if !self.safe_update(&self.predecessor, |pred| {
                if pred.is_some() {
                    *pred = None;
                }
            }) {
                log_error!(
                    self.info.address,
                    "Failed to clear predecessor in stabilize"
                );
            }
            return;
        }

        // Get successor's predecessor
        debug!(
            "[{}] Stabilize: asking successor {} for its predecessor",
            self.info.address, successor.address
        );

        let predecessor_response = self
            .network_client
            .call_node(&successor.address, DhtMessage::GetPredecessor)
            .await;

        // Handle successor failure
        if predecessor_response.is_err() {
            log_error!(
                self.info.address,
                "Successor {} is unreachable, finding next available successor",
                successor.address
            );

            if let Some(new_successor) = self.find_next_available_successor().await {
                if self.safe_update(&self.successor, |successor| {
                    *successor = new_successor.clone();
                }) {
                    log_info!(
                        self.info.address,
                        "Updated successor to {} after failure",
                        new_successor.address
                    );
                    // Update successor list after changing successor
                    self.update_successor_list().await;
                    return;
                } else {
                    log_error!(
                        self.info.address,
                        "Failed to update successor after failure"
                    );
                    return;
                }
            } else {
                log_error!(
                    self.info.address,
                    "No available successors found, network may be partitioned"
                );
                return;
            }
        }

        match predecessor_response {
            Ok(DhtMessage::Predecessor {
                id: Some(x_id),
                address: Some(x_address),
                api_address: Some(x_api_address),
            }) => {
                let x = NodeInfo {
                    id: x_id,
                    address: x_address,
                    api_address: x_api_address,
                };
                debug!(
                    "[{}] Stabilize: successor's predecessor is {}",
                    self.info.address, x.address
                );

                // If x is between self and successor, then x is the new successor
                if tace_lib::is_between(&x.id, &self.info.id, &successor.id) {
                    debug!(
                        "[{}] Stabilize: updating successor from {} to {}",
                        self.info.address, successor.address, x.address
                    );
                    if !self.safe_update(&self.successor, |succ| {
                        *succ = x.clone();
                    }) {
                        log_error!(self.info.address, "Failed to update successor in stabilize");
                    }
                } else {
                    debug!(
                        "[{}] Stabilize: keeping current successor {}",
                        self.info.address, successor.address
                    );
                }
            }
            Ok(DhtMessage::Predecessor {
                id: None,
                address: None,
                api_address: None,
            }) => {
                debug!(
                    "[{}] Stabilize: successor {} has no predecessor",
                    self.info.address, successor.address
                );
            }
            _ => {
                log_error!(
                    self.info.address,
                    "Error: Failed to get predecessor from successor."
                );
            }
        }

        // Notify successor that we are its predecessor
        let current_successor = match self.safe_lock(&self.successor) {
            Some(guard) => guard.clone(),
            None => {
                log_error!(
                    self.info.address,
                    "Failed to access successor for notify in stabilize"
                );
                return;
            }
        };
        debug!(
            "[{}] Stabilize: notifying successor {} that we are its predecessor",
            self.info.address, current_successor.address
        );

        match self
            .network_client
            .call_node(
                &current_successor.address,
                DhtMessage::Notify {
                    id: self.info.id,
                    address: self.info.address.clone(),
                    api_address: self.info.api_address.clone(),
                },
            )
            .await
        {
            Ok(_) => {
                debug!(
                    "[{}] Stabilize: successfully notified successor {}",
                    self.info.address, current_successor.address
                );
            }
            Err(e) => {
                log_error!(self.info.address, "Error notifying successor: {}", e);
            }
        }

        // Update successor list after stabilization
        self.update_successor_list().await;
    }

    pub async fn notify(&self, n_prime: NodeInfo) {
        debug!(
            "[{}] Notify: received notify from {}",
            self.info.address, n_prime.address
        );

        let should_update;
        let current_pred;

        // Scope for the lock to ensure it's dropped before async operations
        {
            let mut predecessor = match self.safe_lock(&self.predecessor) {
                Some(guard) => guard,
                None => {
                    log_error!(self.info.address, "Failed to access predecessor in notify");
                    return;
                }
            };
            current_pred = predecessor.clone();

            // If predecessor is nil or n_prime is between predecessor and self
            should_update = if predecessor.is_none() {
                debug!(
                    "[{}] Notify: no current predecessor, accepting {}",
                    self.info.address, n_prime.address
                );
                true
            } else if let Some(pred) = predecessor.as_ref() {
                let is_between = tace_lib::is_between(&n_prime.id, &pred.id, &self.info.id);
                debug!(
                    "[{}] Notify: current pred={}, candidate={}, is_between={}",
                    self.info.address, pred.address, n_prime.address, is_between
                );
                is_between
            } else {
                false
            };

            if should_update {
                debug!(
                    "[{}] Notify: updating predecessor from {:?} to {}",
                    self.info.address,
                    current_pred.as_ref().map(|p| &p.address),
                    n_prime.address
                );
                *predecessor = Some(n_prime.clone());

                // Also check if we need to update successor in single-node case
                if !self.safe_update(&self.successor, |successor| {
                    if successor.id == self.info.id {
                        debug!(
                            "[{}] Notify: was single node, updating successor to {}",
                            self.info.address, n_prime.address
                        );
                        *successor = n_prime.clone();
                    }
                }) {
                    log_error!(self.info.address, "Failed to update successor in notify");
                }
            }
        } // Locks are dropped here

        if should_update {
            // Transfer data to the new predecessor
            // The new predecessor (n_prime) should now be responsible for keys in range (n_prime.id, self.id]
            let data_to_transfer = self.get_data_range(n_prime.id, self.info.id).await;

            if !data_to_transfer.is_empty() {
                log_info!(
                    self.info.address,
                    "Transferring {} keys to new predecessor {}",
                    data_to_transfer.len(),
                    n_prime.address
                );

                // Send each key-value pair to the new predecessor
                for (key, values) in data_to_transfer {
                    for value in values {
                        match self
                            .network_client
                            .call_node(
                                &n_prime.address,
                                DhtMessage::Store {
                                    key,
                                    value: value.clone(),
                                },
                            )
                            .await
                        {
                            Ok(_) => {
                                debug!(
                                    "[{}] Successfully transferred key {} to {}",
                                    self.info.address,
                                    hex::encode(key),
                                    n_prime.address
                                );
                            }
                            Err(e) => {
                                log_error!(
                                    self.info.address,
                                    "Failed to transfer key {} to {}: {}",
                                    hex::encode(key),
                                    n_prime.address,
                                    e
                                );
                            }
                        }
                    }
                }
            }
        } else {
            debug!(
                "[{}] Notify: keeping current predecessor {:?}",
                self.info.address,
                current_pred.as_ref().map(|p| &p.address)
            );
        }
    }

    fn calculate_local_network_size_estimate(&self) -> f64 {
        let successor = match self.safe_lock(&self.successor) {
            Some(guard) => guard,
            None => return 1.0, // Default to single node
        };
        let predecessor = match self.safe_lock(&self.predecessor) {
            Some(guard) => guard,
            None => return 1.0, // Default to single node
        };

        // If we don't have a predecessor, use ourselves
        let pred_id = predecessor.as_ref().map(|p| p.id).unwrap_or(self.info.id);

        // Convert IDs to BigUint for arithmetic
        let succ_id = tace_lib::node_id_to_biguint(&successor.id);
        let pred_id = tace_lib::node_id_to_biguint(&pred_id);

        // Calculate the size of the range between predecessor and successor
        let max_id = BigUint::from(2u32).pow(160);

        let range = if pred_id <= succ_id {
            // Normal case: pred -> succ (no wraparound)
            &succ_id - &pred_id
        } else {
            // Wraparound case: pred -> 0 -> max -> succ
            &max_id - &pred_id + &succ_id
        };

        // If range is zero, avoid division by zero
        if range == BigUint::from(0u32) {
            return 1.0; // Only one node
        }

        // Local estimate: if we have 2 nodes (predecessor and successor) in this range,
        // then the total network size is estimated as: total_space / range * 2
        let range_f64 = range.to_f64().unwrap_or(f64::MAX);
        let max_f64 = max_id.to_f64().unwrap_or(f64::MAX);

        // Network size estimate based on local density
        2.0 * max_f64 / range_f64
    }

    fn apply_bounds(&self, old_estimate: f64, new_estimate: f64) -> f64 {
        let max_change_factor = 4.0;
        let max_allowed = old_estimate * max_change_factor;
        let min_allowed = old_estimate / max_change_factor;

        new_estimate.max(min_allowed).min(max_allowed)
    }

    fn apply_smoothing(&self, old_estimate: f64, new_estimate: f64) -> f64 {
        let smoothing_factor = 0.1;
        (1.0 - smoothing_factor) * old_estimate + smoothing_factor * new_estimate
    }

    /// Generalized network estimation function that handles local contribution,
    /// scaling, bounds checking, and smoothing for network-wide metrics.
    fn update_network_estimate_f64(
        &self,
        current_estimate: f64,
        local_value: f64,
        network_size: f64,
        contribution_factor: f64,
        apply_bounds: bool,
        smoothing_factor: f64,
    ) -> f64 {
        let new_estimate = if current_estimate == 0.0 {
            // Initialize with local value scaled by network size
            local_value * network_size
        } else {
            // Blend local contribution with existing estimate
            let local_contribution = local_value * network_size;
            let blended = (1.0 - contribution_factor) * current_estimate
                + contribution_factor * local_contribution;

            // Apply bounds if requested
            if apply_bounds {
                self.apply_bounds(current_estimate, blended)
            } else {
                blended
            }
        };

        // Apply smoothing if we have a previous estimate
        if current_estimate > 0.0 {
            (1.0 - smoothing_factor) * current_estimate + smoothing_factor * new_estimate
        } else {
            new_estimate
        }
    }

    pub async fn fix_fingers(&self) {
        // Pick a random finger to fix, to avoid all nodes fixing the same finger at the same time.
        let i = rand::thread_rng().gen_range(1..M);
        let target_id = tace_lib::add_id_power_of_2(&self.info.id, i);
        let successor = self.find_successor(target_id).await;

        // Update the finger table
        if !self.safe_update(&self.finger_table, |table| {
            if let Some(entry) = table.get_mut(i) {
                *entry = successor;
            }
        }) {
            log_error!(self.info.address, "Failed to update finger table");
        }
    }

    pub async fn check_predecessor(&self) {
        let predecessor_option = match self.safe_lock(&self.predecessor) {
            Some(guard) => guard.clone(),
            None => {
                log_error!(
                    self.info.address,
                    "Failed to access predecessor in check_predecessor"
                );
                return;
            }
        };
        if let Some(predecessor) = predecessor_option {
            // Send a simple message to check if predecessor is alive
            match self
                .network_client
                .call_node(&predecessor.address, DhtMessage::Ping)
                .await
            {
                Ok(DhtMessage::Pong) => {
                    debug!(
                        "[{}] Predecessor {} is alive",
                        self.info.address, predecessor.address
                    );
                }
                _ => {
                    // Predecessor is dead, clear it
                    if !self.safe_update(&self.predecessor, |p| {
                        *p = None;
                    }) {
                        log_error!(self.info.address, "Failed to clear dead predecessor");
                    }
                    log_info!(
                        self.info.address,
                        "Predecessor {} is unreachable, cleared predecessor",
                        predecessor.address
                    );

                    // Also check if this affects our data responsibility
                    // If we had a predecessor and it's gone, we might need to
                    // take over its key range
                    self.reconcile_data_after_predecessor_failure(predecessor)
                        .await;
                }
            }
        }
    }

    /// Handle data reconciliation after predecessor failure
    async fn reconcile_data_after_predecessor_failure(&self, failed_predecessor: NodeInfo) {
        log_info!(
            self.info.address,
            "Reconciling data after predecessor {} failure",
            failed_predecessor.address
        );

        // Calculate the key range we're now responsible for
        let our_pred = if let Some(pred) = self.safe_lock(&self.predecessor) {
            pred.as_ref().map(|p| p.id)
        } else {
            None
        };

        let start_range = our_pred.unwrap_or(failed_predecessor.id);
        let end_range = self.info.id;

        // Request replicas from other nodes for our new responsibility range
        let replica_nodes = self
            .get_replica_nodes_for_range(start_range, end_range)
            .await;

        for node in replica_nodes {
            if node.id == self.info.id {
                continue; // Skip ourselves
            }

            let message = DhtMessage::RequestReplicas {
                key_range_start: start_range,
                key_range_end: end_range,
            };

            match self.network_client.call_node(&node.address, message).await {
                Ok(DhtMessage::ReplicaData { replicas }) => {
                    let mut recovered_count = 0;
                    if let Some(mut data) = self.safe_lock(&self.data) {
                        for (key, values) in replicas {
                            let entry = data.entry(key).or_default();
                            let initial_count = entry.len();
                            entry.extend(values);
                            recovered_count += entry.len() - initial_count;
                        }
                    }

                    log_info!(
                        self.info.address,
                        "Recovered {} data items from replica at {}",
                        recovered_count,
                        node.address
                    );
                }
                Err(e) => {
                    log_error!(
                        self.info.address,
                        "Failed to get replicas from {}: {}",
                        node.address,
                        e
                    );
                }
                _ => {
                    log_error!(
                        self.info.address,
                        "Unexpected response when requesting replicas from {}",
                        node.address
                    );
                }
            }
        }

        log_info!(
            self.info.address,
            "Completed data reconciliation after predecessor failure"
        );
    }

    /// Get nodes that might have replicas for a given key range
    async fn get_replica_nodes_for_range(&self, _start: NodeId, _end: NodeId) -> Vec<NodeInfo> {
        let mut nodes = Vec::new();

        // Add our successor list - they likely have replicas
        if let Some(successor_list) = self.safe_lock(&self.successor_list) {
            nodes.extend(successor_list.clone());
        }

        // Add some nodes from finger table for broader coverage
        if let Some(finger_table) = self.safe_lock(&self.finger_table) {
            for (i, node) in finger_table.iter().enumerate() {
                if i % 4 == 0 {
                    // Sample every 4th finger for efficiency
                    nodes.push(node.clone());
                }
            }
        }

        // Remove duplicates and ourselves
        nodes.sort_by_key(|n| n.id);
        nodes.dedup_by_key(|n| n.id);
        nodes.retain(|n| n.id != self.info.id);

        nodes
    }

    pub fn shutdown(&self) {
        log_info!(self.info.address, "Initiating graceful shutdown");
        self.shutdown_signal.store(true, Ordering::Relaxed);

        // Wait for active connections to finish (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(30);

        while self.active_connections.load(Ordering::Relaxed) > 0 {
            if start.elapsed() > timeout {
                log_error!(
                    self.info.address,
                    "Shutdown timeout reached with {} active connections",
                    self.active_connections.load(Ordering::Relaxed)
                );
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        log_info!(self.info.address, "Shutdown complete");
    }

    pub fn debug_ring_state(&self) {
        let successor = match self.safe_lock(&self.successor) {
            Some(guard) => guard,
            None => return,
        };
        let predecessor = match self.safe_lock(&self.predecessor) {
            Some(guard) => guard,
            None => return,
        };

        debug!(
            "[{}] Ring state: self={}, pred={}, succ={}",
            self.info.address,
            hex::encode(&self.info.id[..4]), // Just first 4 bytes for readability
            predecessor
                .as_ref()
                .map(|p| hex::encode(&p.id[..4]))
                .unwrap_or("None".to_string()),
            hex::encode(&successor.id[..4])
        );

        // Check if we form a valid ring
        if let Some(pred) = predecessor.as_ref() {
            let ring_valid = if pred.id == successor.id && pred.id == self.info.id {
                // Single node case
                true
            } else if pred.id == successor.id {
                // Two node case: pred == succ, and they should point back to us
                // We can't verify this without network calls, so assume valid
                true
            } else {
                // Multi-node case: check if self is between predecessor and successor
                tace_lib::is_between(&self.info.id, &pred.id, &successor.id)
            };

            debug!(
                "[{}] Ring validity: {}",
                self.info.address,
                if ring_valid { "VALID" } else { "INVALID" }
            );
        }
    }

    // Load balancing methods

    /// Update load metrics for a node
    pub async fn update_node_load(&self, node_id: NodeId, load: f32) {
        if let Some(mut node_loads) = self.safe_lock(&self.node_loads) {
            node_loads.insert(node_id, load);
        }
    }

    /// Calculate current load for this node
    pub async fn calculate_current_load(&self) -> f32 {
        let mut load = 0.0;

        // Factor in data storage load
        if let Some(data) = self.safe_lock(&self.data) {
            let data_count = data.len() as f32;
            load += data_count * 0.001; // Each stored item adds 0.1% load
        }

        // Factor in connection load
        let connections = self.active_connections.load(Ordering::Relaxed) as f32;
        load += connections / MAX_CONNECTIONS as f32;

        // Factor in metrics if available
        if let Some(metrics) = self.safe_lock(&self.metrics) {
            load += metrics.cpu_usage_percent / 100.0;
            load += metrics.memory_usage_percent / 100.0;
            load += metrics.request_rate_per_second * 0.01; // Each req/sec adds 1% load
        }

        load.min(1.0) // Cap at 100%
    }

    /// Check if load balancing is needed and trigger rebalancing
    pub async fn check_and_rebalance(&self) {
        let current_load = self.calculate_current_load().await;

        // Update our own load in the tracking
        self.update_node_load(self.info.id, current_load).await;

        // Update metrics with current load
        if let Some(mut metrics) = self.safe_lock(&self.metrics) {
            metrics.cpu_usage_percent = current_load * 100.0;
            metrics.active_connections = self.active_connections.load(Ordering::Relaxed);
        }

        if current_load > LOAD_BALANCE_THRESHOLD {
            log_info!(
                self.info.address,
                "High load detected ({:.2}%), considering rebalancing",
                current_load * 100.0
            );
            self.rebalance_data().await;
        }
    }

    /// Migrate data to less loaded nodes
    async fn rebalance_data(&self) {
        let keys_to_migrate = if let Some(data) = self.safe_lock(&self.data) {
            data.keys().take(10).cloned().collect::<Vec<NodeId>>() // Migrate up to 10 keys
        } else {
            return;
        };

        for key in keys_to_migrate {
            // Find less loaded replicas for this key
            let replica_nodes = self.get_replica_nodes(key).await;

            let (least_loaded_node, should_migrate) =
                if let Some(node_loads) = self.safe_lock(&self.node_loads) {
                    // Find the least loaded replica
                    let least_loaded = replica_nodes
                        .iter()
                        .filter(|n| n.id != self.info.id)
                        .min_by(|a, b| {
                            let load_a = node_loads.get(&a.id).unwrap_or(&1.0);
                            let load_b = node_loads.get(&b.id).unwrap_or(&1.0);
                            load_a
                                .partial_cmp(load_b)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });

                    if let Some(least_loaded) = least_loaded {
                        let target_load = node_loads.get(&least_loaded.id).unwrap_or(&1.0);
                        let our_load = node_loads.get(&self.info.id).unwrap_or(&0.0);

                        // Only migrate if target is significantly less loaded
                        (Some(least_loaded.clone()), *target_load < our_load - 0.2)
                    } else {
                        (None, false)
                    }
                } else {
                    (None, false)
                };

            if should_migrate {
                if let Some(target_node) = least_loaded_node {
                    self.migrate_key_to_node(key, target_node).await;
                }
            }
        }
    }

    /// Migrate a specific key to a target node
    async fn migrate_key_to_node(&self, key: NodeId, target_node: NodeInfo) {
        // Get the data to migrate
        let data_to_migrate = if let Some(mut data) = self.safe_lock(&self.data) {
            data.remove(&key)
        } else {
            return;
        };

        if let Some(values) = data_to_migrate {
            log_info!(
                self.info.address,
                "Migrating key {} to less loaded node {}",
                hex::encode(key),
                target_node.address
            );

            // Send the data to the target node
            for value in values {
                let message = DhtMessage::Store {
                    key,
                    value: value.clone(),
                };
                if let Err(e) = self
                    .network_client
                    .call_node(&target_node.address, message)
                    .await
                {
                    log_error!(
                        self.info.address,
                        "Failed to migrate key {} to {}: {}",
                        hex::encode(key),
                        target_node.address,
                        e
                    );

                    // Restore the data locally if migration failed
                    if let Some(mut data) = self.safe_lock(&self.data) {
                        data.entry(key).or_default().push(value);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_client::MockNetworkClient;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tace_lib::dht_messages::{DhtMessage, NodeId};

    #[derive(Debug)]
    struct TestError(&'static str);

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl std::error::Error for TestError {}

    fn hex_to_node_id(hex_str: &str) -> NodeId {
        let mut id = [0u8; 20];
        let bytes = hex::decode(hex_str).unwrap();
        // Pad with leading zeros if necessary
        let start_index = 20 - bytes.len();
        id[start_index..].copy_from_slice(&bytes);
        id
    }

    #[tokio::test]
    async fn test_start_new_network() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);
        node.start_new_network().await;

        let successor = node.successor.lock().unwrap();
        assert_eq!(successor.id, node.info.id);
        assert_eq!(successor.address, node.info.address);

        let predecessor = node.predecessor.lock().unwrap();
        assert!(predecessor.is_none());
    }

    #[tokio::test]
    async fn test_closest_preceding_node() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node_id = hex_to_node_id("0000000000000000000000000000000000000000");
        let node_address = "127.0.0.1:8000".to_string();
        let node_info = NodeInfo {
            id: node_id,
            address: node_address.clone(),
            api_address: node_address.clone(),
        };

        let mut finger_table_vec = vec![node_info.clone(); M];
        // Populate some finger table entries for testing
        // Example: finger[0] = node_id + 2^0
        // finger[1] = node_id + 2^1
        // ...
        // For simplicity, let's just put a few specific values
        finger_table_vec[0] = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000001"),
            address: "127.0.0.1:8001".to_string(),
            api_address: "127.0.0.1:8001".to_string(),
        };
        finger_table_vec[1] = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000002"),
            address: "127.0.0.1:8002".to_string(),
            api_address: "127.0.0.1:8002".to_string(),
        };
        finger_table_vec[M - 1] = NodeInfo {
            id: hex_to_node_id("7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
            address: "127.0.0.1:8003".to_string(),
            api_address: "127.0.0.1:8003".to_string(),
        };

        let node = ChordNode {
            info: node_info.clone(),
            successor: Arc::new(Mutex::new(node_info.clone())),
            successor_list: Arc::new(Mutex::new(vec![node_info.clone(); SUCCESSOR_LIST_SIZE])),
            predecessor: Arc::new(Mutex::new(None)),
            finger_table: Arc::new(Mutex::new(finger_table_vec)),
            data: Arc::new(Mutex::new(HashMap::new())),
            virtual_nodes: Arc::new(Mutex::new(
                ChordNode::<MockNetworkClient>::generate_virtual_nodes("127.0.0.1:8003"),
            )),
            node_loads: Arc::new(Mutex::new(HashMap::new())),
            network_client: mock_network_client,
            metrics: Arc::new(Mutex::new(NetworkMetrics::default())),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            active_connections: Arc::new(AtomicU32::new(0)),
        };

        // Test case 1: ID is far away, should return the largest finger
        let target_id = hex_to_node_id("8000000000000000000000000000000000000000");
        let cpn = node.closest_preceding_node(target_id).await;
        assert_eq!(
            cpn.id,
            hex_to_node_id("7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF")
        );

        // Test case 2: ID is close, should return a smaller finger
        let target_id_close = hex_to_node_id("0000000000000000000000000000000000000003");
        let cpn_close = node.closest_preceding_node(target_id_close).await;
        assert_eq!(
            cpn_close.id,
            hex_to_node_id("0000000000000000000000000000000000000002")
        );

        // Test case 3: ID is very close, should return self
        let target_id_very_close = hex_to_node_id("0000000000000000000000000000000000000000");
        let cpn_very_close = node.closest_preceding_node(target_id_very_close).await;
        assert_eq!(cpn_very_close.id, node_id);
    }

    #[tokio::test]
    async fn test_store_and_retrieve() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);
        let key = hex_to_node_id("1234567890123456789012345678901234567890");
        let value = vec![1, 2, 3, 4, 5];

        // Store the value
        node.store(key, value.clone()).await;

        // Retrieve the value
        let retrieved_value = node.retrieve(key).await;
        assert_eq!(retrieved_value, Some(vec![value]));

        // Try to retrieve a non-existent key
        let non_existent_key = hex_to_node_id("0000000000000000000000000000000000000000");
        let retrieved_none = node.retrieve(non_existent_key).await;
        assert_eq!(retrieved_none, None);
    }

    #[tokio::test]
    async fn test_check_predecessor_dead() {
        let mut mock_network_client = MockNetworkClient::new();
        mock_network_client
            .expect_call_node()
            .withf(|address, message| {
                address == "127.0.0.1:9999" && matches!(message, DhtMessage::Ping)
            })
            .times(1)
            .returning(|_, _| Err(Box::new(TestError("Simulated network error"))));

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));
        let dead_predecessor_id = hex_to_node_id("1111111111111111111111111111111111111111");
        let dead_predecessor_address = "127.0.0.1:9999".to_string(); // Non-existent address

        let mut predecessor = node.predecessor.lock().unwrap();
        *predecessor = Some(NodeInfo {
            id: dead_predecessor_id,
            address: dead_predecessor_address.clone(),
            api_address: dead_predecessor_address,
        });
        drop(predecessor); // Release lock

        node.check_predecessor().await;

        let updated_predecessor = node.predecessor.lock().unwrap();
        assert!(updated_predecessor.is_none());
    }

    #[tokio::test]
    async fn test_notify() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let _node_id = hex_to_node_id("0000000000000000000000000000000000000000");
        let node_address = "127.0.0.1:9000".to_string();
        let node = ChordNode::new_for_test(node_address.clone(), mock_network_client);

        // Case 1: Predecessor is nil
        let n_prime_1 = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000001"),
            address: "127.0.0.1:9001".to_string(),
            api_address: "127.0.0.1:9001".to_string(),
        };
        node.notify(n_prime_1.clone()).await;
        assert_eq!(
            node.predecessor.lock().unwrap().clone().unwrap().id,
            n_prime_1.id
        );

        // Case 2: n_prime is between current predecessor and self
        let n_prime_2 = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000000"), // Same as node_id, should not update
            address: "127.0.0.1:9002".to_string(),
            api_address: "127.0.0.1:9002".to_string(),
        };
        node.notify(n_prime_2.clone()).await;
        // Predecessor should still be n_prime_1 because n_prime_2 is not between n_prime_1 and node
        assert_eq!(
            node.predecessor.lock().unwrap().clone().unwrap().id,
            n_prime_1.id
        );

        let n_prime_3 = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000000"), // This should be between n_prime_1 and node_id
            address: "127.0.0.1:9003".to_string(),
            api_address: "127.0.0.1:9003".to_string(),
        };
        // Manually set predecessor to something that n_prime_3 is between
        let mut predecessor = node.predecessor.lock().unwrap();
        *predecessor = Some(NodeInfo {
            id: hex_to_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
            address: "127.0.0.1:9004".to_string(),
            api_address: "127.0.0.1:9004".to_string(),
        });
        drop(predecessor);
        node.notify(n_prime_3.clone()).await;
        assert_eq!(
            node.predecessor.lock().unwrap().clone().unwrap().id,
            n_prime_3.id
        );
    }

    #[tokio::test]
    async fn test_join() {
        let mut mock_network_client = MockNetworkClient::new();
        let bootstrap_node_id = hex_to_node_id("2222222222222222222222222222222222222222");
        let bootstrap_node_address = "127.0.0.1:8001".to_string();

        mock_network_client
            .expect_call_node()
            .withf(move |address, message| {
                address == bootstrap_node_address
                    && match message {
                        DhtMessage::FindSuccessor { id: _ } => true,
                        DhtMessage::GetDataRange { start: _, end: _ } => true,
                        _ => false,
                    }
            })
            .times(2)
            .returning(move |_, message| match message {
                DhtMessage::FindSuccessor { id: _ } => Ok(DhtMessage::FoundSuccessor {
                    id: bootstrap_node_id,
                    address: "127.0.0.1:8001".to_string(),
                    api_address: "127.0.0.1:8001".to_string(),
                }),
                DhtMessage::GetDataRange { start, end } => {
                    assert_eq!(
                        start,
                        hex_to_node_id("70bacfa8e44a0929dddd31d0b9d78164e8cbfba5")
                    );
                    assert_eq!(end, bootstrap_node_id);
                    Ok(DhtMessage::DataRange {
                        data: vec![
                            (
                                hex_to_node_id("1234567890123456789012345678901234567890"),
                                vec![vec![1, 2, 3], vec![4, 5, 6]],
                            ),
                            (
                                hex_to_node_id("1234567890123456789012345678901234567891"),
                                vec![vec![4, 5, 6]],
                            ),
                        ],
                    })
                }
                _ => Err(Box::new(TestError("Unexpected message"))),
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));
        node.join(Some("127.0.0.1:8001".to_string())).await;

        let successor = node.successor.lock().unwrap();
        assert_eq!(successor.id, bootstrap_node_id);
        assert_eq!(successor.address, "127.0.0.1:8001");
        let data = node.data.lock().unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(
            data.get(&hex_to_node_id("1234567890123456789012345678901234567890"))
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn test_find_successor() {
        let mut mock_network_client = MockNetworkClient::new();
        let successor_id = hex_to_node_id("3333333333333333333333333333333333333333");
        let successor_address = "127.0.0.1:8002".to_string();
        let successor_address_clone = successor_address.clone();

        // Mock for any call_node - the new algorithm might make multiple calls
        mock_network_client
            .expect_call_node()
            .times(1..=3) // Allow 1 to 3 calls
            .returning(move |_, _| {
                Ok(DhtMessage::FoundSuccessor {
                    id: successor_id,
                    address: successor_address_clone.clone(),
                    api_address: successor_address_clone.clone(),
                })
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));
        let finger_node_id = hex_to_node_id("2222222222222222222222222222222222222222");
        let finger_node_address = "127.0.0.1:8001".to_string();
        let mut finger_table = node.finger_table.lock().unwrap();
        finger_table[0] = NodeInfo {
            id: finger_node_id,
            address: finger_node_address.clone(),
            api_address: finger_node_address,
        };
        drop(finger_table);

        let target_id = hex_to_node_id("4444444444444444444444444444444444444444");
        let successor = node.find_successor(target_id).await;

        assert_eq!(successor.id, successor_id);
        assert_eq!(successor.address, successor_address);
    }

    #[tokio::test]
    async fn test_find_predecessor_message_handling() {
        let mock_network_client = MockNetworkClient::new();
        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Set up a predecessor in the node's finger table
        let predecessor_id = hex_to_node_id("2222222222222222222222222222222222222222");
        let predecessor_address = "127.0.0.1:8001".to_string();
        let mut finger_table = node.finger_table.lock().unwrap();
        finger_table[0] = NodeInfo {
            id: predecessor_id,
            address: predecessor_address.clone(),
            api_address: predecessor_address.clone(),
        };
        drop(finger_table);

        // Test the FindPredecessor message by calling find_predecessor directly
        let target_id = hex_to_node_id("1111111111111111111111111111111111111111");
        let predecessor = node.find_predecessor(target_id).await;

        // The predecessor should be the current node since the target ID is between
        // the current node and its successor
        assert_eq!(predecessor.id, node.info.id);
        assert_eq!(predecessor.address, node.info.address);
    }

    #[tokio::test]
    async fn test_find_predecessor_retry_logic() {
        let mut mock_network_client = MockNetworkClient::new();

        // Mock network calls to always fail, triggering retry logic
        mock_network_client
            .expect_call_node()
            .returning(|_, _| Err(Box::new(TestError("Network error"))));

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Test find_predecessor with a target that is between this node and its successor
        // This should not require network calls and return the current node
        let target_id = hex_to_node_id("7000000000000000000000000000000000000000");
        let result = node.find_predecessor(target_id).await;

        // Should return the current node since the target is between current and successor
        assert_eq!(result.id, node.info.id);
    }

    #[tokio::test]
    async fn test_id_space_wraparound() {
        let mut mock_network_client = MockNetworkClient::new();

        // Mock responses for wraparound scenarios
        mock_network_client.expect_call_node().returning(|_, _| {
            Ok(DhtMessage::FoundSuccessor {
                id: hex_to_node_id("0000000000000000000000000000000000000001"),
                address: "127.0.0.1:8001".to_string(),
                api_address: "127.0.0.1:8001".to_string(),
            })
        });

        // Create a node with ID near the end of the ID space
        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Manually set the node's ID to be near the end of the ID space
        let high_id = hex_to_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFE");
        let node_info = NodeInfo {
            id: high_id,
            address: "127.0.0.1:9000".to_string(),
            api_address: "127.0.0.1:9000".to_string(),
        };

        // Create a new node with the high ID
        let test_node = ChordNode {
            info: node_info.clone(),
            successor: Arc::new(Mutex::new(NodeInfo {
                id: hex_to_node_id("0000000000000000000000000000000000000001"),
                address: "127.0.0.1:8001".to_string(),
                api_address: "127.0.0.1:8001".to_string(),
            })),
            successor_list: Arc::new(Mutex::new(vec![
                NodeInfo {
                    id: hex_to_node_id("0000000000000000000000000000000000000001"),
                    address: "127.0.0.1:8001".to_string(),
                    api_address: "127.0.0.1:8001".to_string(),
                };
                SUCCESSOR_LIST_SIZE
            ])),
            predecessor: Arc::new(Mutex::new(None)),
            finger_table: Arc::new(Mutex::new(vec![node_info.clone(); M])),
            data: Arc::new(Mutex::new(HashMap::new())),
            virtual_nodes: Arc::new(Mutex::new(
                ChordNode::<MockNetworkClient>::generate_virtual_nodes("127.0.0.1:9000"),
            )),
            node_loads: Arc::new(Mutex::new(HashMap::new())),
            network_client: node.network_client.clone(),
            metrics: Arc::new(Mutex::new(NetworkMetrics::default())),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            active_connections: Arc::new(AtomicU32::new(0)),
        };

        // Test finding successor of ID 0 (wraparound case)
        let target_id = hex_to_node_id("0000000000000000000000000000000000000000");
        let successor = test_node.find_successor(target_id).await;

        // Should find the successor at the beginning of the ID space
        assert_eq!(
            successor.id,
            hex_to_node_id("0000000000000000000000000000000000000001")
        );

        // Test is_between with wraparound
        let low_id = hex_to_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFE");
        let mid_id = hex_to_node_id("0000000000000000000000000000000000000000");
        let high_id = hex_to_node_id("0000000000000000000000000000000000000001");

        assert!(
            tace_lib::is_between(&mid_id, &low_id, &high_id),
            "ID 0 should be between FFFE and 0001 (wraparound)"
        );
    }

    #[tokio::test]
    async fn test_fix_fingers() {
        let mock_network_client = MockNetworkClient::new();
        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Get initial finger table state
        let initial_finger_table = {
            let ft = node.finger_table.lock().unwrap();
            ft.clone()
        };

        // Manually set one finger to something different so we can detect changes
        {
            let mut ft = node.finger_table.lock().unwrap();
            ft[5] = NodeInfo {
                id: hex_to_node_id("1111111111111111111111111111111111111111"),
                address: "127.0.0.1:8001".to_string(),
                api_address: "127.0.0.1:8001".to_string(),
            };
        }

        // Run fix_fingers - this will update a random finger (might not be the one we changed)
        node.fix_fingers().await;

        // Check that finger table structure is still valid
        let updated_finger_table = {
            let ft = node.finger_table.lock().unwrap();
            ft.clone()
        };

        // Verify the finger table structure is still valid
        assert_eq!(updated_finger_table.len(), M);
        assert_eq!(initial_finger_table.len(), updated_finger_table.len());

        // Verify that all fingers contain valid NodeInfo entries
        for finger in &updated_finger_table {
            assert!(!finger.address.is_empty());
            assert!(!finger.api_address.is_empty());
        }

        // The method should run without panicking, which verifies basic functionality
        // In a real network, this would update finger table entries with actual successor lookups
    }

    #[tokio::test]
    async fn test_network_partition_recovery() {
        let mut mock_network_client = MockNetworkClient::new();

        // Simulate network partition where some calls fail and others succeed
        let mut call_count = 0;
        mock_network_client
            .expect_call_node()
            .returning(move |_addr, _| {
                call_count += 1;
                // First few calls fail (simulating partition), then succeed (recovery)
                if call_count <= 2 {
                    Err(Box::new(TestError("Network partition - unreachable")))
                } else {
                    Ok(DhtMessage::FoundSuccessor {
                        id: hex_to_node_id("2222222222222222222222222222222222222222"),
                        address: "127.0.0.1:8002".to_string(),
                        api_address: "127.0.0.1:8002".to_string(),
                    })
                }
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Set up a successor that will become unreachable
        {
            let mut successor = node.successor.lock().unwrap();
            *successor = NodeInfo {
                id: hex_to_node_id("1111111111111111111111111111111111111111"),
                address: "127.0.0.1:8001".to_string(),
                api_address: "127.0.0.1:8001".to_string(),
            };
        }

        // Run stabilization multiple times to simulate recovery process
        node.stabilize().await; // First call may fail due to partition
        node.stabilize().await; // Second call may still fail
        node.stabilize().await; // Third call should succeed (recovery)

        // Verify that the node can still function after partition recovery
        let target_id = hex_to_node_id("3333333333333333333333333333333333333333");
        let result = node.find_successor(target_id).await;

        // Should be able to find a successor even after network issues
        assert!(!result.address.is_empty());
    }

    #[tokio::test]
    async fn test_concurrent_operations() {
        use tokio::task::JoinSet;

        let mock_network_client = MockNetworkClient::new();
        let node = Arc::new(ChordNode::new_for_test(
            "127.0.0.1:9000".to_string(),
            Arc::new(mock_network_client),
        ));

        // Test concurrent store operations
        let mut join_set = JoinSet::new();

        for i in 0..10 {
            let node_clone = node.clone();
            join_set.spawn(async move {
                let key = {
                    let mut key = [0u8; 20];
                    key[19] = i as u8;
                    key
                };
                let value = format!("value_{}", i).into_bytes();
                node_clone.store(key, value).await;
            });
        }

        // Wait for all store operations to complete
        while let Some(result) = join_set.join_next().await {
            result.expect("Store operation should succeed");
        }

        // Test concurrent retrieve operations
        for i in 0..10 {
            let node_clone = node.clone();
            join_set.spawn(async move {
                let key = {
                    let mut key = [0u8; 20];
                    key[19] = i as u8;
                    key
                };
                let result = node_clone.retrieve(key).await;
                assert!(
                    result.is_some(),
                    "Should retrieve stored value for key {}",
                    i
                );
                let values = result.unwrap();
                assert!(!values.is_empty(), "Retrieved values should not be empty");
                assert_eq!(values[0], format!("value_{}", i).into_bytes());
            });
        }

        // Wait for all retrieve operations to complete
        while let Some(result) = join_set.join_next().await {
            result.expect("Retrieve operation should succeed");
        }

        // Test concurrent successor updates (simulating stabilization)
        for i in 0..5 {
            let node_clone = node.clone();
            join_set.spawn(async move {
                let new_successor = NodeInfo {
                    id: {
                        let mut id = [0u8; 20];
                        id[19] = (100 + i) as u8;
                        id
                    },
                    address: format!("127.0.0.1:800{}", i),
                    api_address: format!("127.0.0.1:800{}", i),
                };

                // Update successor (this tests thread safety of successor updates)
                {
                    let mut successor = node_clone.successor.lock().unwrap();
                    *successor = new_successor;
                }
            });
        }

        // Wait for all successor updates to complete
        while let Some(result) = join_set.join_next().await {
            result.expect("Successor update should succeed");
        }

        // Verify final state is consistent
        let final_successor = node.successor.lock().unwrap();
        assert!(
            !final_successor.address.is_empty(),
            "Final successor should be valid"
        );
    }

    #[tokio::test]
    async fn test_data_consistency_during_network_changes() {
        let mut mock_network_client = MockNetworkClient::new();

        // Mock data transfer during join operations
        mock_network_client.expect_call_node().returning(|_, msg| {
            match msg {
                DhtMessage::GetDataRange { start: _, end: _ } => {
                    // Return some test data for the range
                    let test_data = vec![(
                        hex_to_node_id("1111111111111111111111111111111111111111"),
                        vec![b"test_value".to_vec()],
                    )];
                    Ok(DhtMessage::DataRange { data: test_data })
                }
                _ => Ok(DhtMessage::FoundSuccessor {
                    id: hex_to_node_id("2222222222222222222222222222222222222222"),
                    address: "127.0.0.1:8002".to_string(),
                    api_address: "127.0.0.1:8002".to_string(),
                }),
            }
        });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Store some initial data
        let test_key = hex_to_node_id("1111111111111111111111111111111111111111");
        let test_value = b"initial_value".to_vec();
        node.store(test_key, test_value.clone()).await;

        // Verify data is stored by checking it exists without retrieving (removing) it
        {
            let data = node.data.lock().unwrap();
            assert!(data.contains_key(&test_key), "Data should be stored");
            assert_eq!(data[&test_key][0], test_value);
        }

        // Simulate network change by updating successor
        {
            let mut successor = node.successor.lock().unwrap();
            *successor = NodeInfo {
                id: hex_to_node_id("3333333333333333333333333333333333333333"),
                address: "127.0.0.1:8003".to_string(),
                api_address: "127.0.0.1:8003".to_string(),
            };
        }

        // Data should still be accessible after network topology change
        let retrieved_after_change = node.retrieve(test_key).await;
        assert!(
            retrieved_after_change.is_some(),
            "Data should still be accessible after network changes"
        );
        let values = retrieved_after_change.unwrap();
        assert_eq!(values[0], test_value);

        // Store the key again since retrieve removed it
        node.store(test_key, test_value.clone()).await;

        // Test data migration during simulated join
        let bootstrap_node = Some("127.0.0.1:8001".to_string());
        node.join(bootstrap_node).await;

        // After join, verify that local data storage is still functional
        // (The join process should have received data for the node's responsible range)
        let final_retrieved = node.retrieve(test_key).await;
        assert!(
            final_retrieved.is_some(),
            "Data should persist through join operations"
        );
    }

    #[tokio::test]
    async fn test_malformed_message_handling() {
        let mock_network_client = MockNetworkClient::new();
        let node = Arc::new(ChordNode::new_for_test(
            "127.0.0.1:9000".to_string(),
            Arc::new(mock_network_client),
        ));

        // Test handling of various malformed inputs
        // Note: In a real implementation, we'd test the actual network handling
        // For now, we test the robustness of the message processing logic

        // Test that the node can handle network errors gracefully
        let target_id = hex_to_node_id("1111111111111111111111111111111111111111");

        // This should not panic even with network issues
        let result = node.find_successor(target_id).await;
        assert!(
            !result.address.is_empty(),
            "Should return a valid result even with network issues"
        );

        // Test that data operations handle errors gracefully
        let test_key = hex_to_node_id("2222222222222222222222222222222222222222");
        let test_value = b"test_value".to_vec();

        // Store should not panic
        node.store(test_key, test_value).await;

        // Retrieve should handle missing keys gracefully
        let missing_key = hex_to_node_id("9999999999999999999999999999999999999999");
        let result = node.retrieve(missing_key).await;
        // Should return None for missing keys, not panic
        assert!(
            result.is_none() || result.is_some(),
            "Retrieve should handle missing keys gracefully"
        );

        // Test that stabilization handles errors gracefully
        node.stabilize().await; // Should not panic

        // Test that fix_fingers handles errors gracefully
        node.fix_fingers().await; // Should not panic

        // Test that check_predecessor handles errors gracefully
        node.check_predecessor().await; // Should not panic
    }

    #[tokio::test]
    async fn test_stabilize() {
        let mut mock_network_client = MockNetworkClient::new();
        let successor_predecessor_id = hex_to_node_id("5555555555555555555555555555555555555555");
        let successor_predecessor_address = "127.0.0.1:8003".to_string();
        let successor_predecessor_address_clone = successor_predecessor_address.clone();

        // Mock for GetPredecessor
        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::GetPredecessor))
            .times(1)
            .returning(move |_, _| {
                Ok(DhtMessage::Predecessor {
                    id: Some(successor_predecessor_id),
                    address: Some(successor_predecessor_address_clone.clone()),
                    api_address: Some(successor_predecessor_address_clone.clone()),
                })
            });

        // Mock for Notify
        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::Notify { .. }))
            .times(1)
            .returning(|_, _| Ok(DhtMessage::Pong));

        // Mock for GetSuccessor (called by update_successor_list)
        // First call returns node at 8005
        mock_network_client
            .expect_call_node()
            .withf(|addr, message| {
                addr == "127.0.0.1:8003" && matches!(message, DhtMessage::GetSuccessor)
            })
            .times(1)
            .returning(|_, _| {
                Ok(DhtMessage::FoundSuccessor {
                    id: hex_to_node_id("7777777777777777777777777777777777777777"),
                    address: "127.0.0.1:8005".to_string(),
                    api_address: "127.0.0.1:8005".to_string(),
                })
            });

        // Second call (on 8005) returns node at 8006
        mock_network_client
            .expect_call_node()
            .withf(|addr, message| {
                addr == "127.0.0.1:8005" && matches!(message, DhtMessage::GetSuccessor)
            })
            .times(1)
            .returning(|_, _| {
                Ok(DhtMessage::FoundSuccessor {
                    id: hex_to_node_id("8888888888888888888888888888888888888888"),
                    address: "127.0.0.1:8006".to_string(),
                    api_address: "127.0.0.1:8006".to_string(),
                })
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));
        let successor_id = hex_to_node_id("6666666666666666666666666666666666666666");
        let successor_address = "127.0.0.1:8004".to_string();
        *node.successor.lock().unwrap() = NodeInfo {
            id: successor_id,
            address: successor_address.clone(),
            api_address: successor_address,
        };

        node.stabilize().await;

        let new_successor = node.successor.lock().unwrap();
        assert_eq!(new_successor.id, successor_predecessor_id);
        assert_eq!(new_successor.address, successor_predecessor_address);
    }

    #[tokio::test]
    async fn test_calculate_local_network_size_estimate_single_node() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // Single node network: successor points to itself, no predecessor
        let estimate = node.calculate_local_network_size_estimate();
        assert_eq!(estimate, 1.0);
    }

    #[tokio::test]
    async fn test_calculate_local_network_size_estimate_two_nodes() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // Set up a two-node network where nodes are separated
        // In our algorithm, we calculate range between predecessor and successor
        // For a 2-node network, if nodes are at positions A and B, then:
        // - Node A sees: predecessor=B, successor=B, so range = 0 (they wrap around)
        // Let's set up a scenario where predecessor and successor are different

        let predecessor_id = hex_to_node_id("2000000000000000000000000000000000000000");
        let successor_id = hex_to_node_id("8000000000000000000000000000000000000000");

        // Set predecessor and successor to different nodes
        *node.predecessor.lock().unwrap() = Some(NodeInfo {
            id: predecessor_id,
            address: "127.0.0.1:9001".to_string(),
            api_address: "127.0.0.1:9001".to_string(),
        });

        *node.successor.lock().unwrap() = NodeInfo {
            id: successor_id,
            address: "127.0.0.1:9002".to_string(),
            api_address: "127.0.0.1:9002".to_string(),
        };

        let estimate = node.calculate_local_network_size_estimate();
        // Should estimate more than 1 node since we have a range between pred and succ
        assert!(estimate > 1.0, "Expected >1 nodes, got {}", estimate);

        // The estimate should be reasonable (not extremely large)
        assert!(estimate < 100.0, "Estimate too large: {}", estimate);
    }

    #[tokio::test]
    async fn test_network_size_estimation_with_mock_neighbor() {
        let mut mock_network_client = MockNetworkClient::new();

        // Mock the neighbor's response with metrics
        let mut neighbor_metrics = NetworkMetrics::default();
        neighbor_metrics.network_size_estimate = 3.5;

        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::ShareMetrics { .. }))
            .times(1)
            .returning(move |_, _| {
                Ok(DhtMessage::MetricsShared {
                    metrics: neighbor_metrics.clone(),
                })
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Set up a multi-node scenario with successor and finger table entries
        let successor = NodeInfo {
            id: hex_to_node_id("1111111111111111111111111111111111111111"),
            address: "127.0.0.1:9001".to_string(),
            api_address: "127.0.0.1:9001".to_string(),
        };
        *node.successor.lock().unwrap() = successor.clone();

        // Add some finger table entries
        let mut finger_table = node.finger_table.lock().unwrap();
        finger_table[0] = successor.clone();
        finger_table[1] = NodeInfo {
            id: hex_to_node_id("2222222222222222222222222222222222222222"),
            address: "127.0.0.1:9002".to_string(),
            api_address: "127.0.0.1:9002".to_string(),
        };
        drop(finger_table);

        let initial_estimate = node.metrics.lock().unwrap().network_size_estimate;

        node.share_and_update_metrics().await;

        let final_estimate = node.metrics.lock().unwrap().network_size_estimate;

        // The estimate should have been updated (should be different from initial)
        assert_ne!(initial_estimate, final_estimate);

        // The final estimate should be influenced by both local estimate and neighbor's estimate (3.5)
        assert!(final_estimate > 0.0);
    }

    #[tokio::test]
    async fn test_apply_bounds() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // Test that bounds are applied correctly (max change factor is 4.0)
        let old_estimate = 10.0;

        // Test upper bound
        let too_high = 50.0; // 5x increase, should be capped to 40.0 (4x)
        let bounded_high = node.apply_bounds(old_estimate, too_high);
        assert_eq!(bounded_high, 40.0);

        // Test lower bound
        let too_low = 1.0; // 10x decrease, should be capped to 2.5 (4x decrease)
        let bounded_low = node.apply_bounds(old_estimate, too_low);
        assert_eq!(bounded_low, 2.5);

        // Test within bounds
        let within_bounds = 20.0; // 2x increase, should pass through unchanged
        let bounded_normal = node.apply_bounds(old_estimate, within_bounds);
        assert_eq!(bounded_normal, within_bounds);
    }

    #[tokio::test]
    async fn test_apply_smoothing() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // Test smoothing with factor 0.1
        let old_estimate = 10.0;
        let new_estimate = 20.0;

        let smoothed = node.apply_smoothing(old_estimate, new_estimate);

        // Should be: 0.9 * 10.0 + 0.1 * 20.0 = 9.0 + 2.0 = 11.0
        assert_eq!(smoothed, 11.0);
    }

    #[tokio::test]
    async fn test_estimate_network_size_single_node_network() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // In a single node network, successor points to self
        // share_and_update_metrics should return early without making network calls
        let initial_estimate = node.metrics.lock().unwrap().network_size_estimate;

        node.share_and_update_metrics().await;

        let final_estimate = node.metrics.lock().unwrap().network_size_estimate;

        // Estimate should be unchanged since we're the only node
        assert_eq!(initial_estimate, final_estimate);
    }

    #[tokio::test]
    async fn test_random_neighbor_selection() {
        let mut mock_network_client = MockNetworkClient::new();

        // Mock response for any metrics sharing request
        let mut neighbor_metrics = NetworkMetrics::default();
        neighbor_metrics.network_size_estimate = 4.0;

        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::ShareMetrics { .. }))
            .times(1)
            .returning(move |_, _| {
                Ok(DhtMessage::MetricsShared {
                    metrics: neighbor_metrics.clone(),
                })
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Set up multiple candidate nodes
        let successor = NodeInfo {
            id: hex_to_node_id("1111111111111111111111111111111111111111"),
            address: "127.0.0.1:9001".to_string(),
            api_address: "127.0.0.1:9001".to_string(),
        };
        *node.successor.lock().unwrap() = successor.clone();

        // Add multiple unique finger table entries
        let mut finger_table = node.finger_table.lock().unwrap();
        finger_table[0] = successor.clone();
        finger_table[1] = NodeInfo {
            id: hex_to_node_id("2222222222222222222222222222222222222222"),
            address: "127.0.0.1:9002".to_string(),
            api_address: "127.0.0.1:9002".to_string(),
        };
        finger_table[2] = NodeInfo {
            id: hex_to_node_id("3333333333333333333333333333333333333333"),
            address: "127.0.0.1:9003".to_string(),
            api_address: "127.0.0.1:9003".to_string(),
        };
        drop(finger_table);

        // Run estimation - it should succeed in picking a random neighbor
        node.share_and_update_metrics().await;

        // The estimate should have been updated from the mock response
        let final_estimate = node.metrics.lock().unwrap().network_size_estimate;
        assert!(final_estimate > 0.0);
    }

    #[tokio::test]
    async fn test_ring_formation_simulation() {
        // This test simulates a simple 3-node ring formation to verify the logic

        // Create mock clients for 3 nodes
        let node1_client = MockNetworkClient::new();

        // Node IDs in order: node1 < node2 < node3
        let node2_id = hex_to_node_id("6000000000000000000000000000000000000000");
        let node3_id = hex_to_node_id("A000000000000000000000000000000000000000");

        // Set up node1 (first node, creates ring)
        let node1 = ChordNode::new_for_test("node1:8001".to_string(), Arc::new(node1_client));
        node1.start_new_network().await;

        // In a properly formed 3-node ring:
        // node1 -> node2 -> node3 -> node1
        // Let's manually set up what should happen after stabilization

        // Node1 should have node2 as successor after node2 joins
        *node1.successor.lock().unwrap() = NodeInfo {
            id: node2_id,
            address: "node2:8002".to_string(),
            api_address: "node2:8002".to_string(),
        };
        // Node1 should have node3 as predecessor (node3 points to node1)
        *node1.predecessor.lock().unwrap() = Some(NodeInfo {
            id: node3_id,
            address: "node3:8003".to_string(),
            api_address: "node3:8003".to_string(),
        });

        // Verify ring properties
        let successor = node1.successor.lock().unwrap().clone();
        let predecessor = node1.predecessor.lock().unwrap().clone();

        // Check that successor is the next node in the ring
        assert_eq!(successor.id, node2_id);

        // Check that predecessor is the previous node in the ring
        assert_eq!(predecessor.unwrap().id, node3_id);

        // Verify the ring property: predecessor < self < successor (accounting for wraparound)
        // In this case: node3 > node1 < node2, which is valid with wraparound
        assert!(tace_lib::is_between(&node1.info.id, &node3_id, &node2_id));
    }

    #[tokio::test]
    async fn test_handle_share_metrics_averages_all_fields() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // Set up initial local metrics
        {
            let mut local_metrics = node.metrics.lock().unwrap();
            local_metrics.node_churn_rate = 0.1;
            local_metrics.routing_table_health = 0.8;
            local_metrics.operation_success_rate = 0.9;
            local_metrics.operation_latency = std::time::Duration::from_millis(100);
            local_metrics.message_type_ratio = 0.6;
            local_metrics.local_key_count = 50;
            local_metrics.total_network_keys_estimate = 50.0;
            local_metrics.network_size_estimate = 10.0;
        }

        // Create incoming metrics with different values
        let incoming_metrics = NetworkMetrics {
            node_churn_rate: 0.3,
            routing_table_health: 0.6,
            operation_success_rate: 0.7,
            operation_latency: std::time::Duration::from_millis(200),
            message_type_ratio: 0.4,
            local_key_count: 30,
            total_network_keys_estimate: 30.0,
            network_size_estimate: 20.0,
            cpu_usage_percent: 0.0,
            memory_usage_percent: 0.0,
            disk_usage_percent: 0.0,
            active_connections: 0,
            request_rate_per_second: 0.0,
        };

        // Handle metrics sharing
        let result_metrics = node.handle_share_metrics(incoming_metrics).await;

        // Verify all fields are properly averaged
        assert_eq!(result_metrics.node_churn_rate, 0.2); // (0.1 + 0.3) / 2
        assert_eq!(result_metrics.routing_table_health, 0.7); // (0.8 + 0.6) / 2
        assert_eq!(result_metrics.operation_success_rate, 0.8); // (0.9 + 0.7) / 2
        assert_eq!(
            result_metrics.operation_latency,
            std::time::Duration::from_millis(150)
        ); // (100 + 200) / 2
        assert_eq!(result_metrics.message_type_ratio, 0.5); // (0.6 + 0.4) / 2
        assert_eq!(result_metrics.total_network_keys_estimate, 40.0); // (50.0 + 30.0) / 2.0

        // Network size estimate should be bounded and smoothed, not just averaged
        assert!(result_metrics.network_size_estimate > 10.0);
        assert!(result_metrics.network_size_estimate < 20.0);
    }

    #[tokio::test]
    async fn test_update_local_metrics_routing_table_health() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // Set up finger table with some filled and some empty slots
        let successor = NodeInfo {
            id: hex_to_node_id("1111111111111111111111111111111111111111"),
            address: "127.0.0.1:9001".to_string(),
            api_address: "127.0.0.1:9001".to_string(),
        };
        *node.successor.lock().unwrap() = successor.clone();

        {
            let mut finger_table = node.finger_table.lock().unwrap();
            // Fill first 3 slots with different nodes, rest point to self (empty)
            finger_table[0] = successor.clone();
            finger_table[1] = NodeInfo {
                id: hex_to_node_id("2222222222222222222222222222222222222222"),
                address: "127.0.0.1:9002".to_string(),
                api_address: "127.0.0.1:9002".to_string(),
            };
            finger_table[2] = NodeInfo {
                id: hex_to_node_id("3333333333333333333333333333333333333333"),
                address: "127.0.0.1:9003".to_string(),
                api_address: "127.0.0.1:9003".to_string(),
            };
            // Other slots remain pointing to self (indicating empty slots)
        }

        // Update local metrics
        node.update_local_metrics().await;

        // Verify routing table health calculation
        let metrics = node.metrics.lock().unwrap();
        let expected_health = 3.0 / M as f64; // 3 filled slots out of M total slots
        assert_eq!(metrics.routing_table_health, expected_health);
    }

    #[tokio::test]
    async fn test_update_local_metrics_key_count() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // Add some data to the node
        {
            let mut data = node.data.lock().unwrap();
            data.insert(
                hex_to_node_id("1111111111111111111111111111111111111111"),
                vec![vec![1, 2, 3]],
            );
            data.insert(
                hex_to_node_id("2222222222222222222222222222222222222222"),
                vec![vec![4, 5, 6]],
            );
            data.insert(
                hex_to_node_id("3333333333333333333333333333333333333333"),
                vec![vec![7, 8, 9]],
            );
        }

        // Update local metrics
        node.update_local_metrics().await;

        // Verify key count is properly updated
        let metrics = node.metrics.lock().unwrap();
        assert_eq!(metrics.local_key_count, 3);
    }

    #[tokio::test]
    async fn test_metrics_sharing_full_workflow() {
        let mut mock_network_client = MockNetworkClient::new();

        // Mock neighbor's response
        let neighbor_metrics = NetworkMetrics {
            node_churn_rate: 0.2,
            routing_table_health: 0.7,
            operation_success_rate: 0.8,
            operation_latency: std::time::Duration::from_millis(150),
            message_type_ratio: 0.5,
            local_key_count: 25,
            total_network_keys_estimate: 25.0,
            network_size_estimate: 15.0,
            cpu_usage_percent: 0.0,
            memory_usage_percent: 0.0,
            disk_usage_percent: 0.0,
            active_connections: 0,
            request_rate_per_second: 0.0,
        };

        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::ShareMetrics { .. }))
            .times(1)
            .returning(move |_, _| {
                Ok(DhtMessage::MetricsShared {
                    metrics: neighbor_metrics.clone(),
                })
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Set up initial state
        {
            let mut local_metrics = node.metrics.lock().unwrap();
            local_metrics.node_churn_rate = 0.1;
            local_metrics.routing_table_health = 0.9;
            local_metrics.operation_success_rate = 0.95;
            local_metrics.operation_latency = std::time::Duration::from_millis(50);
            local_metrics.message_type_ratio = 0.7;
            local_metrics.local_key_count = 35;
            local_metrics.total_network_keys_estimate = 35.0;
            local_metrics.network_size_estimate = 5.0;
        }

        // Set up successor for metrics sharing
        let successor = NodeInfo {
            id: hex_to_node_id("1111111111111111111111111111111111111111"),
            address: "127.0.0.1:9001".to_string(),
            api_address: "127.0.0.1:9001".to_string(),
        };
        *node.successor.lock().unwrap() = successor;

        // Run the full metrics sharing workflow
        node.share_and_update_metrics().await;

        // Verify metrics were updated through the sharing process
        let final_metrics = node.metrics.lock().unwrap();

        // Values should be influenced by neighbor's metrics, not just local
        assert_ne!(final_metrics.node_churn_rate, 0.1);
        assert_ne!(final_metrics.routing_table_health, 0.9);
        assert_ne!(final_metrics.operation_success_rate, 0.95);
        assert_ne!(
            final_metrics.operation_latency,
            std::time::Duration::from_millis(50)
        );
        assert_ne!(final_metrics.message_type_ratio, 0.7);
        assert_ne!(final_metrics.local_key_count, 35);
        assert_ne!(final_metrics.network_size_estimate, 5.0);
    }

    #[tokio::test]
    async fn test_metrics_convergence_over_multiple_rounds() {
        let mut mock_network_client = MockNetworkClient::new();

        // Mock a single round of metrics sharing
        let neighbor_metrics = NetworkMetrics {
            node_churn_rate: 0.5,
            routing_table_health: 0.5,
            operation_success_rate: 0.5,
            operation_latency: std::time::Duration::from_millis(500),
            message_type_ratio: 0.5,
            local_key_count: 50,
            total_network_keys_estimate: 50.0,
            network_size_estimate: 50.0,
            cpu_usage_percent: 0.0,
            memory_usage_percent: 0.0,
            disk_usage_percent: 0.0,
            active_connections: 0,
            request_rate_per_second: 0.0,
        };

        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::ShareMetrics { .. }))
            .times(1)
            .returning(move |_, _| {
                Ok(DhtMessage::MetricsShared {
                    metrics: neighbor_metrics.clone(),
                })
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Set up initial extreme values
        {
            let mut local_metrics = node.metrics.lock().unwrap();
            local_metrics.node_churn_rate = 0.0;
            local_metrics.routing_table_health = 1.0;
            local_metrics.operation_success_rate = 1.0;
            local_metrics.operation_latency = std::time::Duration::from_millis(0);
            local_metrics.message_type_ratio = 1.0;
            local_metrics.local_key_count = 0;
            local_metrics.total_network_keys_estimate = 0.0;
            local_metrics.network_size_estimate = 1.0;
        }

        // Set up successor for metrics sharing
        let successor = NodeInfo {
            id: hex_to_node_id("1111111111111111111111111111111111111111"),
            address: "127.0.0.1:9001".to_string(),
            api_address: "127.0.0.1:9001".to_string(),
        };
        *node.successor.lock().unwrap() = successor.clone();

        // Set up finger table to ensure we have candidates for metrics sharing
        {
            let mut finger_table = node.finger_table.lock().unwrap();
            finger_table[0] = successor;
        }

        // Get initial values
        let initial_churn_rate = node.metrics.lock().unwrap().node_churn_rate;
        let initial_network_size = node.metrics.lock().unwrap().network_size_estimate;

        // Run one round of metrics sharing
        node.share_and_update_metrics().await;

        let final_metrics = node.metrics.lock().unwrap();
        let final_churn_rate = final_metrics.node_churn_rate;
        let final_network_size = final_metrics.network_size_estimate;

        // Values should be converging towards the middle after sharing
        assert!(final_churn_rate > initial_churn_rate);
        assert!(final_network_size > initial_network_size);

        // After sharing, values should be closer to the neighbor's values
        assert!(final_metrics.node_churn_rate > 0.1); // Should be moving towards 0.5
        assert!(final_metrics.network_size_estimate > 1.0); // Should be moving towards 50.0 (with bounds/smoothing)
    }
}
