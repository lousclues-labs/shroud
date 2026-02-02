//! Configuration module
//!
//! Provides persistent configuration storage for user preferences.

pub mod settings;

pub use settings::{
    AllowedClients, Config, ConfigManager, DnsMode, GatewayConfig, HeadlessConfig, Ipv6Mode,
};

// Re-export KillSwitchConfig for when it's needed
#[allow(unused_imports)]
pub use settings::KillSwitchConfig;
