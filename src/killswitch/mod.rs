//! Kill switch module
//!
//! Provides VPN kill switch functionality using iptables.
//! When enabled, blocks all traffic except through the VPN tunnel.

pub mod cleanup;
pub mod firewall;

pub use cleanup::{cleanup_stale_on_startup, cleanup_with_fallback, CleanupResult};
pub use firewall::KillSwitch;
