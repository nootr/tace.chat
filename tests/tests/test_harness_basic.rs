//! Basic test harness functionality tests
//! Verifies core test harness components like node creation, network simulation, etc.
use tace_integration_tests::integration::TestHarness;
use tace_node::NetworkClient;

#[tokio::test]
async fn test_harness_creation() {
    let harness = TestHarness::new();
    let addresses = harness.get_all_node_addresses().await;
    assert!(addresses.is_empty());
}

#[tokio::test]
async fn test_node_creation() {
    let mut harness = TestHarness::new();

    // Create a single node
    let node_address = harness
        .add_node([1; 20], 8001, 9001)
        .await
        .expect("Failed to add node");

    // Verify it was added
    let addresses = harness.get_all_node_addresses().await;
    assert_eq!(addresses.len(), 1);
    assert_eq!(addresses[0], node_address);

    // Verify we can get the node
    let node = harness.get_node(&node_address).await;
    assert!(node.is_some());
}

#[tokio::test]
async fn test_network_simulator() {
    use tace_integration_tests::integration::NetworkSimulator;
    use tace_lib::dht_messages::DhtMessage;
    use tokio::sync::mpsc;

    let simulator = NetworkSimulator::new();

    // Register a node
    let (tx, mut rx) = mpsc::unbounded_channel();
    simulator
        .register_node("127.0.0.1:8001".to_string(), tx)
        .await;

    // Check if node is registered
    assert!(simulator.is_node_registered("127.0.0.1:8001").await);

    // Create client and test basic ping
    let client = simulator.create_client("127.0.0.1:8000".to_string());
    tokio::spawn(async move {
        if let Some(sim_message) = rx.recv().await {
            if let tace_integration_tests::integration::SimulatorMessage::Request {
                from,
                message,
                request_id: _,
                response_sender,
            } = sim_message
            {
                println!("Received message from {}: {:?}", from, message);
                // Echo back a pong for ping
                match message {
                    DhtMessage::Ping => {
                        let _ = response_sender.send(DhtMessage::Pong);
                    }
                    _ => {
                        let _ = response_sender.send(DhtMessage::Error {
                            message: "Unknown message".to_string(),
                        });
                    }
                }
            }
        }
    });

    let response = client.call_node("127.0.0.1:8001", DhtMessage::Ping).await;
    assert!(response.is_ok());

    match response.unwrap() {
        DhtMessage::Pong => println!("Ping successful!"),
        _ => panic!("Expected Pong response"),
    }
}
