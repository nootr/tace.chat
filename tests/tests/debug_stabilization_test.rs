//! Debug test to understand stabilization behavior

use tace_integration_tests::integration::{TestHarness, NetworkInvariants};
use std::time::Duration;

#[tokio::test]
async fn debug_three_node_stabilization() {
    let mut harness = TestHarness::new();

    // Create 3 nodes
    let node1 = harness.add_node([1; 20], 8001, 9001).await.expect("Failed to add node1");
    let node2 = harness.add_node([2; 20], 8002, 9002).await.expect("Failed to add node2");
    let node3 = harness.add_node([3; 20], 8003, 9003).await.expect("Failed to add node3");

    println!("=== Created 3 nodes ===");
    println!("Node1: {}", node1);
    println!("Node2: {}", node2);
    println!("Node3: {}", node3);

    // Start all nodes
    harness.start_all_nodes().await.expect("Failed to start nodes");
    
    // Create network with first node
    harness.connect_node_to_network(&node1, None).await.expect("Failed to create network");
    
    println!("\n=== After node1 creates network ===");
    debug_print_network_state(&harness).await;

    // Join second node
    harness.connect_node_to_network(&node2, Some(&node1)).await.expect("Failed to join node2");
    
    println!("\n=== After node2 joins ===");
    debug_print_network_state(&harness).await;

    // Run some stabilization cycles
    println!("\n=== Running 3 stabilization cycles ===");
    for i in 0..3 {
        harness.trigger_stabilization_round().await.expect("Stabilization failed");
        println!("\n--- After stabilization cycle {} ---", i + 1);
        debug_print_network_state(&harness).await;
    }

    // Join third node
    harness.connect_node_to_network(&node3, Some(&node1)).await.expect("Failed to join node3");
    
    println!("\n=== After node3 joins ===");
    debug_print_network_state(&harness).await;

    // Run more stabilization cycles
    println!("\n=== Running 10 more stabilization cycles ===");
    for i in 0..10 {
        harness.trigger_stabilization_round().await.expect("Stabilization failed");
        println!("\n--- After stabilization cycle {} ---", i + 1);
        debug_print_network_state(&harness).await;
        
        // Check invariants
        let violations = NetworkInvariants::check_all(&harness).await;
        if violations.is_empty() {
            println!("✅ All invariants satisfied after {} cycles!", i + 1);
            return;
        } else {
            println!("⚠️  Invariant violations: {}", violations.len());
            for violation in &violations {
                println!("   - {}: {}", violation.name, violation.description);
            }
        }
    }
}

async fn debug_print_network_state(harness: &TestHarness) {
    let addresses = harness.get_all_node_addresses().await;
    
    for address in &addresses {
        if let Some(node) = harness.get_node(address).await {
            let successor = node.get_successor().await;
            let predecessor = node.get_predecessor().await;
            
            println!("Node {}: successor={:?}, predecessor={:?}", 
                address,
                successor.as_ref().map(|(_, addr, _)| addr.as_str()),
                predecessor.as_ref().map(|(_, addr, _)| addr.as_str())
            );
        }
    }
}