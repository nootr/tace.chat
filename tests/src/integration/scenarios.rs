use std::time::Duration;
use tace_lib::dht_messages::NodeId;
use crate::integration::{TestHarness, NetworkInvariants, InvariantViolation};

/// Common test scenarios for DHT network integration testing
pub struct TestScenarios;

impl TestScenarios {
    /// Basic network formation test
    /// Creates a small network and verifies basic connectivity
    pub async fn basic_network_formation() -> Result<(), Box<dyn std::error::Error>> {
        let mut harness = TestHarness::new();

        // Create 3 nodes
        let node1 = harness.add_node([1; 20], 8001, 9001).await?;
        let node2 = harness.add_node([2; 20], 8002, 9002).await?;
        let node3 = harness.add_node([3; 20], 8003, 9003).await?;

        // Start all nodes
        harness.start_all_nodes().await?;

        // Create network with first node
        harness.connect_node_to_network(&node1, None).await?;

        // Join other nodes to the network
        harness.connect_node_to_network(&node2, Some(&node1)).await?;
        harness.connect_node_to_network(&node3, Some(&node1)).await?;

        // Wait for stabilization
        harness.wait_for_stabilization(30).await?;

        // Check invariants
        let violations = NetworkInvariants::check_all(&harness).await;
        if !violations.is_empty() {
            return Err(format!("Invariant violations: {:?}", violations).into());
        }

        Ok(())
    }

    /// Node join/leave stress test
    /// Tests network stability under dynamic membership
    pub async fn dynamic_membership_test() -> Result<(), Box<dyn std::error::Error>> {
        let mut harness = TestHarness::new();

        // Start with a stable 5-node network
        let mut nodes = Vec::new();
        for i in 0..5 {
            let node_id = [(i + 1) as u8; 20];
            let node = harness.add_node(node_id, 8000 + i as u16, 9000 + i as u16).await?;
            nodes.push(node);
        }

        harness.start_all_nodes().await?;

        // Create network
        harness.connect_node_to_network(&nodes[0], None).await?;
        for i in 1..5 {
            harness.connect_node_to_network(&nodes[i], Some(&nodes[0])).await?;
        }

        harness.wait_for_stabilization(30).await?;

        // Add more nodes dynamically
        for i in 5..10 {
            let node_id = [(i + 1) as u8; 20];
            let new_node = harness.add_node(node_id, 8000 + i as u16, 9000 + i as u16).await?;
            harness.connect_node_to_network(&new_node, Some(&nodes[0])).await?;
            
            // Brief stabilization
            harness.timing().sleep(Duration::from_millis(500)).await;
            
            // Check invariants after each join
            let violations = NetworkInvariants::check_ring_connectivity(&harness).await;
            if !violations.is_empty() {
                return Err(format!("Ring connectivity failed after adding node {}: {:?}", i, violations).into());
            }
            
            nodes.push(new_node);
        }

        // Final stabilization
        harness.wait_for_stabilization(60).await?;

        // Check all invariants
        let violations = NetworkInvariants::check_all(&harness).await;
        if !violations.is_empty() {
            return Err(format!("Final invariant violations: {:?}", violations).into());
        }

        Ok(())
    }

    /// Data storage and retrieval test
    /// Verifies data operations work correctly across the network
    pub async fn data_operations_test() -> Result<(), Box<dyn std::error::Error>> {
        let mut harness = TestHarness::new();

        // Create 4-node network
        let mut nodes = Vec::new();
        for i in 0..4 {
            let node_id = [(i + 1) as u8; 20];
            let node = harness.add_node(node_id, 8000 + i as u16, 9000 + i as u16).await?;
            nodes.push(node);
        }

        harness.start_all_nodes().await?;

        // Form network
        harness.connect_node_to_network(&nodes[0], None).await?;
        for i in 1..4 {
            harness.connect_node_to_network(&nodes[i], Some(&nodes[0])).await?;
        }

        harness.wait_for_stabilization(30).await?;

        // Store data from different nodes
        let test_data = vec![
            ([10; 20], b"data1".to_vec()),
            ([20; 20], b"data2".to_vec()),
            ([30; 20], b"data3".to_vec()),
            ([40; 20], b"data4".to_vec()),
        ];

        for (i, (key, value)) in test_data.iter().enumerate() {
            harness.store_data(&nodes[i % nodes.len()], *key, value.clone()).await?;
        }

        // Wait for replication
        harness.timing().sleep(Duration::from_millis(1000)).await;

        // Retrieve data from different nodes
        for (key, expected_value) in &test_data {
            for node in &nodes {
                if let Some(retrieved_values) = harness.retrieve_data(node, *key).await? {
                    if !retrieved_values.iter().any(|v| v == expected_value) {
                        return Err(format!("Data mismatch for key {:?} from node {}", key, node).into());
                    }
                } else {
                    return Err(format!("Data not found for key {:?} from node {}", key, node).into());
                }
            }
        }

        Ok(())
    }

