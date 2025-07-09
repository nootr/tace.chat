// Shared Rust library
pub mod dht_messages;

// Checks if an ID is between two other IDs in a circular ID space.
// `id` is between `start` and `end` if `start < id <= end` in the circular space.
pub fn is_between(
    id: &dht_messages::NodeId,
    start: &dht_messages::NodeId,
    end: &dht_messages::NodeId,
) -> bool {
    if start == end {
        // If start and end are the same, the interval (start, end] is empty
        return false;
    }
    if start < end {
        start < id && id <= end
    } else {
        // Wraps around (start > end)
        start < id || id <= end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dht_messages::NodeId;

    fn hex_to_node_id(hex_str: &str) -> NodeId {
        let mut id = [0u8; 20];
        let bytes = hex::decode(hex_str).unwrap();
        // Pad with leading zeros if necessary
        let start_index = 20 - bytes.len();
        id[start_index..].copy_from_slice(&bytes);
        id
    }

    #[test]
    fn test_is_between_no_wrap() {
        let start = hex_to_node_id("0000000000000000000000000000000000000000");
        let end = hex_to_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");
        let id_in = hex_to_node_id("8000000000000000000000000000000000000000");
        let id_out = hex_to_node_id("0000000000000000000000000000000000000000"); // Should be false because exclusive of start

        assert!(is_between(&id_in, &start, &end));
        assert!(!is_between(&id_out, &start, &end));
        assert!(is_between(&end, &start, &end)); // Inclusive of end
    }

    #[test]
    fn test_is_between_wrap_around() {
        let start = hex_to_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"); // Max ID
        let end = hex_to_node_id("0000000000000000000000000000000000000000"); // Min ID (wraps around)

        // Test cases for (FFFF..., 0000...] (only 0000... is in this interval)
        let id_in_end = hex_to_node_id("0000000000000000000000000000000000000000"); // Equal to end
        let id_out_start = hex_to_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"); // Equal to start

        assert!(is_between(&id_in_end, &start, &end)); // Should be true
        assert!(!is_between(&id_out_start, &start, &end)); // Should be false

        // Another wrap-around example: (0x80..., 0x20...] (using 20-byte IDs)
        let start2 = hex_to_node_id("8000000000000000000000000000000000000000");
        let end2 = hex_to_node_id("2000000000000000000000000000000000000000");
        let id_in_between = hex_to_node_id("9000000000000000000000000000000000000000"); // Example ID in the interval
        let id_in_other_side = hex_to_node_id("1000000000000000000000000000000000000000"); // Example ID in the interval

        assert!(is_between(&id_in_between, &start2, &end2));
        assert!(is_between(&id_in_other_side, &start2, &end2));
    }
}
