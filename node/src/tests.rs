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
        };
        finger_table_vec[1] = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000002"),
            address: "127.0.0.1:8002".to_string(),
        };
        finger_table_vec[M - 1] = NodeInfo {
            id: hex_to_node_id("7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
            address: "127.0.0.1:8003".to_string(),
        };

        let node = ChordNode {
            info: node_info.clone(),
            successor: Arc::new(Mutex::new(node_info.clone())),
            predecessor: Arc::new(Mutex::new(None)),
            finger_table: Arc::new(Mutex::new(finger_table_vec)),
            data: Arc::new(Mutex::new(HashMap::new())),
            network_client: mock_network_client,
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
            address: dead_predecessor_address,
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
        };
        // Manually set predecessor to something that n_prime_3 is between
        let mut predecessor = node.predecessor.lock().unwrap();
        *predecessor = Some(NodeInfo {
            id: hex_to_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
            address: "127.0.0.1:9004".to_string(),
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
                })
            });

        let node =
            ChordNode::new_for_test("127.0.0.1:9000".to_string(), Arc::new(mock_network_client));
        let finger_node_id = hex_to_node_id("2222222222222222222222222222222222222222");
        let finger_node_address = "127.0.0.1:8001".to_string();
        let mut finger_table = node.finger_table.lock().unwrap();
        finger_table[0] = NodeInfo {
            id: finger_node_id,
            address: finger_node_address,
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
            address: successor_address,
        };

        node.stabilize().await;

        let new_successor = node.successor.lock().unwrap();
        assert_eq!(new_successor.id, successor_predecessor_id);
        assert_eq!(new_successor.address, successor_predecessor_address);
    }
}
