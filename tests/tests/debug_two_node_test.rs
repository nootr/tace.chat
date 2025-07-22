//! Debug test to understand 2-node stabilization behavior

use tace_integration_tests::integration::{NetworkInvariants, TestHarness};

#[tokio::test]
async fn debug_two_node_stabilization() {
    let mut harness = TestHarness::new();

    // Create 2 nodes
    let node1 = harness
        .add_node([1; 20], 8001, 9001)
        .await
        .expect("Failed to add node1");
    let node2 = harness
        .add_node([2; 20], 8002, 9002)
        .await
        .expect("Failed to add node2");

    println!("=== Created 2 nodes ===");
    println!("Node1: {} (ID: {:?})", node1, [1; 20]);
    println!("Node2: {} (ID: {:?})", node2, [2; 20]);

    // Start all nodes
    harness
        .start_all_nodes()
        .await
        .expect("Failed to start nodes");

    // Create network with first node
    harness
        .connect_node_to_network(&node1, None)
        .await
        .expect("Failed to create network");

    println!("\n=== After node1 creates network ===");
    debug_print_network_state(&harness).await;

    // Join second node
    harness
        .connect_node_to_network(&node2, Some(&node1))
        .await
        .expect("Failed to join node2");

    println!("\n=== After node2 joins ===");
    debug_print_network_state(&harness).await;

    // Run stabilization cycles one by one and observe
    println!("\n=== Running stabilization cycles ===");
    for i in 0..10 {
        harness
            .trigger_stabilization_round()
            .await
            .expect("Stabilization failed");
        println!("\n--- After stabilization cycle {} ---", i + 1);
        debug_print_network_state(&harness).await;

        // Check if we have the proper 2-node ring
        if let (Some(n1), Some(n2)) = (
            harness.get_node(&node1).await,
            harness.get_node(&node2).await,
        ) {
            let n1_successor = n1.get_successor().await;
            let n2_successor = n2.get_successor().await;
            let n1_predecessor = n1.get_predecessor().await;
            let n2_predecessor = n2.get_predecessor().await;

            println!("Ring analysis:");
            println!(
                "  N1 -> {:?}",
                n1_successor.as_ref().map(|(_, addr, _)| addr)
            );
            println!(
                "  N1 <- {:?}",
                n1_predecessor.as_ref().map(|(_, addr, _)| addr)
            );
            println!(
                "  N2 -> {:?}",
                n2_successor.as_ref().map(|(_, addr, _)| addr)
            );
            println!(
                "  N2 <- {:?}",
                n2_predecessor.as_ref().map(|(_, addr, _)| addr)
            );

            // Check if proper ring is formed
            let ring_formed = match (n1_successor, n2_successor, n1_predecessor, n2_predecessor) {
                (
                    Some((_, n1_succ, _)),
                    Some((_, n2_succ, _)),
                    Some((_, n1_pred, _)),
                    Some((_, n2_pred, _)),
                ) => {
                    let ring1 = n1_succ == node2
                        && n2_succ == node1
                        && n1_pred == node2
                        && n2_pred == node1;
                    println!(
                        "  Ring check: {}",
                        if ring1 {
                            "✅ Perfect 2-node ring"
                        } else {
                            "❌ Not a proper ring"
                        }
                    );
                    ring1
                }
                _ => {
                    println!("  Ring check: ❌ Missing successors/predecessors");
                    false
                }
            };

            if ring_formed {
                println!("✅ Proper 2-node ring achieved after {} cycles!", i + 1);
                return;
            }
        }

        // Check invariants
        let violations = NetworkInvariants::check_all(&harness).await;
        if violations.is_empty() {
            println!("✅ All invariants satisfied");
        } else {
            println!("⚠️  Invariant violations: {}", violations.len());
            for violation in &violations {
                println!("   - {}: {}", violation.name, violation.description);
            }
        }
    }

    println!("❌ Failed to form proper 2-node ring after 10 cycles");
}

async fn debug_print_network_state(harness: &TestHarness) {
    let addresses = harness.get_all_node_addresses().await;

    for address in &addresses {
        if let Some(node) = harness.get_node(address).await {
            let successor = node.get_successor().await;
            let predecessor = node.get_predecessor().await;

            println!(
                "Node {}: successor={:?}, predecessor={:?}",
                address,
                successor.as_ref().map(|(_, addr, _)| addr.as_str()),
                predecessor.as_ref().map(|(_, addr, _)| addr.as_str())
            );
        }
    }
}
