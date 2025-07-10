use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};
use tace_lib::dht_messages::{DhtMessage, NodeId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

mod network_client;
use network_client::{NetworkClient, RealNetworkClient};

macro_rules! log_info {
    ($address:expr, $($arg:tt)*) => ({
        println!("[{}] {}", $address, format_args!($($arg)*));
    })
}

macro_rules! log_error {
    ($address:expr, $($arg:tt)*) => ({
        eprintln!("[{}] {}", $address, format_args!($($arg)*));
    })
}

const M: usize = 160; // Number of bits in Chord ID space (SHA-1 produces 160-bit hash)

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: NodeId,
    pub address: String,
}

#[derive(Debug)] // Keep Debug, remove Clone
pub struct ChordNode<T: NetworkClient> {
    pub info: NodeInfo,
    pub successor: Arc<Mutex<NodeInfo>>,
    pub predecessor: Arc<Mutex<Option<NodeInfo>>>,
    pub finger_table: Arc<Mutex<Vec<NodeInfo>>>,
    pub data: Arc<Mutex<HashMap<NodeId, Vec<u8>>>>,
    network_client: Arc<T>,
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
        }
    }
}

impl<T: NetworkClient> ChordNode<T> {
    pub async fn new(address: String, network_client: Arc<T>) -> Self {
        let id = Self::generate_node_id(&address);
        let info = NodeInfo {
            id,
            address: address.clone(),
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
        }
    }

    #[cfg(test)]
    pub fn new_for_test(address: String, network_client: Arc<T>) -> Self {
        let id = Self::generate_node_id(&address);
        let info = NodeInfo {
            id,
            address: address.clone(),
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
        }
    }

    fn generate_node_id(address: &str) -> NodeId {
        let mut hasher = Sha1::new();
        hasher.update(address.as_bytes());
        hasher.finalize().into()
    }

    pub async fn start(&self) {
        log_info!(
            self.info.address,
            "Chord Node {} starting at {}",
            hex::encode(self.info.id),
            self.info.address
        );
        let listener = TcpListener::bind(&self.info.address)
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
                        }
                    }
                    DhtMessage::GetSuccessor => {
                        let successor = self.successor.lock().unwrap().clone();
                        DhtMessage::FoundSuccessor {
                            id: successor.id,
                            address: successor.address,
                        }
                    }
                    DhtMessage::ClosestPrecedingNode { id } => {
                        let cpn = self.closest_preceding_node(id).await;
                        DhtMessage::FoundClosestPrecedingNode {
                            id: cpn.id,
                            address: cpn.address,
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
                        }
                    }
                    DhtMessage::Notify { id, address } => {
                        self.notify(NodeInfo { id, address }).await;
                        DhtMessage::Pong // Send a Pong response
                    }
                    DhtMessage::Ping => DhtMessage::Pong,
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
                    Ok(DhtMessage::FoundSuccessor { id, address }) => {
                        let mut successor = self.successor.lock().unwrap();
                        *successor = NodeInfo { id, address };
                        log_info!(
                            self.info.address,
                            "Joined network. Successor: {} at {}",
                            hex::encode(successor.id),
                            successor.address
                        );
                    }
                    _ => {
                        log_error!(
                            self.info.address,
                            "Failed to get successor from bootstrap node."
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
            Ok(DhtMessage::FoundSuccessor { id, address }) => NodeInfo { id, address },
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
                    Ok(DhtMessage::FoundClosestPrecedingNode { id, address }) => {
                        n_prime = NodeInfo { id, address };
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
                Ok(DhtMessage::FoundSuccessor { id, address }) => {
                    current_successor = NodeInfo { id, address };
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

        // If this node is its own successor, it's the only node in the network.
        // In this case, its predecessor should be None.
        if self.info.id == successor.id {
            let mut predecessor = self.predecessor.lock().unwrap();
            if predecessor.is_some() {
                *predecessor = None;
            }
            return;
        }

        // Get successor's predecessor
        match self
            .network_client
            .call_node(&successor.address, DhtMessage::GetPredecessor)
            .await
        {
            Ok(DhtMessage::Predecessor {
                id: Some(x_id),
                address: Some(x_address),
            }) => {
                let x = NodeInfo {
                    id: x_id,
                    address: x_address,
                };
                // If x is between self and successor, then x is the new successor
                if tace_lib::is_between(&x.id, &self.info.id, &successor.id) {
                    let mut current_successor = self.successor.lock().unwrap();
                    *current_successor = x.clone();
                }
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
        if let Err(e) = self
            .network_client
            .call_node(
                &current_successor.address,
                DhtMessage::Notify {
                    id: self.info.id,
                    address: self.info.address.clone(),
                },
            )
            .await
        {
            log_error!(self.info.address, "Error notifying successor: {}", e);
        }
    }

    pub async fn notify(&self, n_prime: NodeInfo) {
        let mut predecessor = self.predecessor.lock().unwrap();
        // If predecessor is nil or n_prime is between predecessor and self
        if predecessor.is_none()
            || tace_lib::is_between(
                &n_prime.id,
                &predecessor.as_ref().unwrap().id,
                &self.info.id,
            )
        {
            *predecessor = Some(n_prime);
        }
    }

    pub async fn fix_fingers(&self) {
        // Just acquire and release lock to test if that's the issue
        {
            let _finger_table = self.finger_table.lock().unwrap();
        } // Lock released

        // Now call find_successor
        let target_id = tace_lib::add_id_power_of_2(&self.info.id, 0);
        let _successor = self.find_successor(target_id).await;
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
}

#[tokio::main]
async fn main() {
    let address = env::var("NODE_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8000".to_string());
    let bootstrap_address = env::var("BOOTSTRAP_ADDRESS").ok();

    let node = Arc::new(ChordNode::new(address, Arc::new(RealNetworkClient)).await); // Wrap ChordNode in Arc

    node.join(bootstrap_address).await;

    let node_clone_for_tasks = node.clone();

    tokio::spawn(async move {
        let node = node_clone_for_tasks;

        loop {
            node.stabilize().await;
            node.fix_fingers().await;
            node.check_predecessor().await;
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }
    });

    node.start().await;
}

#[cfg(test)]
mod tests;
