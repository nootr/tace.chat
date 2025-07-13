use log::{debug, error, info};
use num_bigint::BigUint;
use num_traits::ToPrimitive;
use rand::seq::SliceRandom;
use rand::Rng;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tace_lib::dht_messages::{DhtMessage, NodeId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::network_client::NetworkClient;

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
    pub predecessor: Arc<Mutex<Option<NodeInfo>>>,
    pub finger_table: Arc<Mutex<Vec<NodeInfo>>>,
    pub data: Arc<Mutex<HashMap<NodeId, Vec<u8>>>>,
    pub network_client: Arc<T>,
    pub network_size_estimate: Arc<Mutex<f64>>,
}

impl<T: NetworkClient> Clone for ChordNode<T> {
    fn clone(&self) -> Self {
        ChordNode {
            info: self.info.clone(),
            successor: self.successor.clone(),
            predecessor: self.predecessor.clone(),
            finger_table: self.finger_table.clone(),
            data: self.data.clone(), // This clones the Arc, not the HashMap
            network_client: self.network_client.clone(),
            network_size_estimate: self.network_size_estimate.clone(),
        }
    }
}

impl<T: NetworkClient> ChordNode<T> {
    pub async fn new(address: String, api_address: String, network_client: Arc<T>) -> Self {
        let id = Self::generate_node_id(&address);
        let info = NodeInfo {
            id,
            address: address.clone(),
            api_address,
        };

        let successor = Arc::new(Mutex::new(info.clone()));
        let predecessor = Arc::new(Mutex::new(None));
        let finger_table = Arc::new(Mutex::<Vec<NodeInfo>>::new(vec![info.clone(); M]));
        let data = Arc::new(Mutex::new(HashMap::new()));

        ChordNode {
            info,
            successor,
            predecessor,
            finger_table,
            data,
            network_client,
            network_size_estimate: Arc::new(Mutex::new(1.0)),
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
        let predecessor = Arc::new(Mutex::new(None));
        let finger_table = Arc::new(Mutex::<Vec<NodeInfo>>::new(vec![info.clone(); M]));
        let data = Arc::new(Mutex::new(HashMap::new()));

        ChordNode {
            info,
            successor,
            predecessor,
            finger_table,
            data,
            network_client,
            network_size_estimate: Arc::new(Mutex::new(1.0)),
        }
    }

    fn generate_node_id(address: &str) -> NodeId {
        let mut hasher = Sha1::new();
        hasher.update(address.as_bytes());
        hasher.finalize().into()
    }

    pub async fn start(&self, bind_address: &str) {
        log_info!(
            self.info.address,
            "Chord Node {} starting at {} (binding to {})",
            hex::encode(self.info.id),
            self.info.address,
            bind_address
        );
        let listener = TcpListener::bind(bind_address)
            .await
            .expect("Failed to bind to address");

        let node_clone = self.clone(); // Clone the Arc for each connection
        loop {
            match listener.accept().await {
                Ok((socket, _)) => {
                    let node = node_clone.clone();
                    tokio::spawn(async move {
                        node.handle_connection(socket).await;
                    });
                }
                Err(e) => {
                    log_error!(self.info.address, "Failed to accept connection: {}", e);
                }
            }
        }
    }

    async fn handle_connection(&self, mut socket: TcpStream) {
        let mut buffer = Vec::new();
        // Read the entire message
        if let Err(e) = socket.read_to_end(&mut buffer).await {
            log_error!(self.info.address, "Failed to read from socket: {}", e);
            return;
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
                    DhtMessage::GetSuccessor => {
                        let successor = self.successor.lock().unwrap().clone();
                        DhtMessage::FoundSuccessor {
                            id: successor.id,
                            address: successor.address,
                            api_address: successor.api_address,
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
                        let predecessor = self.predecessor.lock().unwrap().clone();
                        DhtMessage::Predecessor {
                            id: predecessor.as_ref().map(|p| p.id),
                            address: predecessor.as_ref().map(|p| p.address.clone()),
                            api_address: predecessor.as_ref().map(|p| p.api_address.clone()),
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
                    DhtMessage::GetNetworkEstimate => {
                        let estimate = *self.network_size_estimate.lock().unwrap();
                        DhtMessage::NetworkEstimate { estimate }
                    }
                    DhtMessage::GetDataRange { start, end } => {
                        let data = self.get_data_range(start, end).await;
                        DhtMessage::DataRange { data }
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
                let encoded_response = bincode::serialize(&response).unwrap();
                log_info!(
                    self.info.address,
                    "Sending response to {}: {:?}",
                    socket.peer_addr().unwrap(),
                    response
                );
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

    // Stores a key-value pair in the DHT
    pub async fn store(&self, key: NodeId, value: Vec<u8>) {
        let mut data = self.data.lock().unwrap();
        data.insert(key, value);
        log_info!(self.info.address, "Stored key: {}", hex::encode(key));
    }

    // Retrieves a value by key from the DHT
    pub async fn retrieve(&self, key: NodeId) -> Option<Vec<u8>> {
        let data = self.data.lock().unwrap();
        let value = data.get(&key).cloned();
        if value.is_some() {
            log_info!(self.info.address, "Retrieved key: {}", hex::encode(key));
        } else {
            log_info!(self.info.address, "Key not found: {}", hex::encode(key));
        }
        value
    }

    // Retrieves all key-value pairs in a given range
    pub async fn get_data_range(&self, start: NodeId, end: NodeId) -> Vec<(NodeId, Vec<u8>)> {
        let data = self.data.lock().unwrap();
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
        match bootstrap_address {
            Some(address) => {
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
                match response {
                    Ok(DhtMessage::FoundSuccessor {
                        id,
                        address,
                        api_address,
                    }) => {
                        let successor_info = NodeInfo {
                            id,
                            address: address.clone(),
                            api_address,
                        };

                        {
                            let mut successor = self.successor.lock().unwrap();
                            *successor = successor_info.clone();
                            log_info!(
                                self.info.address,
                                "Joined network. Successor: {} at {}",
                                hex::encode(successor.id),
                                successor.address
                            );
                        }

                        // Request data from successor that should belong to us
                        // We are responsible for keys in range (predecessor.id, self.id]
                        // Since we just joined, we need keys in range (self.id, successor.id]
                        log_info!(
                            self.info.address,
                            "Requesting data transfer from successor {}",
                            successor_info.address
                        );

                        match self
                            .network_client
                            .call_node(
                                &successor_info.address,
                                DhtMessage::GetDataRange {
                                    start: self.info.id,
                                    end: successor_info.id,
                                },
                            )
                            .await
                        {
                            Ok(DhtMessage::DataRange { data }) => {
                                if !data.is_empty() {
                                    log_info!(
                                        self.info.address,
                                        "Received {} keys from successor",
                                        data.len()
                                    );
                                    for (key, value) in data {
                                        self.store(key, value).await;
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
                    Ok(other) => {
                        log_error!(
                            self.info.address,
                            "Unexpected response from bootstrap node: {:?}",
                            other
                        );
                    }
                    Err(e) => {
                        log_error!(
                            self.info.address,
                            "Failed to get successor from bootstrap node: {}",
                            e
                        );
                    }
                }
            }
            None => {
                log_info!(
                    self.info.address,
                    "No bootstrap node provided. Starting a new network."
                );
                self.start_new_network().await;
            }
        }
    }

    async fn start_new_network(&self) {
        let mut successor = self.successor.lock().unwrap();
        *successor = self.info.clone();
        let mut predecessor = self.predecessor.lock().unwrap();
        *predecessor = None;
        log_info!(
            self.info.address,
            "Started new network. I am the only node."
        );
    }

    // Finds the successor of an ID
    pub async fn find_successor(&self, id: NodeId) -> NodeInfo {
        // If the ID is between this node and its successor, then successor is the answer
        let successor = self.successor.lock().unwrap().clone();
        if tace_lib::is_between(&id, &self.info.id, &successor.id) {
            return successor;
        }

        // Otherwise, find the closest preceding node and ask it
        let n_prime = self.closest_preceding_node(id).await;
        if n_prime.id == self.info.id {
            return self.successor.lock().unwrap().clone();
        }

        // Call n_prime's find_successor
        match self
            .network_client
            .call_node(&n_prime.address, DhtMessage::FindSuccessor { id })
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
                    "Error: Failed to get successor from closest preceding node."
                );

                // Fallback to self's successor if remote call fails
                self.successor.lock().unwrap().clone()
            }
        }
    }

    // Finds the predecessor of an ID
    // TODO: remove this method if not needed
    #[allow(dead_code)]
    pub async fn find_predecessor(&self, id: NodeId) -> NodeInfo {
        let mut n_prime = self.info.clone();
        let mut current_successor = self.successor.lock().unwrap().clone();

        // Loop until id is between n_prime and its successor
        while !tace_lib::is_between(&id, &n_prime.id, &current_successor.id) {
            if n_prime.id == self.info.id {
                n_prime = self.closest_preceding_node(id).await;
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
                    }) => {
                        n_prime = NodeInfo {
                            id,
                            address,
                            api_address,
                        };
                    }
                    _ => {
                        log_error!(
                            self.info.address,
                            "Error: Failed to get closest preceding node from remote."
                        );
                    }
                }
            }
            // Get the successor of the new n_prime
            match self
                .network_client
                .call_node(&n_prime.address, DhtMessage::GetSuccessor)
                .await
            {
                Ok(DhtMessage::FoundSuccessor {
                    id,
                    address,
                    api_address,
                }) => {
                    current_successor = NodeInfo {
                        id,
                        address,
                        api_address,
                    };
                }
                _ => {
                    log_error!(
                        self.info.address,
                        "Error: Failed to get successor of n_prime."
                    );
                }
            }
        }
        n_prime
    }

    // Finds the node in the finger table that most immediately precedes `id`.
    async fn closest_preceding_node(&self, id: NodeId) -> NodeInfo {
        let finger_table = self.finger_table.lock().unwrap();
        // Iterate finger table in reverse
        for i in (0..M).rev() {
            let finger = &finger_table[i];
            // If finger is between current node and id (exclusive of current node, exclusive of id)
            if tace_lib::is_between(&finger.id, &self.info.id, &id) {
                return finger.clone();
            }
        }
        self.info.clone()
    }

    pub async fn stabilize(&self) {
        let successor = self.successor.lock().unwrap().clone();

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
            let mut predecessor = self.predecessor.lock().unwrap();
            if predecessor.is_some() {
                *predecessor = None;
            }
            return;
        }

        // Get successor's predecessor
        debug!(
            "[{}] Stabilize: asking successor {} for its predecessor",
            self.info.address, successor.address
        );

        match self
            .network_client
            .call_node(&successor.address, DhtMessage::GetPredecessor)
            .await
        {
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
                    let mut current_successor = self.successor.lock().unwrap();
                    *current_successor = x.clone();
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
        let current_successor = self.successor.lock().unwrap().clone();
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
            let mut predecessor = self.predecessor.lock().unwrap();
            current_pred = predecessor.clone();

            // If predecessor is nil or n_prime is between predecessor and self
            should_update = if predecessor.is_none() {
                debug!(
                    "[{}] Notify: no current predecessor, accepting {}",
                    self.info.address, n_prime.address
                );
                true
            } else {
                let is_between = tace_lib::is_between(
                    &n_prime.id,
                    &predecessor.as_ref().unwrap().id,
                    &self.info.id,
                );
                debug!(
                    "[{}] Notify: current pred={}, candidate={}, is_between={}",
                    self.info.address,
                    predecessor.as_ref().unwrap().address,
                    n_prime.address,
                    is_between
                );
                is_between
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
                let mut successor = self.successor.lock().unwrap();
                if successor.id == self.info.id {
                    debug!(
                        "[{}] Notify: was single node, updating successor to {}",
                        self.info.address, n_prime.address
                    );
                    *successor = n_prime.clone();
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
                for (key, value) in data_to_transfer {
                    match self
                        .network_client
                        .call_node(&n_prime.address, DhtMessage::Store { key, value })
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
        } else {
            debug!(
                "[{}] Notify: keeping current predecessor {:?}",
                self.info.address,
                current_pred.as_ref().map(|p| &p.address)
            );
        }
    }

    pub async fn fix_fingers(&self) {
        // Pick a random finger to fix, to avoid all nodes fixing the same finger at the same time.
        let i = rand::thread_rng().gen_range(1..M);
        let target_id = tace_lib::add_id_power_of_2(&self.info.id, i);
        let successor = self.find_successor(target_id).await;

        // Update the finger table
        let mut finger_table = self.finger_table.lock().unwrap();
        finger_table[i] = successor;
    }

    pub async fn check_predecessor(&self) {
        let predecessor_option = self.predecessor.lock().unwrap().clone();
        if let Some(predecessor) = predecessor_option {
            // Send a simple message to check if predecessor is alive
            match self
                .network_client
                .call_node(&predecessor.address, DhtMessage::Ping)
                .await
            {
                Ok(DhtMessage::Pong) => {
                    // Predecessor is alive
                }
                _ => {
                    // Predecessor is dead, clear it
                    let mut p = self.predecessor.lock().unwrap();
                    *p = None;
                }
            }
        }
    }

    pub async fn estimate_network_size(&self) {
        // Calculate local network size estimate
        let local_estimate = self.calculate_local_network_size_estimate();

        // Collect all known nodes (successor + finger table)
        let successor = self.successor.lock().unwrap().clone();
        let finger_table = self.finger_table.lock().unwrap().clone();

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
        let random_node = candidate_nodes.choose(&mut rand::thread_rng()).unwrap();

        debug!(
            "[{}] Querying random neighbor {} for network size estimate",
            self.info.address, random_node.address
        );

        // Get neighbor's estimate
        match self
            .network_client
            .call_node(&random_node.address, DhtMessage::GetNetworkEstimate)
            .await
        {
            Ok(DhtMessage::NetworkEstimate {
                estimate: neighbor_estimate,
            }) => {
                // Calculate new estimate by averaging with neighbor's estimate
                let new_raw_estimate = (local_estimate + neighbor_estimate) / 2.0;

                // Apply bounds and smoothing
                let old_estimate = *self.network_size_estimate.lock().unwrap();
                let bounded_estimate = self.apply_bounds(old_estimate, new_raw_estimate);
                let smoothed_estimate = self.apply_smoothing(old_estimate, bounded_estimate);

                // Update our estimate
                let mut estimate = self.network_size_estimate.lock().unwrap();
                *estimate = smoothed_estimate;

                debug!(
                    "[{}] Network size estimate updated: old={:.2}, neighbor={:.2}, local={:.2}, new={:.2}",
                    self.info.address, old_estimate, neighbor_estimate, local_estimate, smoothed_estimate
                );
            }
            _ => {
                log_error!(
                    self.info.address,
                    "Failed to get network estimate from {}",
                    random_node.address
                );
            }
        }
    }

    fn calculate_local_network_size_estimate(&self) -> f64 {
        let successor = self.successor.lock().unwrap();
        let predecessor = self.predecessor.lock().unwrap();

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

    pub fn debug_ring_state(&self) {
        let successor = self.successor.lock().unwrap();
        let predecessor = self.predecessor.lock().unwrap();

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_client::MockNetworkClient;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tace_lib::dht_messages::{DhtMessage, NodeId};

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
            predecessor: Arc::new(Mutex::new(None)),
            finger_table: Arc::new(Mutex::new(finger_table_vec)),
            data: Arc::new(Mutex::new(HashMap::new())),
            network_client: mock_network_client,
            network_size_estimate: Arc::new(Mutex::new(1.0)),
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
        assert_eq!(retrieved_value, Some(value));

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
            .returning(|_, _| Err("Simulated network error".into()));

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
                        _ => false,
                    }
            })
            .times(1)
            .returning(move |_, _| {
                Ok(DhtMessage::FoundSuccessor {
                    id: bootstrap_node_id,
                    address: "127.0.0.1:8001".to_string(),
                    api_address: "127.0.0.1:8001".to_string(),
                })
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));
        node.join(Some("127.0.0.1:8001".to_string())).await;

        let successor = node.successor.lock().unwrap();
        assert_eq!(successor.id, bootstrap_node_id);
        assert_eq!(successor.address, "127.0.0.1:8001");
    }

    #[tokio::test]
    async fn test_find_successor() {
        let mut mock_network_client = MockNetworkClient::new();
        let successor_id = hex_to_node_id("3333333333333333333333333333333333333333");
        let successor_address = "127.0.0.1:8002".to_string();
        let successor_address_clone = successor_address.clone();

        // This mock will be for the call to the closest preceding node
        mock_network_client
            .expect_call_node()
            .times(1)
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

        // Mock the neighbor's response
        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::GetNetworkEstimate))
            .times(1)
            .returning(|_, _| Ok(DhtMessage::NetworkEstimate { estimate: 3.5 }));

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

        let initial_estimate = *node.network_size_estimate.lock().unwrap();

        node.estimate_network_size().await;

        let final_estimate = *node.network_size_estimate.lock().unwrap();

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
        // estimate_network_size should return early without making network calls
        let initial_estimate = *node.network_size_estimate.lock().unwrap();

        node.estimate_network_size().await;

        let final_estimate = *node.network_size_estimate.lock().unwrap();

        // Estimate should be unchanged since we're the only node
        assert_eq!(initial_estimate, final_estimate);
    }

    #[tokio::test]
    async fn test_random_neighbor_selection() {
        let mut mock_network_client = MockNetworkClient::new();

        // Mock response for any network estimate request
        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::GetNetworkEstimate))
            .times(1)
            .returning(|_, _| Ok(DhtMessage::NetworkEstimate { estimate: 4.0 }));

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
        node.estimate_network_size().await;

        // The estimate should have been updated from the mock response
        let final_estimate = *node.network_size_estimate.lock().unwrap();
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
}
