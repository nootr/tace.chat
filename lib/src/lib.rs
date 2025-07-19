/// Shared Rust library
pub mod dht_messages;
pub mod keys;
pub mod metrics;

use num_bigint::BigUint;
use num_traits::identities::One;

/// Converts NodeId (byte array) to BigUint
pub fn node_id_to_biguint(id: &dht_messages::NodeId) -> BigUint {
    BigUint::from_bytes_be(id)
}

/// Converts BigUint to NodeId (byte array), padding with zeros if necessary
pub fn biguint_to_node_id(biguint: &BigUint) -> dht_messages::NodeId {
    let bytes = biguint.to_bytes_be();
    let mut id = [0u8; 20];
    let start_index = 20 - bytes.len();
    id[start_index..].copy_from_slice(&bytes);
    id
}

/// Calculates (id + 2^i) mod 2^M
pub fn add_id_power_of_2(id: &dht_messages::NodeId, i: usize) -> dht_messages::NodeId {
    let id_biguint = node_id_to_biguint(id);
    let two_pow_i = BigUint::one() << i;
    let two_pow_m = BigUint::one() << 160; // M = 160

    let result = (id_biguint + two_pow_i) % two_pow_m;
    biguint_to_node_id(&result)
}

/// Checks if an ID is between two other IDs in a circular ID space.
/// `id` is between `start` and `end` if `start < id <= end` in the circular space.
pub fn is_between(
    id: &dht_messages::NodeId,
    start: &dht_messages::NodeId,
    end: &dht_messages::NodeId,
) -> bool {
    let id_b = node_id_to_biguint(id);
    let start_b = node_id_to_biguint(start);
    let end_b = node_id_to_biguint(end);

    if start_b == end_b {
        // If start and end are the same, the interval (start, end] is empty
        return false;
    }
    if start_b < end_b {
        start_b < id_b && id_b <= end_b
    } else {
        // Wraps around (start > end)
        start_b < id_b || id_b <= end_b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dht_messages::NodeId;
    use num_bigint::BigUint;
    use num_traits::identities::Zero;

    fn hex_to_node_id(hex_str: &str) -> NodeId {
        let mut id = [0u8; 20];
        let bytes = hex::decode(hex_str).unwrap();
        // Pad with leading zeros if necessary
        let start_index = 20 - bytes.len();
        id[start_index..].copy_from_slice(&bytes);
        id
    }

    #[test]
    fn test_node_id_biguint_conversion() {
        let id_hex = "0123456789abcdef0123456789abcdef01234567";
        let node_id = hex_to_node_id(id_hex);
        let biguint = node_id_to_biguint(&node_id);
        let converted_node_id = biguint_to_node_id(&biguint);
        assert_eq!(node_id, converted_node_id);

        let zero_id = hex_to_node_id("0000000000000000000000000000000000000000");
        assert_eq!(node_id_to_biguint(&zero_id), BigUint::zero());

        let max_id = hex_to_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");
        let expected_max_biguint = (BigUint::one() << 160) - BigUint::one();
        assert_eq!(node_id_to_biguint(&max_id), expected_max_biguint);
    }

    #[test]
    fn test_add_id_power_of_2() {
        let id = hex_to_node_id("0000000000000000000000000000000000000000"); // 0
        let expected_id_1 = hex_to_node_id("0000000000000000000000000000000000000001"); // 1
        let expected_id_2 = hex_to_node_id("0000000000000000000000000000000000000002"); // 2
        let expected_id_4 = hex_to_node_id("0000000000000000000000000000000000000004"); // 4

        assert_eq!(add_id_power_of_2(&id, 0), expected_id_1);
        assert_eq!(add_id_power_of_2(&id, 1), expected_id_2);
        assert_eq!(add_id_power_of_2(&id, 2), expected_id_4);

        // Test wrap around
        let max_id = hex_to_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");
        let wrapped_id = add_id_power_of_2(&max_id, 0); // MAX_ID + 1 should be 0
        assert_eq!(
            wrapped_id,
            hex_to_node_id("0000000000000000000000000000000000000000")
        );
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
