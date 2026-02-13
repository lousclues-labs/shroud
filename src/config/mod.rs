//! Configuration module
//!
//! Provides persistent configuration storage for user preferences.

pub mod settings;

pub use settings::{Config, ConfigManager, DnsMode, HeadlessConfig, Ipv6Mode};

// Re-export KillSwitchConfig for when it's needed
#[allow(unused_imports)]
pub use settings::KillSwitchConfig;
