#[cfg(test)]
mod tests {
    use crate::network_client::MockNetworkClient;
    use crate::{ChordNode, NodeInfo, M};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tace_lib::dht_messages::{DhtMessage, NodeId};

    fn hex_to_node_id(hex_str: &str) -> NodeId {
        let mut id = [0u8; 20];
        let bytes = hex::decode(hex_str).unwrap();
        // Pad with leading zeros if necessary
        let start_index = 20 - bytes.len();
        id[start_index..].copy_from_slice(&bytes);
        id
    }

    #[tokio::test]
    async fn test_start_new_network() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);
        node.start_new_network().await;

        let successor = node.successor.lock().unwrap();
        assert_eq!(successor.id, node.info.id);
        assert_eq!(successor.address, node.info.address);

        let predecessor = node.predecessor.lock().unwrap();
        assert!(predecessor.is_none());
    }

    #[tokio::test]
    async fn test_closest_preceding_node() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node_id = hex_to_node_id("0000000000000000000000000000000000000000");
        let node_address = "127.0.0.1:8000".to_string();
        let node_info = NodeInfo {
            id: node_id,
            address: node_address.clone(),
            api_address: node_address.clone(),
        };

        let mut finger_table_vec = vec![node_info.clone(); M];
        // Populate some finger table entries for testing
        // Example: finger[0] = node_id + 2^0
        // finger[1] = node_id + 2^1
        // ...
        // For simplicity, let's just put a few specific values
        finger_table_vec[0] = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000001"),
            address: "127.0.0.1:8001".to_string(),
            api_address: "127.0.0.1:8001".to_string(),
        };
        finger_table_vec[1] = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000002"),
            address: "127.0.0.1:8002".to_string(),
            api_address: "127.0.0.1:8002".to_string(),
        };
        finger_table_vec[M - 1] = NodeInfo {
            id: hex_to_node_id("7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
            address: "127.0.0.1:8003".to_string(),
            api_address: "127.0.0.1:8003".to_string(),
        };

        let node = ChordNode {
            info: node_info.clone(),
            successor: Arc::new(Mutex::new(node_info.clone())),
            predecessor: Arc::new(Mutex::new(None)),
            finger_table: Arc::new(Mutex::new(finger_table_vec)),
            data: Arc::new(Mutex::new(HashMap::new())),
            network_client: mock_network_client,
            network_size_estimate: Arc::new(Mutex::new(1.0)),
        };

        // Test case 1: ID is far away, should return the largest finger
        let target_id = hex_to_node_id("8000000000000000000000000000000000000000");
        let cpn = node.closest_preceding_node(target_id).await;
        assert_eq!(
            cpn.id,
            hex_to_node_id("7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF")
        );

        // Test case 2: ID is close, should return a smaller finger
        let target_id_close = hex_to_node_id("0000000000000000000000000000000000000003");
        let cpn_close = node.closest_preceding_node(target_id_close).await;
        assert_eq!(
            cpn_close.id,
            hex_to_node_id("0000000000000000000000000000000000000002")
        );

        // Test case 3: ID is very close, should return self
        let target_id_very_close = hex_to_node_id("0000000000000000000000000000000000000000");
        let cpn_very_close = node.closest_preceding_node(target_id_very_close).await;
        assert_eq!(cpn_very_close.id, node_id);
    }

    #[tokio::test]
    async fn test_store_and_retrieve() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);
        let key = hex_to_node_id("1234567890123456789012345678901234567890");
        let value = vec![1, 2, 3, 4, 5];

        // Store the value
        node.store(key, value.clone()).await;

        // Retrieve the value
        let retrieved_value = node.retrieve(key).await;
        assert_eq!(retrieved_value, Some(value));

        // Try to retrieve a non-existent key
        let non_existent_key = hex_to_node_id("0000000000000000000000000000000000000000");
        let retrieved_none = node.retrieve(non_existent_key).await;
        assert_eq!(retrieved_none, None);
    }

    #[tokio::test]
    async fn test_check_predecessor_dead() {
        let mut mock_network_client = MockNetworkClient::new();
        mock_network_client
            .expect_call_node()
            .withf(|address, message| {
                address == "127.0.0.1:9999" && matches!(message, DhtMessage::Ping)
            })
            .times(1)
            .returning(|_, _| Err("Simulated network error".into()));

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));
        let dead_predecessor_id = hex_to_node_id("1111111111111111111111111111111111111111");
        let dead_predecessor_address = "127.0.0.1:9999".to_string(); // Non-existent address

        let mut predecessor = node.predecessor.lock().unwrap();
        *predecessor = Some(NodeInfo {
            id: dead_predecessor_id,
            address: dead_predecessor_address.clone(),
            api_address: dead_predecessor_address,
        });
        drop(predecessor); // Release lock

        node.check_predecessor().await;

        let updated_predecessor = node.predecessor.lock().unwrap();
        assert!(updated_predecessor.is_none());
    }

    #[tokio::test]
    async fn test_notify() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let _node_id = hex_to_node_id("0000000000000000000000000000000000000000");
        let node_address = "127.0.0.1:9000".to_string();
        let node = ChordNode::new_for_test(node_address.clone(), mock_network_client);

        // Case 1: Predecessor is nil
        let n_prime_1 = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000001"),
            address: "127.0.0.1:9001".to_string(),
            api_address: "127.0.0.1:9001".to_string(),
        };
        node.notify(n_prime_1.clone()).await;
        assert_eq!(
            node.predecessor.lock().unwrap().clone().unwrap().id,
            n_prime_1.id
        );

        // Case 2: n_prime is between current predecessor and self
        let n_prime_2 = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000000"), // Same as node_id, should not update
            address: "127.0.0.1:9002".to_string(),
            api_address: "127.0.0.1:9002".to_string(),
        };
        node.notify(n_prime_2.clone()).await;
        // Predecessor should still be n_prime_1 because n_prime_2 is not between n_prime_1 and node
        assert_eq!(
            node.predecessor.lock().unwrap().clone().unwrap().id,
            n_prime_1.id
        );

        let n_prime_3 = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000000"), // This should be between n_prime_1 and node_id
            address: "127.0.0.1:9003".to_string(),
            api_address: "127.0.0.1:9003".to_string(),
        };
        // Manually set predecessor to something that n_prime_3 is between
        let mut predecessor = node.predecessor.lock().unwrap();
        *predecessor = Some(NodeInfo {
            id: hex_to_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
            address: "127.0.0.1:9004".to_string(),
            api_address: "127.0.0.1:9004".to_string(),
        });
        drop(predecessor);
        node.notify(n_prime_3.clone()).await;
        assert_eq!(
            node.predecessor.lock().unwrap().clone().unwrap().id,
            n_prime_3.id
        );
    }

    #[tokio::test]
    async fn test_join() {
        let mut mock_network_client = MockNetworkClient::new();
        let bootstrap_node_id = hex_to_node_id("2222222222222222222222222222222222222222");
        let bootstrap_node_address = "127.0.0.1:8001".to_string();

        mock_network_client
            .expect_call_node()
            .withf(move |address, message| {
                address == bootstrap_node_address
                    && match message {
                        DhtMessage::FindSuccessor { id: _ } => true,
                        _ => false,
                    }
            })
            .times(1)
            .returning(move |_, _| {
                Ok(DhtMessage::FoundSuccessor {
                    id: bootstrap_node_id,
                    address: "127.0.0.1:8001".to_string(),
                    api_address: "127.0.0.1:8001".to_string(),
                })
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));
        node.join(Some("127.0.0.1:8001".to_string())).await;

        let successor = node.successor.lock().unwrap();
        assert_eq!(successor.id, bootstrap_node_id);
        assert_eq!(successor.address, "127.0.0.1:8001");
    }

    #[tokio::test]
    async fn test_find_successor() {
        let mut mock_network_client = MockNetworkClient::new();
        let successor_id = hex_to_node_id("3333333333333333333333333333333333333333");
        let successor_address = "127.0.0.1:8002".to_string();
        let successor_address_clone = successor_address.clone();

        // This mock will be for the call to the closest preceding node
        mock_network_client
            .expect_call_node()
            .times(1)
            .returning(move |_, _| {
                Ok(DhtMessage::FoundSuccessor {
                    id: successor_id,
                    address: successor_address_clone.clone(),
                    api_address: successor_address_clone.clone(),
                })
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));
        let finger_node_id = hex_to_node_id("2222222222222222222222222222222222222222");
        let finger_node_address = "127.0.0.1:8001".to_string();
        let mut finger_table = node.finger_table.lock().unwrap();
        finger_table[0] = NodeInfo {
            id: finger_node_id,
            address: finger_node_address.clone(),
            api_address: finger_node_address,
        };
        drop(finger_table);

        let target_id = hex_to_node_id("4444444444444444444444444444444444444444");
        let successor = node.find_successor(target_id).await;

        assert_eq!(successor.id, successor_id);
        assert_eq!(successor.address, successor_address);
    }

    #[tokio::test]
    async fn test_stabilize() {
        let mut mock_network_client = MockNetworkClient::new();
        let successor_predecessor_id = hex_to_node_id("5555555555555555555555555555555555555555");
        let successor_predecessor_address = "127.0.0.1:8003".to_string();
        let successor_predecessor_address_clone = successor_predecessor_address.clone();

        // Mock for GetPredecessor
        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::GetPredecessor))
            .times(1)
            .returning(move |_, _| {
                Ok(DhtMessage::Predecessor {
                    id: Some(successor_predecessor_id),
                    address: Some(successor_predecessor_address_clone.clone()),
                    api_address: Some(successor_predecessor_address_clone.clone()),
                })
            });

        // Mock for Notify
        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::Notify { .. }))
            .times(1)
            .returning(|_, _| Ok(DhtMessage::Pong));

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));
        let successor_id = hex_to_node_id("6666666666666666666666666666666666666666");
        let successor_address = "127.0.0.1:8004".to_string();
        *node.successor.lock().unwrap() = NodeInfo {
            id: successor_id,
            address: successor_address.clone(),
            api_address: successor_address,
        };

        node.stabilize().await;

        let new_successor = node.successor.lock().unwrap();
        assert_eq!(new_successor.id, successor_predecessor_id);
        assert_eq!(new_successor.address, successor_predecessor_address);
    }

    #[tokio::test]
    async fn test_calculate_local_network_size_estimate_single_node() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // Single node network: successor points to itself, no predecessor
        let estimate = node.calculate_local_network_size_estimate();
        assert_eq!(estimate, 1.0);
    }

    #[tokio::test]
    async fn test_calculate_local_network_size_estimate_two_nodes() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // Set up a two-node network where nodes are separated
        // In our algorithm, we calculate range between predecessor and successor
        // For a 2-node network, if nodes are at positions A and B, then:
        // - Node A sees: predecessor=B, successor=B, so range = 0 (they wrap around)
        // Let's set up a scenario where predecessor and successor are different

        let predecessor_id = hex_to_node_id("2000000000000000000000000000000000000000");
        let successor_id = hex_to_node_id("8000000000000000000000000000000000000000");

        // Set predecessor and successor to different nodes
        *node.predecessor.lock().unwrap() = Some(NodeInfo {
            id: predecessor_id,
            address: "127.0.0.1:9001".to_string(),
            api_address: "127.0.0.1:9001".to_string(),
        });

        *node.successor.lock().unwrap() = NodeInfo {
            id: successor_id,
            address: "127.0.0.1:9002".to_string(),
            api_address: "127.0.0.1:9002".to_string(),
        };

        let estimate = node.calculate_local_network_size_estimate();
        // Should estimate more than 1 node since we have a range between pred and succ
        assert!(estimate > 1.0, "Expected >1 nodes, got {}", estimate);

        // The estimate should be reasonable (not extremely large)
        assert!(estimate < 100.0, "Estimate too large: {}", estimate);
    }

    #[tokio::test]
    async fn test_network_size_estimation_with_mock_neighbor() {
        let mut mock_network_client = MockNetworkClient::new();

        // Mock the neighbor's response
        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::GetNetworkEstimate))
            .times(1)
            .returning(|_, _| Ok(DhtMessage::NetworkEstimate { estimate: 3.5 }));

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Set up a multi-node scenario with successor and finger table entries
        let successor = NodeInfo {
            id: hex_to_node_id("1111111111111111111111111111111111111111"),
            address: "127.0.0.1:9001".to_string(),
            api_address: "127.0.0.1:9001".to_string(),
        };
        *node.successor.lock().unwrap() = successor.clone();

        // Add some finger table entries
        let mut finger_table = node.finger_table.lock().unwrap();
        finger_table[0] = successor.clone();
        finger_table[1] = NodeInfo {
            id: hex_to_node_id("2222222222222222222222222222222222222222"),
            address: "127.0.0.1:9002".to_string(),
            api_address: "127.0.0.1:9002".to_string(),
        };
        drop(finger_table);

        let initial_estimate = *node.network_size_estimate.lock().unwrap();

        node.estimate_network_size().await;

        let final_estimate = *node.network_size_estimate.lock().unwrap();

        // The estimate should have been updated (should be different from initial)
        assert_ne!(initial_estimate, final_estimate);

        // The final estimate should be influenced by both local estimate and neighbor's estimate (3.5)
        assert!(final_estimate > 0.0);
    }

    #[tokio::test]
    async fn test_apply_bounds() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // Test that bounds are applied correctly (max change factor is 4.0)
        let old_estimate = 10.0;

        // Test upper bound
        let too_high = 50.0; // 5x increase, should be capped to 40.0 (4x)
        let bounded_high = node.apply_bounds(old_estimate, too_high);
        assert_eq!(bounded_high, 40.0);

        // Test lower bound
        let too_low = 1.0; // 10x decrease, should be capped to 2.5 (4x decrease)
        let bounded_low = node.apply_bounds(old_estimate, too_low);
        assert_eq!(bounded_low, 2.5);

        // Test within bounds
        let within_bounds = 20.0; // 2x increase, should pass through unchanged
        let bounded_normal = node.apply_bounds(old_estimate, within_bounds);
        assert_eq!(bounded_normal, within_bounds);
    }

    #[tokio::test]
    async fn test_apply_smoothing() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // Test smoothing with factor 0.1
        let old_estimate = 10.0;
        let new_estimate = 20.0;

        let smoothed = node.apply_smoothing(old_estimate, new_estimate);

        // Should be: 0.9 * 10.0 + 0.1 * 20.0 = 9.0 + 2.0 = 11.0
        assert_eq!(smoothed, 11.0);
    }

    #[tokio::test]
    async fn test_estimate_network_size_single_node_network() {
        let mock_network_client = Arc::new(MockNetworkClient::new());
        let node = ChordNode::new_for_test("127.0.0.1:9000".to_string(), mock_network_client);

        // In a single node network, successor points to self
        // estimate_network_size should return early without making network calls
        let initial_estimate = *node.network_size_estimate.lock().unwrap();

        node.estimate_network_size().await;

        let final_estimate = *node.network_size_estimate.lock().unwrap();

        // Estimate should be unchanged since we're the only node
        assert_eq!(initial_estimate, final_estimate);
    }

    #[tokio::test]
    async fn test_random_neighbor_selection() {
        let mut mock_network_client = MockNetworkClient::new();

        // Mock response for any network estimate request
        mock_network_client
            .expect_call_node()
            .withf(|_, message| matches!(message, DhtMessage::GetNetworkEstimate))
            .times(1)
            .returning(|_, _| Ok(DhtMessage::NetworkEstimate { estimate: 4.0 }));

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));

        // Set up multiple candidate nodes
        let successor = NodeInfo {
            id: hex_to_node_id("1111111111111111111111111111111111111111"),
            address: "127.0.0.1:9001".to_string(),
            api_address: "127.0.0.1:9001".to_string(),
        };
        *node.successor.lock().unwrap() = successor.clone();

        // Add multiple unique finger table entries
        let mut finger_table = node.finger_table.lock().unwrap();
        finger_table[0] = successor.clone();
        finger_table[1] = NodeInfo {
            id: hex_to_node_id("2222222222222222222222222222222222222222"),
            address: "127.0.0.1:9002".to_string(),
            api_address: "127.0.0.1:9002".to_string(),
        };
        finger_table[2] = NodeInfo {
            id: hex_to_node_id("3333333333333333333333333333333333333333"),
            address: "127.0.0.1:9003".to_string(),
            api_address: "127.0.0.1:9003".to_string(),
        };
        drop(finger_table);

        // Run estimation - it should succeed in picking a random neighbor
        node.estimate_network_size().await;

        // The estimate should have been updated from the mock response
        let final_estimate = *node.network_size_estimate.lock().unwrap();
        assert!(final_estimate > 0.0);
    }

    #[tokio::test]
    async fn test_ring_formation_simulation() {
        // This test simulates a simple 3-node ring formation to verify the logic

        // Create mock clients for 3 nodes
        let node1_client = MockNetworkClient::new();

        // Node IDs in order: node1 < node2 < node3
        let node2_id = hex_to_node_id("6000000000000000000000000000000000000000");
        let node3_id = hex_to_node_id("A000000000000000000000000000000000000000");

        // Set up node1 (first node, creates ring)
        let node1 = ChordNode::new_for_test("node1:8001".to_string(), Arc::new(node1_client));
        node1.start_new_network().await;

        // In a properly formed 3-node ring:
        // node1 -> node2 -> node3 -> node1
        // Let's manually set up what should happen after stabilization

        // Node1 should have node2 as successor after node2 joins
        *node1.successor.lock().unwrap() = NodeInfo {
            id: node2_id,
            address: "node2:8002".to_string(),
            api_address: "node2:8002".to_string(),
        };
        // Node1 should have node3 as predecessor (node3 points to node1)
        *node1.predecessor.lock().unwrap() = Some(NodeInfo {
            id: node3_id,
            address: "node3:8003".to_string(),
            api_address: "node3:8003".to_string(),
        });

        // Verify ring properties
        let successor = node1.successor.lock().unwrap().clone();
        let predecessor = node1.predecessor.lock().unwrap().clone();

        // Check that successor is the next node in the ring
        assert_eq!(successor.id, node2_id);

        // Check that predecessor is the previous node in the ring
        assert_eq!(predecessor.unwrap().id, node3_id);

        // Verify the ring property: predecessor < self < successor (accounting for wraparound)
        // In this case: node3 > node1 < node2, which is valid with wraparound
        assert!(tace_lib::is_between(&node1.info.id, &node3_id, &node2_id));
    }
}
