//! System tray module
//!
//! Provides the system tray UI for the VPN manager.

pub mod icons;
pub mod service;

pub use service::{SharedState, VpnCommand, VpnTray};
