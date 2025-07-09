use serde::{Deserialize, Serialize};

// Placeholder for a Node ID (e.g., SHA1 hash)
pub type NodeId = [u8; 20]; // 160-bit ID for Chord

#[derive(Debug, Serialize, Deserialize)]
pub enum DhtMessage {
    // Message to find the successor of a given ID
    FindSuccessor {
        id: NodeId,
    },
    // Response to FindSuccessor
    FoundSuccessor {
        id: NodeId,
        address: String,
    },
    // Notify a node that we believe we are its predecessor
    Notify {
        id: NodeId,
        address: String,
    },
    // Request for a node's predecessor
    GetPredecessor,
    // Response with a node's predecessor
    Predecessor {
        id: Option<NodeId>,
        address: Option<String>,
    },
    // Store a key-value pair
    Store {
        key: NodeId,
        value: Vec<u8>,
    },
    // Retrieve a value by key
    Retrieve {
        key: NodeId,
    },
    // Response with a retrieved value
    Retrieved {
        key: NodeId,
        value: Option<Vec<u8>>,
    },
    // Request for the closest preceding node
    ClosestPrecedingNode {
        id: NodeId,
    },
    // Response with the closest preceding node
    FoundClosestPrecedingNode {
        id: NodeId,
        address: String,
    },
    // Request for a node's successor
    GetSuccessor,
}
