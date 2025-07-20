use crate::integration::TestHarness;
use std::collections::{HashMap, HashSet};

/// Network invariants that should always hold in a properly functioning DHT
/// These represent the core correctness properties of the Chord protocol
pub struct NetworkInvariants;

#[derive(Debug)]
pub struct InvariantViolation {
    pub name: String,
    pub description: String,
    pub affected_nodes: Vec<String>,
}

impl NetworkInvariants {
    /// Check all invariants and return any violations
    pub async fn check_all(harness: &TestHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();

        violations.extend(Self::check_ring_connectivity(harness).await);
        violations.extend(Self::check_successor_consistency(harness).await);
        violations.extend(Self::check_predecessor_consistency(harness).await);
        violations.extend(Self::check_data_availability(harness).await);
        violations.extend(Self::check_data_consistency(harness).await);
        violations.extend(Self::check_load_balance(harness).await);
        violations.extend(Self::check_fault_tolerance(harness).await);

        violations
    }

    /// Invariant 1: Ring Connectivity
    /// Every node should be reachable from every other node by following successors
    pub async fn check_ring_connectivity(harness: &TestHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();
        let node_addresses = harness.get_all_node_addresses().await;

        if node_addresses.is_empty() {
            return violations;
        }

        for start_address in &node_addresses {
            let mut visited = HashSet::new();
            let mut current_address = start_address.clone();
            let mut steps = 0;
            let max_steps = node_addresses.len() * 2; // Safety limit

            loop {
                if visited.contains(&current_address) || steps >= max_steps {
                    break;
                }

                visited.insert(current_address.clone());
                steps += 1;

                // Get successor of current node
                if let Some(node) = harness.get_node(&current_address).await {
                    if let Some((_, successor_addr, _)) = node.get_successor().await {
                        current_address = successor_addr;
                    } else {
                        violations.push(InvariantViolation {
                            name: "Ring Connectivity".to_string(),
                            description: format!("Node {} has no successor", current_address),
                            affected_nodes: vec![current_address.clone()],
                        });
                        break;
                    }
                } else {
                    break;
                }
            }

            // Check if we visited all nodes
            if visited.len() != node_addresses.len() {
                violations.push(InvariantViolation {
                    name: "Ring Connectivity".to_string(),
                    description: format!("Not all nodes reachable from {}", start_address),
                    affected_nodes: node_addresses
                        .iter()
                        .filter(|addr| !visited.contains(*addr))
                        .cloned()
                        .collect(),
                });
            }
        }

        violations
    }

    /// Invariant 2: Successor Consistency
    /// For each node n, successor(n).predecessor should be n (or at least point to n)
    pub async fn check_successor_consistency(harness: &TestHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();
        let node_addresses = harness.get_all_node_addresses().await;

        for address in &node_addresses {
            if let Some(node) = harness.get_node(address).await {
                if let Some((_, successor_addr, _)) = node.get_successor().await {
                    if let Some(successor_node) = harness.get_node(&successor_addr).await {
                        if let Some((_, pred_addr, _)) = successor_node.get_predecessor().await {
                            if pred_addr != *address {
                                violations.push(InvariantViolation {
                                    name: "Successor Consistency".to_string(),
                                    description: format!(
                                        "Node {}'s successor {} has predecessor {} instead of {}",
                                        address, successor_addr, pred_addr, address
                                    ),
                                    affected_nodes: vec![address.clone(), successor_addr],
                                });
                            }
                        }
                    }
                }
            }
        }

        violations
    }

    /// Invariant 3: Predecessor Consistency
    /// For each node n, predecessor(n).successor should be n
    pub async fn check_predecessor_consistency(harness: &TestHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();
        let node_addresses = harness.get_all_node_addresses().await;

        for address in &node_addresses {
            if let Some(node) = harness.get_node(address).await {
                if let Some((_, predecessor_addr, _)) = node.get_predecessor().await {
                    if let Some(predecessor_node) = harness.get_node(&predecessor_addr).await {
                        if let Some((_, succ_addr, _)) = predecessor_node.get_successor().await {
                            if succ_addr != *address {
                                violations.push(InvariantViolation {
                                    name: "Predecessor Consistency".to_string(),
                                    description: format!(
                                        "Node {}'s predecessor {} has successor {} instead of {}",
                                        address, predecessor_addr, succ_addr, address
                                    ),
                                    affected_nodes: vec![address.clone(), predecessor_addr],
                                });
                            }
                        }
                    }
                }
            }
        }

        violations
    }