    /// Fault tolerance test
    /// Tests network resilience to node failures
    pub async fn fault_tolerance_test() -> Result<(), Box<dyn std::error::Error>> {
        let mut harness = TestHarness::new();

        // Create 6-node network for better fault tolerance
        let mut nodes = Vec::new();
        for i in 0..6 {
            let node_id = [(i + 1) as u8; 20];
            let node = harness.add_node(node_id, 8000 + i as u16, 9000 + i as u16).await?;
            nodes.push(node);
        }

        harness.start_all_nodes().await?;

        // Form network
        harness.connect_node_to_network(&nodes[0], None).await?;
        for i in 1..6 {
            harness.connect_node_to_network(&nodes[i], Some(&nodes[0])).await?;
        }

        harness.wait_for_stabilization(30).await?;

        // Store some data
        let test_keys = vec![[10; 20], [20; 20], [30; 20]];
        for (i, key) in test_keys.iter().enumerate() {
            harness.store_data(&nodes[i], *key, format!("data{}", i).into_bytes()).await?;
        }

        harness.timing().sleep(Duration::from_millis(1000)).await;

        // Fail 2 nodes
        harness.fail_node(&nodes[1]).await?;
        harness.fail_node(&nodes[3]).await?;

        // Wait for network to adapt
        harness.timing().sleep(Duration::from_millis(2000)).await;

        // Verify network is still functional
        let violations = NetworkInvariants::check_ring_connectivity(&harness).await;
        if !violations.is_empty() {
            return Err(format!("Network not connected after failures: {:?}", violations).into());
        }

        // Verify data is still accessible
        for key in &test_keys {
            let mut found = false;
            for node in &nodes[..] {
                if nodes[1] == *node || nodes[3] == *node {
                    continue; // Skip failed nodes
                }
                
                if harness.retrieve_data(node, *key).await?.is_some() {
                    found = true;
                    break;
                }
            }
            
            if !found {
                return Err(format!("Data lost for key {:?} after node failures", key).into());
            }
        }

        // Recover nodes
        harness.recover_node(&nodes[1]).await?;
        harness.recover_node(&nodes[3]).await?;

        // Final stabilization and check
        harness.wait_for_stabilization(30).await?;
        let violations = NetworkInvariants::check_all(&harness).await;
        if !violations.is_empty() {
            return Err(format!("Invariant violations after recovery: {:?}", violations).into());
        }

        Ok(())
    }

    /// Large network scalability test
    /// Tests behavior with many nodes
    pub async fn scalability_test(node_count: usize) -> Result<(), Box<dyn std::error::Error>> {
        let mut harness = TestHarness::new();

        // Speed up timing for large tests
        harness.timing().set_time_multiplier(5.0).await;

        // Create many nodes
        let mut nodes = Vec::new();
        for i in 0..node_count {
            let node_id = [(i + 1) as u8; 20];
            let node = harness.add_node(node_id, 8000 + i as u16, 9000 + i as u16).await?;
            nodes.push(node);
        }

        harness.start_all_nodes().await?;

        // Form network gradually
        harness.connect_node_to_network(&nodes[0], None).await?;
        
        for i in 1..node_count {
            harness.connect_node_to_network(&nodes[i], Some(&nodes[0])).await?;
            
            // Check connectivity every 10 nodes
            if i % 10 == 0 {
                harness.timing().sleep(Duration::from_millis(100)).await;
                let violations = NetworkInvariants::check_ring_connectivity(&harness).await;
                if !violations.is_empty() {
                    return Err(format!("Connectivity lost at {} nodes", i).into());
                }
            }
        }

        // Final stabilization
        harness.wait_for_stabilization(60).await?;

        // Check final state
        let violations = NetworkInvariants::check_all(&harness).await;
        if !violations.is_empty() {
            return Err(format!("Scalability test failed with {} nodes: {:?}", node_count, violations).into());
        }

        Ok(())
    }

    /// Network partition and healing test
    /// Simulates network splits and recovery
    pub async fn partition_healing_test() -> Result<(), Box<dyn std::error::Error>> {
        let mut harness = TestHarness::new();

        // Create 8-node network (will split into 2 groups of 4)
        let mut nodes = Vec::new();
        for i in 0..8 {
            let node_id = [(i + 1) as u8; 20];
            let node = harness.add_node(node_id, 8000 + i as u16, 9000 + i as u16).await?;
            nodes.push(node);
        }

        harness.start_all_nodes().await?;

        // Form network
        harness.connect_node_to_network(&nodes[0], None).await?;
        for i in 1..8 {
            harness.connect_node_to_network(&nodes[i], Some(&nodes[0])).await?;
        }

        harness.wait_for_stabilization(30).await?;

        // Create partition by failing communication between two groups
        for i in 0..4 {
            harness.fail_node(&nodes[i]).await?;
        }

        // Wait for partition to stabilize
        harness.timing().sleep(Duration::from_millis(2000)).await;

        // Each partition should still be internally consistent
        // (This is a simplified test - real partition testing would be more complex)

        // Heal the partition
        for i in 0..4 {
            harness.recover_node(&nodes[i]).await?;
        }

        // Wait for network to heal
        harness.wait_for_stabilization(60).await?;

        // Verify final consistency
        let violations = NetworkInvariants::check_all(&harness).await;
        if !violations.is_empty() {
            return Err(format!("Network not consistent after partition healing: {:?}", violations).into());
        }

        Ok(())
    }

    /// Run a suite of basic tests
    pub async fn run_basic_test_suite() -> Result<(), Box<dyn std::error::Error>> {
        println!("Running basic network formation test...");
        Self::basic_network_formation().await?;
        
        println!("Running data operations test...");
        Self::data_operations_test().await?;
        
        println!("Running dynamic membership test...");
        Self::dynamic_membership_test().await?;
        
        println!("Running fault tolerance test...");
        Self::fault_tolerance_test().await?;
        
        println!("All basic tests passed!");
        Ok(())
    }

    /// Run comprehensive test suite
    pub async fn run_comprehensive_test_suite() -> Result<(), Box<dyn std::error::Error>> {
        // Run basic tests first
        Self::run_basic_test_suite().await?;
        
        println!("Running scalability test (20 nodes)...");
        Self::scalability_test(20).await?;
        
        println!("Running partition healing test...");
        Self::partition_healing_test().await?;
        
        println!("All comprehensive tests passed!");
        Ok(())
    }
}