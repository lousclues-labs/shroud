//! Health check module
//!
//! Provides connectivity verification for VPN tunnels to detect degraded states.

pub mod checker;

pub use checker::{HealthChecker, HealthResult};
