//! Simple data operations test for debugging

use tace_integration_tests::integration::{NetworkInvariants, TestHarness};

#[tokio::test]
async fn test_simple_data_operations() {
    let mut harness = TestHarness::new();

    // Create just 2 nodes for simpler testing
    let node1 = harness
        .add_node([1; 20], 8001, 9001)
        .await
        .expect("Failed to add node1");
    let node2 = harness
        .add_node([2; 20], 8002, 9002)
        .await
        .expect("Failed to add node2");

    println!("=== Created 2 nodes ===");

    // Start nodes
    harness
        .start_all_nodes()
        .await
        .expect("Failed to start nodes");

    // Form network
    harness
        .connect_node_to_network(&node1, None)
        .await
        .expect("Failed to create network");
    harness
        .connect_node_to_network(&node2, Some(&node1))
        .await
        .expect("Failed to join node2");

    println!("=== Nodes joined network ===");

    // Wait for stabilization
    if let Err(e) = harness.wait_for_stabilization(5).await {
        println!("⚠️ Stabilization failed: {}", e);
        println!("Continuing with partial stabilization...");
    } else {
        println!("✅ Network stabilized");
    }

    // Check ring state
    let violations = NetworkInvariants::check_all(&harness).await;
    println!("Network violations: {}", violations.len());
    for violation in &violations {
        println!("  - {}: {}", violation.name, violation.description);
    }

    // Test simple data operations
    let test_key = [10; 20];
    let test_value = b"test_data".to_vec();

    println!("=== Testing data storage ===");

    // Store data from node1
    match harness
        .store_data(&node1, test_key, test_value.clone())
        .await
    {
        Ok(_) => println!("✅ Data stored successfully"),
        Err(e) => println!("❌ Data storage failed: {}", e),
    }

    // Wait briefly for any propagation
    harness
        .timing()
        .sleep(std::time::Duration::from_millis(100))
        .await;

    println!("=== Testing data retrieval ===");

    // Try to retrieve from both nodes
    for (i, node) in [&node1, &node2].iter().enumerate() {
        match harness.retrieve_data(node, test_key).await {
            Ok(Some(values)) => {
                println!("✅ Node{} retrieved data: {} values", i + 1, values.len());
                if values.iter().any(|v| v == &test_value) {
                    println!("✅ Node{} has correct data", i + 1);
                } else {
                    println!("❌ Node{} has wrong data", i + 1);
                }
            }
            Ok(None) => println!("⚠️ Node{} found no data", i + 1),
            Err(e) => println!("❌ Node{} retrieval failed: {}", i + 1, e),
        }
    }

    println!("=== Test completed ===");
}