    /// Invariant 4: Data Availability
    /// All stored data should be retrievable from the network
    pub async fn check_data_availability(harness: &TestHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();
        let stored_data = harness.get_stored_data().await;
        let node_addresses = harness.get_all_node_addresses().await;

        if stored_data.is_empty() || node_addresses.is_empty() {
            return violations; // Nothing to check
        }

        // Try to retrieve each stored key from at least one node
        for (key, expected_value) in &stored_data {
            let mut found = false;
            let mut failed_nodes = Vec::new();

            for node_address in &node_addresses {
                match harness.retrieve_data(node_address, *key).await {
                    Ok(Some(retrieved_values)) => {
                        // Check if any of the retrieved values match the expected value
                        if retrieved_values.iter().any(|v| v == expected_value) {
                            found = true;
                            break;
                        }
                    }
                    Ok(None) => {
                        failed_nodes.push(node_address.clone());
                    }
                    Err(_) => {
                        failed_nodes.push(node_address.clone());
                    }
                }
            }

            if !found {
                violations.push(InvariantViolation {
                    name: "Data Availability".to_string(),
                    description: format!("Key {:?} is not retrievable from any node", key),
                    affected_nodes: failed_nodes,
                });
            }
        }

        violations
    }

    /// Invariant 5: Data Consistency
    /// The same key should return the same value from any node
    pub async fn check_data_consistency(harness: &TestHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();
        let stored_data = harness.get_stored_data().await;
        let node_addresses = harness.get_all_node_addresses().await;

        if stored_data.is_empty() || node_addresses.len() < 2 {
            return violations; // Nothing to check or only one node
        }

        // For each stored key, retrieve from multiple nodes and compare
        for (key, expected_value) in &stored_data {
            let mut retrieved_values: HashMap<String, Vec<Vec<u8>>> = HashMap::new();

            // Collect retrieval results from all nodes
            for node_address in &node_addresses {
                match harness.retrieve_data(node_address, *key).await {
                    Ok(Some(values)) => {
                        retrieved_values.insert(node_address.clone(), values);
                    }
                    Ok(None) => {
                        // Key not found at this node - this might be normal due to DHT routing
                    }
                    Err(_) => {
                        // Retrieval error - this might be normal if node doesn't have the data
                    }
                }
            }

            // Check consistency among nodes that returned values
            if retrieved_values.len() > 1 {
                let mut baseline: Option<&Vec<Vec<u8>>> = None;
                let mut inconsistent_nodes = Vec::new();

                for (node_address, values) in &retrieved_values {
                    if let Some(baseline_values) = baseline {
                        // Compare with baseline
                        if !values
                            .iter()
                            .any(|v| baseline_values.iter().any(|b| b == v))
                        {
                            inconsistent_nodes.push(node_address.clone());
                        }
                    } else {
                        baseline = Some(values);
                    }
                }

                // Also check that at least one node returns the expected value
                let has_expected = retrieved_values
                    .values()
                    .any(|values| values.iter().any(|v| v == expected_value));

                if !inconsistent_nodes.is_empty() || !has_expected {
                    violations.push(InvariantViolation {
                        name: "Data Consistency".to_string(),
                        description: format!(
                            "Key {:?} returns inconsistent values across nodes{}",
                            key,
                            if !has_expected {
                                " and doesn't match expected value"
                            } else {
                                ""
                            }
                        ),
                        affected_nodes: inconsistent_nodes,
                    });
                }
            }
        }

        violations
    }

