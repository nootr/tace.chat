# DHT Node Integration Testing Framework

A comprehensive testing framework for distributed hash table (DHT) node networks that simulates real-world conditions without requiring actual network infrastructure.

## Overview

This integration testing framework provides:

- **Multi-node network simulation** - Test with dozens of nodes simultaneously
- **Controllable timing** - Step through operations or speed up/slow down time
- **Network simulation** - Route DHT messages between nodes without TCP overhead
- **Invariant validation** - Continuously verify network correctness properties
- **Fault injection** - Simulate node failures, network partitions, and message drops
- **Real production code testing** - Uses actual ChordNode implementation

## Architecture

```
tests/
├── integration/
│   ├── mod.rs                 # Module exports and documentation
│   ├── network_simulator.rs   # Simulated network communication
│   ├── test_harness.rs       # Multi-node test orchestration
│   ├── timing_control.rs     # Controllable timing system
│   ├── invariants.rs         # Network correctness validation
│   └── scenarios.rs          # Common test scenarios
├── integration_tests.rs      # Example integration tests
└── README.md                 # This file
```

## Key Components

### NetworkSimulator
Simulates network communication between nodes:
- Routes DHT messages without actual TCP connections
- Supports message dropping, latency simulation, and node failures
- Provides `NetworkClient` implementation for production code

### TestHarness
Orchestrates multi-node tests:
- Creates and manages multiple ChordNode instances
- Handles node joining/leaving operations
- Provides data storage/retrieval testing
- Integrates with timing control and network simulation

### TimingController
Controls test timing and synchronization:
- Manual mode: Step through operations explicitly
- Time multiplier: Speed up or slow down operations
- Barriers: Synchronize multiple concurrent operations
- Condition waiting: Wait for specific network states

### NetworkInvariants
Validates network correctness:
- Ring connectivity: All nodes reachable via successors
- Successor/predecessor consistency
- Data availability and consistency
- Load balancing
- Fault tolerance

## Network Invariants

The framework continuously validates these critical properties:

1. **Ring Connectivity**: Every node should be reachable from every other node by following successors
2. **Successor Consistency**: For each node n, successor(n).predecessor should point back to n
3. **Predecessor Consistency**: For each node n, predecessor(n).successor should point to n
4. **Data Availability**: All stored data should remain retrievable from the network
5. **Data Consistency**: The same key should return the same value from any node
6. **Load Balance**: Data should be reasonably distributed across nodes
7. **Fault Tolerance**: Network should remain functional after node failures

## Usage Examples

### Basic Network Test

```rust
use integration::{TestHarness, NetworkInvariants};

#[tokio::test]
async fn test_basic_network() {
    let mut harness = TestHarness::new();

    // Create 3 nodes
    let node1 = harness.add_node([1; 20], 8001, 9001).await?;
    let node2 = harness.add_node([2; 20], 8002, 9002).await?;
    let node3 = harness.add_node([3; 20], 8003, 9003).await?;

    harness.start_all_nodes().await?;

    // Form network
    harness.connect_node_to_network(&node1, None).await?;
    harness.connect_node_to_network(&node2, Some(&node1)).await?;
    harness.connect_node_to_network(&node3, Some(&node1)).await?;

    // Wait for stabilization
    harness.wait_for_stabilization(30).await?;

    // Verify network correctness
    let violations = NetworkInvariants::check_all(&harness).await;
    assert!(violations.is_empty());
}
```

### Data Operations Test

```rust
#[tokio::test]
async fn test_data_operations() {
    let mut harness = TestHarness::new();
    // ... setup network ...

    // Store data
    let key = [42; 20];
    let value = b"test data".to_vec();
    harness.store_data(&node1, key, value.clone()).await?;

    // Retrieve from different node
    let retrieved = harness.retrieve_data(&node2, key).await?;
    assert_eq!(retrieved, Some(vec![value]));
}
```

### Fault Tolerance Test

```rust
#[tokio::test]
async fn test_node_failure() {
    let mut harness = TestHarness::new();
    // ... setup network ...

    // Simulate node failure
    harness.fail_node(&node2).await?;

    // Verify network adapts
    harness.wait_for_stabilization(30).await?;
    let violations = NetworkInvariants::check_ring_connectivity(&harness).await;
    assert!(violations.is_empty());

    // Recover node
    harness.recover_node(&node2).await?;
}
```

### Timing Control

