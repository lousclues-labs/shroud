//! D-Bus module for NetworkManager integration
//!
//! Provides real-time VPN state change notifications via D-Bus signals,
//! replacing polling-based state detection for faster response times.

pub mod monitor;
#[allow(dead_code)]
pub mod types;

#[cfg(test)]
mod tests;

pub use monitor::{NmEvent, NmMonitor};
