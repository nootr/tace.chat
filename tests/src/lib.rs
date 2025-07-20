//! Integration tests for the DHT node network
//! 
//! This module demonstrates how to use the integration testing framework
//! to test real-world scenarios with multiple nodes.

pub mod integration;

use std::time::Duration;
use tace_node::NetworkClient;
use integration::{TestHarness, NetworkInvariants, TestScenarios};

#[tokio::test]
async fn test_basic_network_formation() {
    TestScenarios::basic_network_formation()
        .await
        .expect("Basic network formation should succeed");
}

#[tokio::test]
async fn test_data_operations() {
    TestScenarios::data_operations_test()
        .await
        .expect("Data operations should work correctly");
}

#[tokio::test]
async fn test_dynamic_membership() {
    TestScenarios::dynamic_membership_test()
        .await
        .expect("Dynamic membership should work correctly");
}

#[tokio::test]
async fn test_fault_tolerance() {
    TestScenarios::fault_tolerance_test()
        .await
        .expect("Network should be fault tolerant");
}

#[tokio::test]
async fn test_small_scalability() {
    TestScenarios::scalability_test(10)
        .await
        .expect("Should scale to 10 nodes");
}

#[tokio::test]
#[ignore] // Long-running test
async fn test_medium_scalability() {
    TestScenarios::scalability_test(50)
        .await
        .expect("Should scale to 50 nodes");
}

#[tokio::test]
async fn test_partition_healing() {
    TestScenarios::partition_healing_test()
        .await
        .expect("Network should recover from partitions");
}

#[tokio::test]
async fn test_invariants_on_simple_network() {
    let mut harness = TestHarness::new();

    // Create simple 3-node network
    let node1 = harness.add_node([1; 20], 8001, 9001).await.unwrap();
    let node2 = harness.add_node([2; 20], 8002, 9002).await.unwrap();
    let node3 = harness.add_node([3; 20], 8003, 9003).await.unwrap();

    harness.start_all_nodes().await.unwrap();

    // Form network
    harness.connect_node_to_network(&node1, None).await.unwrap();
    harness.connect_node_to_network(&node2, Some(&node1)).await.unwrap();
    harness.connect_node_to_network(&node3, Some(&node1)).await.unwrap();

    // Wait for stabilization
    harness.wait_for_stabilization(30).await.unwrap();

    // Test individual invariants
    let ring_violations = NetworkInvariants::check_ring_connectivity(&harness).await;
    assert!(ring_violations.is_empty(), "Ring should be connected: {:?}", ring_violations);

    let succ_violations = NetworkInvariants::check_successor_consistency(&harness).await;
    assert!(succ_violations.is_empty(), "Successors should be consistent: {:?}", succ_violations);

    let pred_violations = NetworkInvariants::check_predecessor_consistency(&harness).await;
    assert!(pred_violations.is_empty(), "Predecessors should be consistent: {:?}", pred_violations);

    // Test all invariants
    let all_violations = NetworkInvariants::check_all(&harness).await;
    assert!(all_violations.is_empty(), "All invariants should hold: {:?}", all_violations);
}

#[tokio::test]
async fn test_timing_control() {
    use std::time::{Duration, Instant};
    
    let mut harness = TestHarness::new();
    
    // Test manual timing mode
    harness.timing().enable_manual_mode().await;
    
    let start = Instant::now();
    
    // Start a delayed operation
    let timing = harness.timing().clone();
    let task = tokio::spawn(async move {
        timing.sleep(Duration::from_secs(10)).await; // Would normally take 10 seconds
    });
    
    // Should not complete immediately
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!task.is_finished());
    
    // Step should allow it to complete
    harness.timing().step().await;
    task.await.unwrap();
    
    // Should have taken much less than 10 seconds
    assert!(start.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn test_network_simulation() {
    use integration::NetworkSimulator;
    use tokio::sync::mpsc;
    use tace_lib::dht_messages::DhtMessage;
    
    let simulator = NetworkSimulator::new();
    
    // Create a mock node
    let (tx, mut rx) = mpsc::unbounded_channel();
    simulator.register_node("127.0.0.1:8001".to_string(), tx).await;
    
    // Create client and send message
    let client = simulator.create_client("127.0.0.1:8000".to_string());
    
    let _response = client.call_node("127.0.0.1:8001", DhtMessage::Ping).await;
    
    // Verify message was received (in real implementation)
    // This would require more sophisticated message handling
}

/// Example of a comprehensive test scenario
#[tokio::test]
#[ignore] // Long-running comprehensive test
async fn comprehensive_integration_test() {
    TestScenarios::run_comprehensive_test_suite()
        .await
        .expect("Comprehensive test suite should pass");
}

/// Example of testing specific failure scenarios
#[tokio::test]
async fn test_rapid_node_churn() {
    let mut harness = TestHarness::new();
    
    // Start with stable network
    let mut nodes = Vec::new();
    for i in 0..5 {
        let node_id = [(i + 1) as u8; 20];
        let node = harness.add_node(node_id, 8000 + i as u16, 9000 + i as u16).await.unwrap();
        nodes.push(node);
    }
    
    harness.start_all_nodes().await.unwrap();
    
    // Form initial network
    harness.connect_node_to_network(&nodes[0], None).await.unwrap();
    for i in 1..5 {
        harness.connect_node_to_network(&nodes[i], Some(&nodes[0])).await.unwrap();
    }
    
    harness.wait_for_stabilization(30).await.unwrap();
    
    // Rapid node additions and removals
    for cycle in 0..3 {
        // Add nodes rapidly
        let mut new_nodes = Vec::new();
        for i in 0..3 {
            let node_id = [(cycle * 3 + i + 10) as u8; 20];
            let node = harness.add_node(node_id, 8100 + (cycle * 3 + i) as u16, 9100 + (cycle * 3 + i) as u16).await.unwrap();
            harness.connect_node_to_network(&node, Some(&nodes[0])).await.unwrap();
            new_nodes.push(node);
            
            // Brief pause
            harness.timing().sleep(Duration::from_millis(100)).await;
        }
        
        // Remove nodes rapidly
        for node in &new_nodes {
            harness.fail_node(node).await.unwrap();
            harness.timing().sleep(Duration::from_millis(100)).await;
        }
        
        // Check network is still healthy
        harness.timing().sleep(Duration::from_millis(500)).await;
        let violations = NetworkInvariants::check_ring_connectivity(&harness).await;
        assert!(violations.is_empty(), "Network should remain connected during churn");
    }
}