```rust
#[tokio::test]
async fn test_with_timing_control() {
    let mut harness = TestHarness::new();

    // Enable manual timing
    harness.timing().enable_manual_mode().await;

    // Operations now wait for explicit steps
    let task = tokio::spawn(async move {
        harness.timing().sleep(Duration::from_secs(10)).await;
    });

    // Step through manually
    harness.timing().step().await;
    task.await.unwrap(); // Completes immediately
}
```

## Running Tests

### Quick Tests
```bash
cargo test --test integration_tests
```

### Include Long-Running Tests
```bash
cargo test --test integration_tests -- --ignored
```

### Specific Test Categories
```bash
# Basic functionality
cargo test test_basic_network_formation

# Fault tolerance
cargo test test_fault_tolerance

# Scalability (may take time)
cargo test test_scalability -- --ignored
```

## Test Scenarios

The framework includes several pre-built test scenarios:

1. **Basic Network Formation** - Simple 3-node network setup and verification
2. **Dynamic Membership** - Nodes joining and leaving dynamically
3. **Data Operations** - Store/retrieve operations across the network
4. **Fault Tolerance** - Network resilience to node failures
5. **Scalability** - Behavior with large numbers of nodes
6. **Partition Healing** - Recovery from network splits

### Running Scenario Suites

```rust
// Run basic test suite
TestScenarios::run_basic_test_suite().await?;

// Run comprehensive tests (longer)
TestScenarios::run_comprehensive_test_suite().await?;

// Run specific scenario
TestScenarios::fault_tolerance_test().await?;
```

## Configuration

### Environment Variables

- `INTEGRATION_TEST_LOG_LEVEL` - Set logging level (debug, info, warn, error)
- `INTEGRATION_TEST_TIMEOUT` - Default timeout for operations (seconds)
- `INTEGRATION_TEST_SPEED` - Time multiplier for faster tests

### Test Customization

```rust
// Speed up tests
harness.timing().set_time_multiplier(5.0).await;

// Add network latency
harness.network().set_latency(100).await; // 100ms

// Simulate message drops
harness.network().set_drop_rate(0.1).await; // 10% drop rate
```

## Advanced Features

### Custom Invariants

Add your own invariants to validate specific properties:

```rust
impl NetworkInvariants {
    pub async fn check_custom_property(harness: &TestHarness) -> Vec<InvariantViolation> {
        // Your custom validation logic
        vec![]
    }
}
```

### Message Interception

Monitor DHT messages for debugging:

```rust
let simulator = harness.network();
// Add message logging or modification
```

### Performance Metrics

Track network performance during tests:

```rust
for node_addr in harness.get_all_node_addresses().await {
    if let Some(node) = harness.get_node(&node_addr).await {
        let metrics = node.get_metrics().await;
        println!("Node {}: {} items, {} messages", node_addr, metrics.data_items, metrics.messages_sent);
    }
}
```

## Best Practices

1. **Start Small** - Begin with 3-5 nodes, then scale up
2. **Check Invariants** - Always validate network properties after operations
3. **Use Timing Control** - Speed up tests but maintain determinism
4. **Test Edge Cases** - Rapid joins/leaves, concurrent operations, failures
5. **Monitor Resources** - Large tests can consume significant memory
6. **Clean Shutdown** - Always properly stop nodes to avoid resource leaks

## Troubleshooting

### Common Issues

**Tests hang indefinitely**
- Check if manual timing mode is enabled when not intended
- Verify all nodes are properly started
- Look for deadlocks in stabilization

**Invariant violations**
- Check network formation order
- Ensure sufficient stabilization time
- Verify node IDs are unique and well-distributed

**Memory usage grows**
- Limit test size or run sequentially
- Check for resource leaks in test cleanup
- Use timing acceleration to reduce test duration

### Debugging

```rust
// Enable detailed logging
env_logger::builder()
    .filter_level(log::LevelFilter::Debug)
    .init();

// Check specific invariants
let violations = NetworkInvariants::check_invariant(&harness, "ring_connectivity").await;
println!("Violations: {:?}", violations);
```

## Contributing

When adding new test scenarios:

1. Follow the existing pattern in `scenarios.rs`
2. Add appropriate invariant checks
3. Include both positive and negative test cases
4. Document expected behavior and timing requirements
5. Consider both small and large network scales

## Future Enhancements

- **Visual Network Monitoring** - Real-time network topology display
- **Performance Benchmarking** - Automated performance regression testing
- **Chaos Engineering** - Random failure injection during operations
- **Protocol Verification** - Formal verification of protocol properties
- **Load Testing** - High-throughput data operation testing
