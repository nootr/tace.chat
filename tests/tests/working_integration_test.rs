//! Working integration tests that verify the framework step by step

use tace_integration_tests::integration::{NetworkInvariants, TestHarness};

#[tokio::test]
async fn test_basic_test_harness_operations() {
    let mut harness = TestHarness::new();

    // Test 1: Create nodes without starting them
    let node1 = harness
        .add_node([1; 20], 8001, 9001)
        .await
        .expect("Failed to add node1");
    let node2 = harness
        .add_node([2; 20], 8002, 9002)
        .await
        .expect("Failed to add node2");

    // Verify nodes were added
    let addresses = harness.get_all_node_addresses().await;
    assert_eq!(addresses.len(), 2);
    assert!(addresses.contains(&node1));
    assert!(addresses.contains(&node2));

    // Verify we can get the nodes
    assert!(harness.get_node(&node1).await.is_some());
    assert!(harness.get_node(&node2).await.is_some());
}

#[tokio::test]
async fn test_network_simulator_basic_operations() {
    use tace_integration_tests::integration::NetworkSimulator;
    use tace_lib::dht_messages::DhtMessage;
    use tace_node::NetworkClient;
    use tokio::sync::mpsc;

    let simulator = NetworkSimulator::new();

    // Test node registration
    let (tx, mut rx) = mpsc::unbounded_channel();
    simulator
        .register_node("127.0.0.1:8001".to_string(), tx)
        .await;

    assert!(simulator.is_node_registered("127.0.0.1:8001").await);
    assert!(!simulator.is_node_registered("127.0.0.1:8002").await);

    // Test network controls
    simulator.set_latency(100).await;
    simulator.set_drop_rate(0.1).await;
    simulator.mark_node_failed("127.0.0.1:8001").await;
    simulator.mark_node_recovered("127.0.0.1:8001").await;

    // Test message routing
    let client = simulator.create_client("127.0.0.1:8000".to_string());

    // Start a background task to handle received messages
    tokio::spawn(async move {
        while let Some(sim_message) = rx.recv().await {
            match sim_message {
                tace_integration_tests::integration::SimulatorMessage::Request {
                    from,
                    message,
                    request_id: _,
                    response_sender,
                } => {
                    println!("Node received message from {}: {:?}", from, message);
                    // Echo back a pong for ping
                    match message {
                        DhtMessage::Ping => {
                            let _ = response_sender.send(DhtMessage::Pong);
                        }
                        _ => {
                            let _ = response_sender.send(DhtMessage::Error {
                                message: "Unknown message".to_string(),
                            });
                        }
                    }
                    break; // Exit after first message for test
                }
                _ => break,
            }
        }
    });

    // Send a ping and verify response
    let response = client.call_node("127.0.0.1:8001", DhtMessage::Ping).await;
    assert!(response.is_ok());

    match response.unwrap() {
        DhtMessage::Pong => {} // Success
        other => panic!("Expected Pong, got {:?}", other),
    }
}

#[tokio::test]
async fn test_timing_controller() {
    use std::time::{Duration, Instant};
    use tace_integration_tests::integration::TimingController;

    let controller = TimingController::new();

    // Test time multiplier
    controller.set_time_multiplier(10.0).await; // 10x faster

    let start = Instant::now();
    controller.sleep(Duration::from_millis(100)).await;
    let elapsed = start.elapsed();

    // Should take ~10ms instead of 100ms
    assert!(
        elapsed < Duration::from_millis(50),
        "Expected fast execution, took {:?}",
        elapsed
    );

    // Test manual mode
    controller.enable_manual_mode().await;

    let start = Instant::now();
    let controller_clone = controller.clone();

    let task = tokio::spawn(async move {
        controller_clone.sleep(Duration::from_secs(1)).await;
    });

    // Task should not complete immediately in manual mode
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!task.is_finished());

    // Step should allow task to complete
    controller.step().await;
    task.await.expect("Task should complete");

    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(100),
        "Manual step should be fast, took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_network_invariants_structure() {
    let harness = TestHarness::new();

    // Test that invariant checks can run on empty network
    let violations = NetworkInvariants::check_all(&harness).await;
    assert!(
        violations.is_empty(),
        "Empty network should have no violations"
    );

    // Test individual invariant checks
    let ring_violations = NetworkInvariants::check_ring_connectivity(&harness).await;
    assert!(ring_violations.is_empty());

    let successor_violations = NetworkInvariants::check_successor_consistency(&harness).await;
    assert!(successor_violations.is_empty());

    let predecessor_violations = NetworkInvariants::check_predecessor_consistency(&harness).await;
    assert!(predecessor_violations.is_empty());

    // Test invariant listing
    let invariant_names = NetworkInvariants::list_invariants();
    assert!(invariant_names.contains(&"ring_connectivity"));
    assert!(invariant_names.contains(&"successor_consistency"));
    assert!(invariant_names.contains(&"predecessor_consistency"));
}

#[tokio::test]
async fn test_single_node_creation_and_access() {
    let mut harness = TestHarness::new();

    // Create a single node
    let node_addr = harness
        .add_node([42; 20], 8001, 9001)
        .await
        .expect("Failed to add node");

    // Get the node and verify it exists
    let _node = harness
        .get_node(&node_addr)
        .await
        .expect("Node should exist");

    // Test basic node properties (without network operations)
    // Note: We can't test network operations yet because the integration isn't complete

    // Verify the node address matches
    assert_eq!(node_addr, "127.0.0.1:8001");
}

#[tokio::test]
async fn test_error_handling() {
    let harness = TestHarness::new();

    // Test accessing non-existent node
    let result = harness.get_node("nonexistent").await;
    assert!(result.is_none());

    // Test network operations on empty network
    let violations = NetworkInvariants::check_all(&harness).await;
    assert!(violations.is_empty()); // Empty network should be valid
}

#[tokio::test]
async fn test_full_integration() {
    // Full integration test demonstrating complete DHT functionality

    let mut harness = TestHarness::new();

    // Create nodes
    let _node1 = harness
        .add_node([1; 20], 8001, 9001)
        .await
        .expect("Failed to add node1");
    let _node2 = harness
        .add_node([2; 20], 8002, 9002)
        .await
        .expect("Failed to add node2");

    // Start nodes (this creates the ChordNode instances but doesn't start network processing)
    harness
        .start_all_nodes()
        .await
        .expect("Failed to start nodes");

    // Connect nodes to the network
    harness
        .connect_node_to_network(&_node1, None)
        .await
        .expect("Failed to connect node1 to network");

    harness
        .connect_node_to_network(&_node2, Some(&_node1))
        .await
        .expect("Failed to connect node2 to network");

    // Wait for network stabilization
    harness
        .wait_for_stabilization(10)
        .await
        .expect("Network failed to stabilize");

    // Test data operations
    let test_key: [u8; 20] = [42; 20];
    let test_value = b"test_value".to_vec();

    harness
        .store_data(&_node1, test_key, test_value.clone())
        .await
        .expect("Failed to store data");

    let retrieved_values = harness
        .retrieve_data(&_node2, test_key)
        .await
        .expect("Failed to retrieve data")
        .expect("Data should exist");

    assert_eq!(
        retrieved_values,
        vec![test_value],
        "Retrieved data should match stored data"
    );

    println!("Full integration test completed successfully");
}
