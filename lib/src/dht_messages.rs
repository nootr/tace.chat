use crate::metrics::NetworkMetrics;
use serde::{Deserialize, Serialize};

// Placeholder for a Node ID (e.g., SHA1 hash)
pub type NodeId = [u8; 20]; // 160-bit ID for Chord

#[derive(Debug, Serialize, Deserialize)]
pub enum DhtMessage {
    // Error message
    Error {
        message: String,
    },
    // Message to find the successor of a given ID
    FindSuccessor {
        id: NodeId,
    },
    // Response to FindSuccessor
    FoundSuccessor {
        id: NodeId,
        address: String,
        api_address: String,
    },
    // Message to find the predecessor of a given ID
    FindPredecessor {
        id: NodeId,
    },
    // Response to FindPredecessor
    FoundPredecessor {
        id: NodeId,
        address: String,
        api_address: String,
    },
    // Notify a node that we believe we are its predecessor
    Notify {
        id: NodeId,
        address: String,
        api_address: String,
    },
    // Request for a node's predecessor
    GetPredecessor,
    // Response with a node's predecessor
    Predecessor {
        id: Option<NodeId>,
        address: Option<String>,
        api_address: Option<String>,
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
        value: Option<Vec<Vec<u8>>>,
    },
    // Request for the closest preceding node
    ClosestPrecedingNode {
        id: NodeId,
    },
    // Response with the closest preceding node
    FoundClosestPrecedingNode {
        id: NodeId,
        address: String,
        api_address: String,
    },
    // Request for a node's successor
    GetSuccessor,
    Ping,
    Pong,
    // Request data in a key range (used when a node joins and needs data)
    GetDataRange {
        start: NodeId,
        end: NodeId,
    },
    // Response with data in the requested range
    DataRange {
        data: Vec<(NodeId, Vec<Vec<u8>>)>,
    },
    // Metrics sharing
    ShareMetrics {
        metrics: NetworkMetrics,
    },
    MetricsShared {
        metrics: NetworkMetrics,
    },
}