    /// Invariant 6: Load Balance
    /// Data should be reasonably distributed across nodes
    pub async fn check_load_balance(harness: &TestHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();
        let node_addresses = harness.get_all_node_addresses().await;

        if node_addresses.len() < 2 {
            return violations; // Can't check balance with fewer than 2 nodes
        }

        let mut load_map = HashMap::new();

        // Collect load metrics from all nodes
        for address in &node_addresses {
            if let Some(node) = harness.get_node(address).await {
                let metrics = node.get_metrics().await;
                load_map.insert(address.clone(), metrics.local_key_count);
            }
        }

        // Calculate load distribution
        let total_items: u64 = load_map.values().sum();
        let expected_per_node = total_items / node_addresses.len() as u64;
        let tolerance = expected_per_node / 2; // Allow 50% deviation

        for (address, load) in &load_map {
            let deviation = if *load > expected_per_node {
                *load - expected_per_node
            } else {
                expected_per_node - *load
            };

            if deviation > tolerance {
                violations.push(InvariantViolation {
                    name: "Load Balance".to_string(),
                    description: format!(
                        "Node {} has {} items, expected ~{} (deviation: {})",
                        address, load, expected_per_node, deviation
                    ),
                    affected_nodes: vec![address.clone()],
                });
            }
        }

        violations
    }

    /// Invariant 7: Fault Tolerance
    /// Network should remain functional after node failures
    pub async fn check_fault_tolerance(harness: &TestHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();
        let node_addresses = harness.get_all_node_addresses().await;

        // Check if any partition exists in the network
        if node_addresses.len() > 1 {
            let reachable_nodes = Self::find_reachable_nodes(harness, &node_addresses[0]).await;

            if reachable_nodes.len() != node_addresses.len() {
                let unreachable: Vec<String> = node_addresses
                    .iter()
                    .filter(|addr| !reachable_nodes.contains(*addr))
                    .cloned()
                    .collect();

                violations.push(InvariantViolation {
                    name: "Fault Tolerance".to_string(),
                    description: "Network partition detected".to_string(),
                    affected_nodes: unreachable,
                });
            }
        }

        violations
    }

    /// Helper function to find all nodes reachable from a starting node
    async fn find_reachable_nodes(harness: &TestHarness, start_address: &str) -> HashSet<String> {
        let mut reachable = HashSet::new();
        let mut to_visit = vec![start_address.to_string()];

        while let Some(address) = to_visit.pop() {
            if reachable.contains(&address) {
                continue;
            }

            reachable.insert(address.clone());

            if let Some(node) = harness.get_node(&address).await {
                // Add successor
                if let Some((_, successor_addr, _)) = node.get_successor().await {
                    if !reachable.contains(&successor_addr) {
                        to_visit.push(successor_addr);
                    }
                }

                // Add predecessor
                if let Some((_, predecessor_addr, _)) = node.get_predecessor().await {
                    if !reachable.contains(&predecessor_addr) {
                        to_visit.push(predecessor_addr);
                    }
                }
            }
        }

        reachable
    }

    /// Check a specific invariant by name
    pub async fn check_invariant(
        harness: &TestHarness,
        invariant_name: &str,
    ) -> Vec<InvariantViolation> {
        match invariant_name {
            "ring_connectivity" => Self::check_ring_connectivity(harness).await,
            "successor_consistency" => Self::check_successor_consistency(harness).await,
            "predecessor_consistency" => Self::check_predecessor_consistency(harness).await,
            "data_availability" => Self::check_data_availability(harness).await,
            "data_consistency" => Self::check_data_consistency(harness).await,
            "load_balance" => Self::check_load_balance(harness).await,
            "fault_tolerance" => Self::check_fault_tolerance(harness).await,
            _ => vec![InvariantViolation {
                name: "Unknown".to_string(),
                description: format!("Unknown invariant: {}", invariant_name),
                affected_nodes: vec![],
            }],
        }
    }

    /// Get list of all available invariants
    pub fn list_invariants() -> Vec<&'static str> {
        vec![
            "ring_connectivity",
            "successor_consistency",
            "predecessor_consistency",
            "data_availability",
            "data_consistency",
            "load_balance",
            "fault_tolerance",
        ]
    }
}
