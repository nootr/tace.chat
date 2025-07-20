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

pub mod network_simulator;
pub mod test_harness;
pub mod timing_control;
pub mod invariants;
pub mod scenarios;

pub use network_simulator::{NetworkSimulator, SimulatedNetworkClient, SimulatorMessage, RequestId};
pub use test_harness::TestHarness;
pub use timing_control::TimingController;
pub use invariants::{NetworkInvariants, InvariantViolation};
pub use scenarios::TestScenarios;