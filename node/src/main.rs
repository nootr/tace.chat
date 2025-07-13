use log::{debug, error, info};
use num_bigint::BigUint;
use num_traits::ToPrimitive;
use rand::seq::SliceRandom;
use rand::Rng;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};
use tace_lib::dht_messages::{DhtMessage, NodeId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

mod api;
mod network_client;

pub use api::run;
pub use network_client::{NetworkClient, RealNetworkClient};

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
    network_client: Arc<T>,
    network_size_estimate: Arc<Mutex<f64>>,
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
                        let mut successor = self.successor.lock().unwrap();
                        *successor = NodeInfo {
                            id,
                            address,
                            api_address,
                        };
                        log_info!(
                            self.info.address,
                            "Joined network. Successor: {} at {}",
                            hex::encode(successor.id),
                            successor.address
                        );
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

        let mut predecessor = self.predecessor.lock().unwrap();
        let current_pred = predecessor.clone();

        // If predecessor is nil or n_prime is between predecessor and self
        let should_update = if predecessor.is_none() {
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
                *successor = n_prime;
            }
        } else {
            debug!(
                "[{}] Notify: keeping current predecessor {}",
                self.info.address,
                predecessor.as_ref().unwrap().address
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

#[tokio::main]
async fn main() {
    // Initialize logger
    env_logger::init();

    let advertise_address =
        env::var("NODE_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8000".to_string());
    let (advertise_host, advertise_port) = advertise_address
        .split_once(':')
        .expect("NODE_ADDRESS must be in the format HOST:PORT");
    let bind_address = env::var("BIND_ADDRESS").unwrap_or_else(|_| {
        format!("0.0.0.0:{}", advertise_port)
    });
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

#[cfg(test)]
mod tests;
