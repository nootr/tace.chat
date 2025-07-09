#[cfg(test)]
mod tests {
    use crate::{ChordNode, NodeInfo, M};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::test;
    use wisp_lib::dht_messages::NodeId;

    fn hex_to_node_id(hex_str: &str) -> NodeId {
        let mut id = [0u8; 20];
        let bytes = hex::decode(hex_str).unwrap();
        // Pad with leading zeros if necessary
        let start_index = 20 - bytes.len();
        id[start_index..].copy_from_slice(&bytes);
        id
    }

    #[test]
    async fn test_start_new_network() {
        let node = ChordNode::new("127.0.0.1:9000".to_string()).await;
        node.start_new_network().await;

        let successor = node.successor.lock().unwrap();
        assert_eq!(successor.id, node.info.id);
        assert_eq!(successor.address, node.info.address);

        let predecessor = node.predecessor.lock().unwrap();
        assert!(predecessor.is_none());
    }

    #[test]
    async fn test_closest_preceding_node() {
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

    #[test]
    async fn test_store_and_retrieve() {
        let node = ChordNode::new("127.0.0.1:9000".to_string()).await;
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

    #[test]
    async fn test_check_predecessor_dead() {
        let node = ChordNode::new("127.0.0.1:9000".to_string()).await;
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

    #[test]
    async fn test_notify() {
        let node_id = hex_to_node_id("0000000000000000000000000000000000000000");
        let node_address = "127.0.0.1:9000".to_string();
        let node = ChordNode::new(node_address.clone()).await;

        // Case 1: Predecessor is nil
        let n_prime_1 = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000001"),
            address: "127.0.0.1:9001".to_string(),
        };
        node.notify(n_prime_1.clone()).await;
        assert_eq!(node.predecessor.lock().unwrap().clone().unwrap().id, n_prime_1.id);

        // Case 2: n_prime is between current predecessor and self
        let n_prime_2 = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000000"), // Same as node_id, should not update
            address: "127.0.0.1:9002".to_string(),
        };
        node.notify(n_prime_2.clone()).await;
        // Predecessor should still be n_prime_1 because n_prime_2 is not between n_prime_1 and node
        assert_eq!(node.predecessor.lock().unwrap().clone().unwrap().id, n_prime_1.id);

        let n_prime_3 = NodeInfo {
            id: hex_to_node_id("0000000000000000000000000000000000000000"), // This should be between n_prime_1 and node_id
            address: "127.0.0.1:9003".to_string(),
        };
        // Manually set predecessor to something that n_prime_3 is between
        *node.predecessor.lock().unwrap() = Some(hex_to_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF").into());
        node.notify(n_prime_3.clone()).await;
        assert_eq!(node.predecessor.lock().unwrap().clone().unwrap().id, n_prime_3.id);
    }
}
