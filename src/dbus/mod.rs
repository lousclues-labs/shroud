//! D-Bus module for NetworkManager integration
//!
//! Provides real-time VPN state change notifications via D-Bus signals,
//! replacing polling-based state detection for faster response times.

pub mod monitor;

pub use monitor::{NmEvent, NmMonitor};
