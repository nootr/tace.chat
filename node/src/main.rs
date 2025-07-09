use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use wisp_lib::dht_messages::{DhtMessage, NodeId};

const M: usize = 160; // Number of bits in Chord ID space (SHA-1 produces 160-bit hash)

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: NodeId,
    pub address: String,
}

#[derive(Debug)] // Keep Debug, remove Clone
pub struct ChordNode {
    pub info: NodeInfo,
    pub successor: Arc<Mutex<NodeInfo>>,
    pub predecessor: Arc<Mutex<Option<NodeInfo>>>,
    pub finger_table: Arc<Mutex<Vec<NodeInfo>>>,
    pub data: Arc<Mutex<HashMap<NodeId, Vec<u8>>>>,
}

impl Clone for ChordNode {
    fn clone(&self) -> Self {
        ChordNode {
            info: self.info.clone(),
            successor: self.successor.clone(),
            predecessor: self.predecessor.clone(),
            finger_table: self.finger_table.clone(),
            data: self.data.clone(), // This clones the Arc, not the HashMap
        }
    }
}

impl ChordNode {
    pub async fn new(address: String) -> Self {
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
        }
    }

    fn generate_node_id(address: &str) -> NodeId {
        let mut hasher = Sha1::new();
        hasher.update(address.as_bytes());
        hasher.finalize().into()
    }

    pub async fn start(self) {
        println!(
            "Chord Node {} starting at {}",
            hex::encode(self.info.id),
            self.info.address
        );
        let listener = TcpListener::bind(&self.info.address).await.unwrap();

        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let node = self.clone();
            tokio::spawn(async move {
                node.handle_connection(socket).await;
            });
        }
    }

    async fn handle_connection(&self, mut socket: TcpStream) {
        let mut buffer = Vec::new();
        // Read the entire message
        if let Err(e) = socket.read_to_end(&mut buffer).await {
            eprintln!("Failed to read from socket: {}", e);
            return;
        }

        match bincode::deserialize::<DhtMessage>(&buffer) {
            Ok(message) => {
                println!("Received message: {:?}\n", message);
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
                        // No response for store for now, or a simple ACK
                        return;
                    }
                    DhtMessage::Retrieve { key } => {
                        let value = self.retrieve(key).await;
                        DhtMessage::Retrieved { key, value }
                    }
                    _ => {
                        // Handle other messages or return an error/unsupported message
                        eprintln!("Unsupported message received: {:?}", message);
                        return;
                    }
                };
                let encoded_response = bincode::serialize(&response).unwrap();
                if let Err(e) = socket.write_all(&encoded_response).await {
                    eprintln!("Failed to write response to socket: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Failed to deserialize message: {}", e);
            }
        }
    }

    // Stores a key-value pair in the DHT
    pub async fn store(&self, key: NodeId, value: Vec<u8>) {
        let mut data = self.data.lock().unwrap();
        data.insert(key, value);
        println!("Stored key: {}", hex::encode(key));
    }

    // Retrieves a value by key from the DHT
    pub async fn retrieve(&self, key: NodeId) -> Option<Vec<u8>> {
        let data = self.data.lock().unwrap();
        let value = data.get(&key).cloned();
        if value.is_some() {
            println!("Retrieved key: {}", hex::encode(key));
        } else {
            println!("Key not found: {}", hex::encode(key));
        }
        value
    }

    async fn call_node(
        address: &str,
        message: DhtMessage,
    ) -> Result<DhtMessage, Box<dyn std::error::Error>> {
        let mut stream = TcpStream::connect(address).await?;
        let encoded = bincode::serialize(&message)?;
        stream.write_all(&encoded).await?;
        stream.shutdown().await?;

        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).await?;
        let response = bincode::deserialize(&buffer)?;
        Ok(response)
    }

    pub async fn join(&self, bootstrap_address: Option<String>) {
        match bootstrap_address {
            Some(address) => {
                println!("Attempting to join network via bootstrap node: {}", address);
                // Find successor from bootstrap node
                let response =
                    Self::call_node(&address, DhtMessage::FindSuccessor { id: self.info.id }).await;
                match response {
                    Ok(DhtMessage::FoundSuccessor { id, address }) => {
                        let mut successor = self.successor.lock().unwrap();
                        *successor = NodeInfo { id, address };
                        println!(
                            "Joined network. Successor: {} at {}",
                            hex::encode(&successor.id),
                            successor.address
                        );
                    }
                    _ => {
                        eprintln!("Failed to get successor from bootstrap node.");
                        // Fallback to starting new network if join fails
                        self.start_new_network().await;
                    }
                }
            }
            None => {
                println!("No bootstrap node provided. Starting a new network.");
                self.start_new_network().await;
            }
        }
    }

    async fn start_new_network(&self) {
        let mut successor = self.successor.lock().unwrap();
        *successor = self.info.clone();
        let mut predecessor = self.predecessor.lock().unwrap();
        *predecessor = None;
        println!("Started new network. I am the only node.");
    }

    // Finds the successor of an ID
    pub async fn find_successor(&self, id: NodeId) -> NodeInfo {
        // If the ID is between this node and its successor, then successor is the answer
        let successor = self.successor.lock().unwrap().clone();
        if wisp_lib::is_between(&id, &self.info.id, &successor.id) {
            return successor;
        }

        // Otherwise, find the closest preceding node and ask it
        let n_prime = self.closest_preceding_node(id).await;
        if n_prime.id == self.info.id {
            return self.successor.lock().unwrap().clone();
        }

        // Call n_prime's find_successor
        match Self::call_node(&n_prime.address, DhtMessage::FindSuccessor { id }).await {
            Ok(DhtMessage::FoundSuccessor { id, address }) => NodeInfo { id, address },
            _ => {
                eprintln!("Error: Failed to get successor from closest preceding node.");
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
        while !wisp_lib::is_between(&id, &n_prime.id, &current_successor.id) {
            if n_prime.id == self.info.id {
                n_prime = self.closest_preceding_node(id).await;
            } else {
                match Self::call_node(&n_prime.address, DhtMessage::ClosestPrecedingNode { id })
                    .await
                {
                    Ok(DhtMessage::FoundClosestPrecedingNode { id, address }) => {
                        n_prime = NodeInfo { id, address };
                    }
                    _ => {
                        eprintln!("Error: Failed to get closest preceding node from remote.");
                        // Fallback to self if remote call fails
                        return self.info.clone();
                    }
                }
            }
            // Get the successor of the new n_prime
            match Self::call_node(&n_prime.address, DhtMessage::GetSuccessor).await {
                Ok(DhtMessage::FoundSuccessor { id, address }) => {
                    current_successor = NodeInfo { id, address };
                }
                _ => {
                    eprintln!("Error: Failed to get successor of n_prime.");
                    // Fallback to self's successor if remote call fails
                    current_successor = self.successor.lock().unwrap().clone();
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
            if wisp_lib::is_between(&finger.id, &self.info.id, &id) {
                return finger.clone();
            }
        }
        self.info.clone()
    }
}

#[tokio::main]
async fn main() {
    println!("Hello from wisp_node!");
    println!("{}", wisp_lib::lib_test());

    let address = env::var("NODE_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8000".to_string());
    let bootstrap_address = env::var("BOOTSTRAP_ADDRESS").ok();

    let node = ChordNode::new(address).await;
    node.join(bootstrap_address).await;
    node.start().await;
}

#[cfg(test)]
mod tests;
