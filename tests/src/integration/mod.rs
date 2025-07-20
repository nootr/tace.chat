//! Integration Testing Framework for DHT Node Network
//!
//! This module provides a comprehensive testing environment that simulates
//! a distributed network of DHT nodes without requiring actual network
//! infrastructure. It includes:
//!
//! - Multi-node network simulation
//! - Controllable timing and event scheduling  
//! - DHT message routing between simulated nodes
//! - Network invariant validation
//! - Real production code testing

pub mod invariants;
pub mod network_simulator;
pub mod scenarios;
pub mod test_harness;
pub mod timing_control;

pub use invariants::{InvariantViolation, NetworkInvariants};
pub use network_simulator::{
    NetworkSimulator, RequestId, SimulatedNetworkClient, SimulatorMessage,
};
pub use scenarios::TestScenarios;
pub use test_harness::TestHarness;
pub use timing_control::TimingController;
