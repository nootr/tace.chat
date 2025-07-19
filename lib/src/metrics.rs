use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Represents the health and usage metrics of the network.
///
/// These metrics are designed to be aggregated across nodes without
/// tracking client-specific data.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct NetworkMetrics {
    /// The average number of nodes joining and leaving the network.
    pub node_churn_rate: f64,

    /// The average fullness of routing tables across the network.
    pub routing_table_health: f64,

    /// The percentage of successful network operations (e.g., STORE, FIND_NODE).
    pub operation_success_rate: f64,

    /// The average latency for network operations.
    #[serde(with = "humantime_serde")]
    pub operation_latency: Duration,

    /// The ratio of control messages to data messages.
    pub message_type_ratio: f64,

    /// The number of key-value pairs stored locally on this node.
    pub local_key_count: u64,

    /// The total estimated number of key-value pairs stored across the entire network.
    pub total_network_keys_estimate: f64,

    /// The estimated number of nodes in the network.
    pub network_size_estimate: f64,

    /// Load balancing metrics
    pub cpu_usage_percent: f32,
    pub memory_usage_percent: f32,
    pub disk_usage_percent: f32,
    pub active_connections: u32,
    pub request_rate_per_second: f32,
}

impl Default for NetworkMetrics {
    fn default() -> Self {
        Self {
            node_churn_rate: 0.0,
            routing_table_health: 0.0,
            operation_success_rate: 1.0, // Assume 100% success initially
            operation_latency: Duration::from_millis(0),
            message_type_ratio: 0.0,
            local_key_count: 0,
            total_network_keys_estimate: 0.0,
            network_size_estimate: 1.0,
            cpu_usage_percent: 0.0,
            memory_usage_percent: 0.0,
            disk_usage_percent: 0.0,
            active_connections: 0,
            request_rate_per_second: 0.0,
        }
    }
}
