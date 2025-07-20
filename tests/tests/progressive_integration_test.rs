//! Progressive integration test to understand what's working step by step

use std::time::Duration;
use tace_integration_tests::integration::{NetworkInvariants, TestHarness};

#[tokio::test]
async fn test_step_01_node_creation() {
    let mut harness = TestHarness::new();

    // Step 1: Can we create nodes?
    let node1 = harness
        .add_node([1; 20], 8001, 9001)
        .await
        .expect("Failed to add node1");
    let node2 = harness
        .add_node([2; 20], 8002, 9002)
        .await
        .expect("Failed to add node2");
    let node3 = harness
        .add_node([3; 20], 8003, 9003)
        .await
        .expect("Failed to add node3");

    println!("✅ Step 1: Node creation works");
    println!("Created nodes: {}, {}, {}", node1, node2, node3);
}

#[tokio::test]
async fn test_step_02_node_starting() {
    let mut harness = TestHarness::new();

    let node1 = harness
        .add_node([1; 20], 8001, 9001)
        .await
        .expect("Failed to add node1");
    let node2 = harness
        .add_node([2; 20], 8002, 9002)
        .await
        .expect("Failed to add node2");

    // Step 2: Can we start nodes?
    harness
        .start_all_nodes()
        .await
        .expect("Failed to start nodes");

    println!("✅ Step 2: Node starting works");

    // Check we can access the nodes
    let n1 = harness.get_node(&node1).await.expect("Should get node1");
    let n2 = harness.get_node(&node2).await.expect("Should get node2");

    // Check initial state
    let n1_successor = n1.get_successor().await;
    let n2_successor = n2.get_successor().await;

    println!("Node1 successor: {:?}", n1_successor);
    println!("Node2 successor: {:?}", n2_successor);
}

#[tokio::test]
async fn test_step_03_single_node_network() {
    let mut harness = TestHarness::new();

    let node1 = harness
        .add_node([1; 20], 8001, 9001)
        .await
        .expect("Failed to add node1");
    harness
        .start_all_nodes()
        .await
        .expect("Failed to start nodes");

    // Step 3: Can we create a single-node network?
    let result = harness.connect_node_to_network(&node1, None).await;
    println!("Single node network creation result: {:?}", result);

    if result.is_ok() {
        // Wait a moment
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Check state
        let n1 = harness.get_node(&node1).await.expect("Should get node1");
        let successor = n1.get_successor().await;
        let predecessor = n1.get_predecessor().await;

        println!("✅ Step 3: Single node network creation works");
        println!("Successor: {:?}", successor);
        println!("Predecessor: {:?}", predecessor);
    } else {
        println!(
            "❌ Step 3: Single node network creation failed: {:?}",
            result
        );
    }
}

#[tokio::test]
async fn test_step_04_join_operation() {
    let mut harness = TestHarness::new();

    let node1 = harness
        .add_node([1; 20], 8001, 9001)
        .await
        .expect("Failed to add node1");
    let node2 = harness
        .add_node([2; 20], 8002, 9002)
        .await
        .expect("Failed to add node2");

    harness
        .start_all_nodes()
        .await
        .expect("Failed to start nodes");

    // Create network with first node
    harness
        .connect_node_to_network(&node1, None)
        .await
        .expect("Failed to create network");

    // Wait a moment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Step 4: Can the second node join?
    let result = harness.connect_node_to_network(&node2, Some(&node1)).await;
    println!("Node2 join result: {:?}", result);

    if result.is_ok() {
        tokio::time::sleep(Duration::from_millis(200)).await;

        let n1 = harness.get_node(&node1).await.expect("Should get node1");
        let n2 = harness.get_node(&node2).await.expect("Should get node2");

        println!("✅ Step 4: Node join operation works");
        println!("Node1 successor: {:?}", n1.get_successor().await);
        println!("Node1 predecessor: {:?}", n1.get_predecessor().await);
        println!("Node2 successor: {:?}", n2.get_successor().await);
        println!("Node2 predecessor: {:?}", n2.get_predecessor().await);
    } else {
        println!("❌ Step 4: Node join operation failed: {:?}", result);
    }
}

#[tokio::test]
async fn test_step_05_stabilization() {
    let mut harness = TestHarness::new();

    let node1 = harness
        .add_node([1; 20], 8001, 9001)
        .await
        .expect("Failed to add node1");
    let node2 = harness
        .add_node([2; 20], 8002, 9002)
        .await
        .expect("Failed to add node2");

    harness
        .start_all_nodes()
        .await
        .expect("Failed to start nodes");
    harness
        .connect_node_to_network(&node1, None)
        .await
        .expect("Failed to create network");
    harness
        .connect_node_to_network(&node2, Some(&node1))
        .await
        .expect("Failed to join node2");

    // Step 5: Does stabilization work?
    let result = harness.wait_for_stabilization(5).await; // Short timeout
    println!("Stabilization result: {:?}", result);

    if result.is_ok() {
        println!("✅ Step 5: Stabilization works");

        // Check final state
        let n1 = harness.get_node(&node1).await.expect("Should get node1");
        let n2 = harness.get_node(&node2).await.expect("Should get node2");

        println!("Final Node1 successor: {:?}", n1.get_successor().await);
        println!("Final Node2 successor: {:?}", n2.get_successor().await);
    } else {
        println!("❌ Step 5: Stabilization failed: {:?}", result);

        // Still check state to see what we have
        let n1 = harness.get_node(&node1).await.expect("Should get node1");
        let n2 = harness.get_node(&node2).await.expect("Should get node2");

        println!("Partial Node1 successor: {:?}", n1.get_successor().await);
        println!("Partial Node2 successor: {:?}", n2.get_successor().await);
    }
}

#[tokio::test]
async fn test_step_06_invariant_checking() {
    let mut harness = TestHarness::new();

    let node1 = harness
        .add_node([1; 20], 8001, 9001)
        .await
        .expect("Failed to add node1");
    let node2 = harness
        .add_node([2; 20], 8002, 9002)
        .await
        .expect("Failed to add node2");

    harness
        .start_all_nodes()
        .await
        .expect("Failed to start nodes");
    harness
        .connect_node_to_network(&node1, None)
        .await
        .expect("Failed to create network");
    harness
        .connect_node_to_network(&node2, Some(&node1))
        .await
        .expect("Failed to join node2");

    // Wait a bit regardless of formal stabilization
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Step 6: What do the invariants say?
    let violations = NetworkInvariants::check_all(&harness).await;

    println!("Invariant violations: {} total", violations.len());
    for violation in &violations {
        println!("  - {}: {}", violation.name, violation.description);
    }

    if violations.is_empty() {
        println!("✅ Step 6: All invariants pass");
    } else {
        println!("⚠️  Step 6: Some invariants fail, but network partially works");
    }

    // Test specific operations
    let n1 = harness.get_node(&node1).await.expect("Should get node1");
    println!("Node1 can be accessed: ✅");

    let successor = n1.get_successor().await;
    if successor.is_some() {
        println!("Node1 has successor: ✅");
    } else {
        println!("Node1 has no successor: ❌");
    }
}
