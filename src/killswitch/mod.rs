//! Kill switch module
//!
//! Provides VPN kill switch functionality using nftables.
//! When enabled, blocks all traffic except through the VPN tunnel.

pub mod firewall;

pub use firewall::KillSwitch;
