//! Debug test to understand wait_for_stabilization behavior with 2 nodes

use tace_integration_tests::integration::TestHarness;

#[tokio::test]
async fn debug_wait_for_stabilization_two_nodes() {
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

    // Test the wait_for_stabilization method
    println!("\n=== Testing wait_for_stabilization ===");
    match harness.wait_for_stabilization(10).await {
        Ok(_) => {
            println!("✅ wait_for_stabilization succeeded");
            debug_print_network_state(&harness).await;
        }
        Err(e) => {
            println!("❌ wait_for_stabilization failed: {}", e);
            debug_print_network_state(&harness).await;

            // Let's manually trigger a few rounds and see what happens
            println!("\n=== Manual stabilization rounds ===");
            for i in 0..5 {
                harness
                    .trigger_stabilization_round()
                    .await
                    .expect("Stabilization failed");
                println!("\n--- After manual round {} ---", i + 1);
                debug_print_network_state(&harness).await;
                check_ring_formation_manually(&harness, &node1, &node2).await;
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

            println!(
                "Node {}: successor={:?}, predecessor={:?}",
                address,
                successor.as_ref().map(|(_, addr, _)| addr.as_str()),
                predecessor.as_ref().map(|(_, addr, _)| addr.as_str())
            );
        }
    }
}

async fn check_ring_formation_manually(harness: &TestHarness, node1: &str, node2: &str) {
    let addresses = harness.get_all_node_addresses().await;
    println!("Checking ring formation for 2-node network:");
    println!("  Addresses: {:?}", addresses);

    if let (Some(n1), Some(n2)) = (harness.get_node(node1).await, harness.get_node(node2).await) {
        let n1_successor = n1.get_successor().await;
        let n2_successor = n2.get_successor().await;
        let n1_predecessor = n1.get_predecessor().await;
        let n2_predecessor = n2.get_predecessor().await;

        // Check the exact same logic as wait_for_stabilization
        let all_have_successors = n1_successor.is_some()
            && n2_successor.is_some()
            && n1_predecessor.is_some()
            && n2_predecessor.is_some();
        println!(
            "  All have successors/predecessors: {}",
            all_have_successors
        );

        if let (Some((_, addr1, _)), Some((_, addr2, _))) = (n1_successor, n2_successor) {
            let ring_formed = (addr1 == addresses[1] && addr2 == addresses[0])
                || (addr1 == addresses[0] && addr2 == addresses[1]);
            println!("  Ring formed (successor check): {}", ring_formed);
            println!(
                "    N1 successor: {}, addresses[1]: {}",
                addr1, addresses[1]
            );
            println!(
                "    N2 successor: {}, addresses[0]: {}",
                addr2, addresses[0]
            );
            println!(
                "    Check 1: {} == {} && {} == {} = {}",
                addr1,
                addresses[1],
                addr2,
                addresses[0],
                addr1 == addresses[1] && addr2 == addresses[0]
            );
            println!(
                "    Check 2: {} == {} && {} == {} = {}",
                addr1,
                addresses[0],
                addr2,
                addresses[1],
                addr1 == addresses[0] && addr2 == addresses[1]
            );
        } else {
            println!("  Ring formed: false (missing successors)");
        }

        if all_have_successors {
            println!("  ✅ Basic requirements met");
        } else {
            println!("  ❌ Basic requirements not met");
        }
    }
}
