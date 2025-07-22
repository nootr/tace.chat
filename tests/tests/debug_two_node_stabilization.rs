//! Debug test to understand why 2-node stabilization isn't working
use std::time::Duration;
use tace_integration_tests::integration::TestHarness;

#[tokio::test]
async fn debug_two_node_stabilization() {
    let mut harness = TestHarness::new();

    // Speed up time significantly for faster convergence
    harness.timing().set_time_multiplier(100.0).await;

    // Create 2 nodes with well-spaced IDs
    let mut node1_id = [0u8; 20];
    node1_id[0] = 0;
    let node1 = harness
        .add_node(node1_id, 8001, 9001)
        .await
        .expect("Failed to add node1");

    let mut node2_id = [0u8; 20];
    node2_id[0] = 128; // Half-way around the ring
    let node2 = harness
        .add_node(node2_id, 8002, 9002)
        .await
        .expect("Failed to add node2");

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

    println!("=== Initial state after connection ===");
    let n1 = harness.get_node(&node1).await.unwrap();
    let n2 = harness.get_node(&node2).await.unwrap();

    let n1_successor = n1.get_successor().await;
    let n2_successor = n2.get_successor().await;
    let n1_predecessor = n1.get_predecessor().await;
    let n2_predecessor = n2.get_predecessor().await;

    println!(
        "Node1: successor={:?}, predecessor={:?}",
        n1_successor.as_ref().map(|s| format!("{:02x}...", s.0[0])),
        n1_predecessor
            .as_ref()
            .map(|p| format!("{:02x}...", p.0[0]))
    );
    println!(
        "Node2: successor={:?}, predecessor={:?}",
        n2_successor.as_ref().map(|s| format!("{:02x}...", s.0[0])),
        n2_predecessor
            .as_ref()
            .map(|p| format!("{:02x}...", p.0[0]))
    );

    // Now manually trigger stabilization rounds and track progress
    for round in 0..20 {
        println!("\n=== Stabilization round {} ===", round + 1);

        // Trigger stabilization
        harness
            .trigger_stabilization_round()
            .await
            .expect("Stabilization failed");

        // Check state after each round
        let n1_successor = n1.get_successor().await;
        let n2_successor = n2.get_successor().await;
        let n1_predecessor = n1.get_predecessor().await;
        let n2_predecessor = n2.get_predecessor().await;

        println!(
            "Node1: successor={:?}, predecessor={:?}",
            n1_successor.as_ref().map(|s| format!("{:02x}...", s.0[0])),
            n1_predecessor
                .as_ref()
                .map(|p| format!("{:02x}...", p.0[0]))
        );
        println!(
            "Node2: successor={:?}, predecessor={:?}",
            n2_successor.as_ref().map(|s| format!("{:02x}...", s.0[0])),
            n2_predecessor
                .as_ref()
                .map(|p| format!("{:02x}...", p.0[0]))
        );

        // Check if ring is formed
        let ring_complete = n1_successor.is_some()
            && n2_successor.is_some()
            && n1_predecessor.is_some()
            && n2_predecessor.is_some();

        if ring_complete {
            if let (
                Some((_, n1_succ, _)),
                Some((_, n2_succ, _)),
                Some((_, n1_pred, _)),
                Some((_, n2_pred, _)),
            ) = (
                &n1_successor,
                &n2_successor,
                &n1_predecessor,
                &n2_predecessor,
            ) {
                let proper_ring = n1_succ == &node2
                    && n2_succ == &node1
                    && n1_pred == &node2
                    && n2_pred == &node1;
                if proper_ring {
                    println!("✓ Perfect 2-node ring formed after {} rounds!", round + 1);
                    break;
                }
            }
        }

        // Small delay
        harness.timing().sleep(Duration::from_millis(100)).await;
    }
}
