// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! Configuration settings
//!
//! Persistent storage for user preferences using TOML format.
//! Config file is stored in XDG_CONFIG_HOME/shroud/config.toml
//!
//! ## Config Schema (version 1)
//!
//! ```toml
//! version = 1
//! auto_reconnect = true
//! auto_connect = false
//! last_server = "us-east-1"  # optional
//! health_check_interval_secs = 30
//! health_degraded_threshold_ms = 5000
//! max_reconnect_attempts = 10
//! kill_switch_enabled = false
//!
//! # DNS leak protection mode: "tunnel" | "strict" | "localhost" | "any"
//! # - tunnel: DNS only via VPN tunnel interfaces (most secure, default)
//! # - strict: tunnel + DoH/DoT blocking (maximum protection)
//! # - localhost: DNS only to 127.0.0.0/8, ::1, 127.0.0.53 (for local resolvers)
//! # - any: DNS to any destination (legacy, least secure)
//! dns_mode = "tunnel"
//! # Block DNS-over-HTTPS to known providers (recommended)
//! block_doh = true
//! # Additional DoH provider IPs to block
//! custom_doh_blocklist = []
//!
//! # IPv6 leak protection: "block" | "tunnel" | "off"
//! # - block: Drop all IPv6 except loopback (most secure, default)
//! # - tunnel: Allow IPv6 only via VPN tunnel interfaces
//! # - off: No special IPv6 handling (legacy)
//! ipv6_mode = "block"
//! ```

use crate::cli::validation::validate_vpn_name;
use thiserror::Error;
use tracing::{debug, info, warn};

/// Errors that can occur during configuration operations.
#[derive(Error, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum ConfigError {
    /// Failed to parse config TOML
    #[error("Failed to parse config: {0}")]
    #[allow(dead_code)]
    Parse(#[from] toml::de::Error),

    /// Failed to serialize config
    #[error("Failed to serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),

    /// Failed to write config file
    #[error("Failed to write config file: {0}")]
    Write(#[source] std::io::Error),

    /// Failed to create config directory
    #[error("Failed to create config directory: {0}")]
    Directory(#[source] std::io::Error),

    /// Atomic rename failed
    #[error("Failed to save config (atomic rename): {0}")]
    Rename(#[source] std::io::Error),
}
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Current config version
const CONFIG_VERSION: u32 = 1;

/// DNS leak protection mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DnsMode {
    /// DNS only via VPN tunnel interfaces (tun*, wg*)
    /// Most secure - prevents DNS leaks entirely
    #[default]
    Tunnel,
    /// Maximum protection: tunnel DNS rules + DoH/DoT blocking
    /// Recommended for privacy-critical use
    Strict,
    /// DNS only to localhost (127.0.0.0/8, ::1, 127.0.0.53)
    /// For systems using systemd-resolved or local caching resolver
    Localhost,
    /// DNS to any destination (legacy behavior, least secure)
    /// Only use if you have specific requirements
    Any,
}

impl std::fmt::Display for DnsMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DnsMode::Tunnel => write!(f, "tunnel"),
            DnsMode::Strict => write!(f, "strict"),
            DnsMode::Localhost => write!(f, "localhost"),
            DnsMode::Any => write!(f, "any"),
        }
    }
}

/// IPv6 leak protection mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Ipv6Mode {
    /// Block all IPv6 except loopback (most secure)
    /// Recommended since most VPNs don't tunnel IPv6
    #[default]
    Block,
    /// Allow IPv6 only via VPN tunnel interfaces
    /// Use if your VPN properly tunnels IPv6
    Tunnel,
    /// No special IPv6 handling (legacy behavior)
    /// Warning: May cause IPv6 leaks
    Off,
}

/// Headless mode configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HeadlessConfig {
    /// Auto-connect to VPN on startup
    pub auto_connect: bool,
    /// Server to connect to on startup
    pub startup_server: Option<String>,
    /// Maximum reconnection attempts (0 = infinite)
    pub max_reconnect_attempts: u32,
    /// Delay between reconnection attempts (seconds)
    pub reconnect_delay_secs: u64,
    /// Enable kill switch before VPN connects
    pub kill_switch_on_boot: bool,
    /// Fail startup if kill switch cannot be enabled
    pub require_kill_switch: bool,
    /// Keep kill switch enabled after Shroud exits
    pub persist_kill_switch: bool,
}

impl Default for HeadlessConfig {
    fn default() -> Self {
        Self {
            auto_connect: false,
            startup_server: None,
            max_reconnect_attempts: 0,
            reconnect_delay_secs: 5,
            kill_switch_on_boot: true,
            require_kill_switch: true,
            persist_kill_switch: false,
        }
    }
}

/// Kill switch specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KillSwitchConfig {
    /// Allow LAN traffic when kill switch is active
    pub allow_lan: bool,
    /// When true, only allow common local-service ports to LAN
    /// (printing, file sharing, mDNS, DNS, DHCP, ICMP).
    /// Blocks arbitrary TCP/UDP to LAN devices.
    #[serde(default)]
    pub lan_restrict_ports: bool,
}

impl Default for KillSwitchConfig {
    fn default() -> Self {
        Self {
            allow_lan: true,
            lan_restrict_ports: false,
        }
    }
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Config file version for migration support
    pub version: u32,
    /// Whether auto-reconnect is enabled
    pub auto_reconnect: bool,
    /// Auto-connect to last_server on startup (desktop mode only)
    ///
    /// When `true` and `last_server` is set, Shroud connects to the last used
    /// VPN on startup if no VPN is already active. Pair with `shroud autostart on`
    /// for automatic VPN protection on login.
    #[serde(default)]
    pub auto_connect: bool,
    /// Last successfully connected server (for quick reconnect)
    pub last_server: Option<String>,
    /// Health check interval in seconds (0 to disable)
    pub health_check_interval_secs: u64,
    /// Health check latency threshold for degraded state (ms)
    pub health_degraded_threshold_ms: u64,
    /// Custom health check endpoints (URLs). If empty, uses built-in defaults.
    #[serde(default)]
    pub health_check_endpoints: Vec<String>,
    /// Maximum reconnection attempts before giving up
    pub max_reconnect_attempts: u32,
    /// Kill switch enabled (blocks non-VPN traffic)
    pub kill_switch_enabled: bool,
    /// DNS leak protection mode
    pub dns_mode: DnsMode,
    /// Block DNS-over-HTTPS to known providers (tunnel/strict)
    #[serde(default = "default_block_doh")]
    pub block_doh: bool,
    /// Additional DoH provider IPs to block
    #[serde(default)]
    pub custom_doh_blocklist: Vec<String>,
    /// IPv6 leak protection mode
    pub ipv6_mode: Ipv6Mode,
    /// Headless mode configuration
    #[serde(default)]
    pub headless: HeadlessConfig,
    /// Kill switch specific configuration
    #[serde(default)]
    pub killswitch: KillSwitchConfig,
    /// Notification settings
    #[serde(default)]
    pub notifications: crate::notifications::manager::NotificationConfig,
}

fn default_block_doh() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            auto_reconnect: true,
            auto_connect: false,
            last_server: None,
            health_check_interval_secs: 30,
            health_degraded_threshold_ms: 5000,
            health_check_endpoints: Vec::new(),
            max_reconnect_attempts: 10,
            kill_switch_enabled: false,
            dns_mode: DnsMode::default(),
            block_doh: default_block_doh(),
            custom_doh_blocklist: Vec::new(),
            ipv6_mode: Ipv6Mode::default(),
            headless: HeadlessConfig::default(),
            killswitch: KillSwitchConfig::default(),
            notifications: Default::default(),
        }
    }
}

impl Config {
    /// Validate config after loading
    pub fn validate(&self) -> Result<(), String> {
        if let Some(ref server) = self.last_server {
            validate_vpn_name(server)
                .map_err(|e| format!("Invalid last_server in config: {}", e))?;
        }

        // Health check interval bounds
        if self.health_check_interval_secs > 0 && self.health_check_interval_secs < 10 {
            return Err(format!(
                "health_check_interval_secs must be 0 (disabled) or >= 10, got {}",
                self.health_check_interval_secs
            ));
        }
        if self.health_check_interval_secs > 300 {
            return Err(format!(
                "health_check_interval_secs must be <= 300, got {}",
                self.health_check_interval_secs
            ));
        }

        // Health degraded threshold bounds
        if self.health_degraded_threshold_ms < 100 || self.health_degraded_threshold_ms > 30000 {
            return Err(format!(
                "health_degraded_threshold_ms must be between 100 and 30000, got {}",
                self.health_degraded_threshold_ms
            ));
        }

        // Max reconnect attempts
        if self.max_reconnect_attempts > 100 {
            return Err(format!(
                "max_reconnect_attempts must be <= 100, got {}",
                self.max_reconnect_attempts
            ));
        }

        // Health check endpoints validation
        if self.health_check_endpoints.len() > 10 {
            return Err(format!(
                "health_check_endpoints must have <= 10 entries, got {}",
                self.health_check_endpoints.len()
            ));
        }
        for (i, endpoint) in self.health_check_endpoints.iter().enumerate() {
            if endpoint.len() > 256 {
                return Err(format!(
                    "health_check_endpoints[{}] exceeds 256 characters",
                    i
                ));
            }
            if !endpoint.starts_with("https://") {
                return Err(format!(
                    "health_check_endpoints[{}] must use https:// scheme: {}",
                    i, endpoint
                ));
            }
        }

        // SECURITY: Validate custom_doh_blocklist entries are valid IPv4 addresses.
        // These flow directly into iptables/nft rules (SHROUD-VULN-022).
        for (i, ip) in self.custom_doh_blocklist.iter().enumerate() {
            if ip.parse::<std::net::Ipv4Addr>().is_err() {
                return Err(format!(
                    "custom_doh_blocklist[{}] is not a valid IPv4 address: {}",
                    i, ip
                ));
            }
        }

        Ok(())
    }
}

/// Configuration manager for loading and saving config
pub struct ConfigManager {
    /// Path to the config file
    config_path: PathBuf,
}

impl ConfigManager {
    /// Create a new config manager
    ///
    /// Uses XDG_CONFIG_HOME/shroud/config.toml or ~/.config/shroud/config.toml
    pub fn new() -> Self {
        let config_dir = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").expect("HOME not set");
                PathBuf::from(home).join(".config")
            })
            .join("shroud");

        Self {
            config_path: config_dir.join("config.toml"),
        }
    }

    /// Create a config manager with a specific config file path
    ///
    /// This is primarily for testing to avoid touching real user config.
    #[cfg(test)]
    pub fn with_path(config_path: PathBuf) -> Self {
        Self { config_path }
    }

    /// Get the config file path
    #[allow(dead_code)]
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    /// Load configuration from disk
    ///
    /// Returns default config if file doesn't exist or can't be parsed.
    /// Performs migration if config version is outdated.
    /// Also migrates config from old openvpn-tray location if present.
    pub fn load(&self) -> Config {
        // NOTE: Legacy config migration from openvpn-tray was removed
        // (SHROUD-VULN-024). The migration followed symlinks and trusted
        // arbitrary content on first load, bypassing all reload protections.

        if !self.config_path.exists() {
            debug!("Config file not found, using defaults");
            return Config::default();
        }

        match fs::read_to_string(&self.config_path) {
            Ok(contents) => self.parse_and_migrate(&contents),
            Err(e) => {
                warn!("Failed to read config file: {}. Using defaults.", e);
                Config::default()
            }
        }
    }

    /// Load configuration with validation, falling back to defaults on validation error.
    pub fn load_validated(&self) -> Config {
        let config = self.load();
        if let Err(e) = config.validate() {
            warn!("Config validation failed, using defaults: {}", e);
            Config::default()
        } else {
            config
        }
    }

    /// Parse config string and migrate if necessary
    fn parse_and_migrate(&self, contents: &str) -> Config {
        // First, try to parse as raw TOML to check version
        let raw: Result<toml::Value, _> = toml::from_str(contents);

        match raw {
            Ok(mut value) => {
                let version = value
                    .get("version")
                    .and_then(|v| v.as_integer())
                    .unwrap_or(0) as u32;

                if version < CONFIG_VERSION {
                    info!(
                        "Migrating config from version {} to {}",
                        version, CONFIG_VERSION
                    );
                    self.migrate(&mut value, version);
                }

                // Parse the (possibly migrated) value into Config
                match value.try_into::<Config>() {
                    Ok(config) => {
                        // SECURITY: Validate BEFORE persisting. The migration
                        // modifies values in-memory but no longer writes to disk
                        // until validation passes (SHROUD-VULN-039).
                        if let Err(e) = config.validate() {
                            warn!("Migrated config failed validation: {}. Using defaults.", e);
                            return Config::default();
                        }
                        // Validation passed — now safe to persist
                        if version < CONFIG_VERSION {
                            if let Err(e) = self.save(&config) {
                                warn!("Failed to save migrated config: {}", e);
                            }
                        }
                        info!("Loaded config from {:?}", self.config_path);
                        config
                    }
                    Err(e) => {
                        warn!("Failed to parse migrated config: {}. Using defaults.", e);
                        Config::default()
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Config file corrupted: {}. Backing up and using defaults.",
                    e
                );

                // Backup corrupted config file
                let backup_path = self.config_path.with_extension("toml.corrupted");
                if let Err(backup_err) = fs::rename(&self.config_path, &backup_path) {
                    warn!("Failed to backup corrupted config: {}", backup_err);
                } else {
                    info!("Corrupted config backed up to {:?}", backup_path);

                    // Write fresh defaults so user has a valid starting point
                    let default_config = Config::default();
                    if let Err(write_err) = self.save(&default_config) {
                        warn!("Failed to write default config: {}", write_err);
                    } else {
                        info!("Fresh default config written");
                    }
                }

                Config::default()
            }
        }
    }

    /// Migrate config from old version to current version
    fn migrate(&self, value: &mut toml::Value, from_version: u32) {
        let table = match value.as_table_mut() {
            Some(t) => t,
            None => return,
        };

        // Migration from version 0 (no version field) to version 1
        if from_version < 1 {
            // Add new fields with defaults if they don't exist
            if !table.contains_key("dns_mode") {
                table.insert(
                    "dns_mode".to_string(),
                    toml::Value::String("tunnel".to_string()),
                );
            }
            if !table.contains_key("block_doh") {
                table.insert("block_doh".to_string(), toml::Value::Boolean(true));
            }
            if !table.contains_key("custom_doh_blocklist") {
                table.insert(
                    "custom_doh_blocklist".to_string(),
                    toml::Value::Array(Vec::new()),
                );
            }
            if !table.contains_key("ipv6_mode") {
                table.insert(
                    "ipv6_mode".to_string(),
                    toml::Value::String("block".to_string()),
                );
            }
        }

        // Always update version to current
        table.insert(
            "version".to_string(),
            toml::Value::Integer(CONFIG_VERSION as i64),
        );

        // NOTE: Migration is now in-memory only. The migrated config is persisted
        // by the caller (parse_and_migrate) AFTER validation passes.
        // This prevents poisoned configs from being written to disk before
        // validation rejects them (SHROUD-VULN-039).
    }

    /// Save configuration to disk.
    ///
    /// Creates the config directory if it doesn't exist.
    /// Uses atomic write (temp file + rename) to prevent corruption on crash.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Directory`] if the config directory cannot be created.
    ///
    /// Returns [`ConfigError::Write`] if the temp file cannot be written or permissions set.
    ///
    /// Returns [`ConfigError::Rename`] if the atomic rename fails.
    ///
    /// Returns [`ConfigError::Serialize`] if the config cannot be serialized to TOML.
    pub fn save(&self, config: &Config) -> Result<(), ConfigError> {
        // Ensure config directory exists
        if let Some(parent) = self.config_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(ConfigError::Directory)?;

                // Set directory permissions to 700
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
                }
            }
        }

        // Ensure version is set correctly
        let mut config_to_save = config.clone();
        config_to_save.version = CONFIG_VERSION;

        let contents = toml::to_string_pretty(&config_to_save)?;

        // Atomic write: write to temp file, then rename
        // This prevents corruption if we crash mid-write
        let temp_path = self.config_path.with_extension("toml.tmp");

        // Write to temp file with correct permissions from the start
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp_path)
                .map_err(ConfigError::Write)?;
            file.write_all(contents.as_bytes())
                .map_err(ConfigError::Write)?;
            file.sync_all().map_err(ConfigError::Write)?;
        }

        #[cfg(not(unix))]
        {
            fs::write(&temp_path, &contents).map_err(ConfigError::Write)?;
        }

        // Atomic rename
        fs::rename(&temp_path, &self.config_path).map_err(ConfigError::Rename)?;

        debug!("Saved config to {:?}", self.config_path);
        Ok(())
    }

    /// Update a single setting and save
    #[allow(dead_code)]
    pub fn update<F>(&self, config: &mut Config, updater: F) -> Result<(), ConfigError>
    where
        F: FnOnce(&mut Config),
    {
        updater(config);
        self.save(config)
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert_eq!(config.version, CONFIG_VERSION);
        assert!(config.auto_reconnect);
        assert!(config.last_server.is_none());
        assert_eq!(config.health_check_interval_secs, 30);
        assert!(config.health_check_endpoints.is_empty());
        assert_eq!(config.max_reconnect_attempts, 10);
        assert_eq!(config.dns_mode, DnsMode::Tunnel);
        assert!(config.block_doh);
        assert!(config.custom_doh_blocklist.is_empty());
        assert_eq!(config.ipv6_mode, Ipv6Mode::Block);
    }

    #[test]
    fn test_config_serialize_deserialize() {
        let config = Config {
            version: 1,
            auto_reconnect: false,
            auto_connect: false,
            last_server: Some("us-east-1".to_string()),
            health_check_interval_secs: 60,
            health_degraded_threshold_ms: 3000,
            health_check_endpoints: vec!["https://example.com/health".to_string()],
            max_reconnect_attempts: 5,
            kill_switch_enabled: true,
            dns_mode: DnsMode::Localhost,
            block_doh: false,
            custom_doh_blocklist: vec!["1.1.1.1".to_string()],
            ipv6_mode: Ipv6Mode::Tunnel,
            headless: HeadlessConfig::default(),
            killswitch: KillSwitchConfig::default(),
            notifications: Default::default(),
        };

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.auto_reconnect, config.auto_reconnect);
        assert_eq!(parsed.last_server, config.last_server);
        assert_eq!(
            parsed.health_check_interval_secs,
            config.health_check_interval_secs
        );
        assert_eq!(parsed.max_reconnect_attempts, config.max_reconnect_attempts);
        assert_eq!(parsed.kill_switch_enabled, config.kill_switch_enabled);
        assert_eq!(parsed.dns_mode, config.dns_mode);
        assert_eq!(parsed.block_doh, config.block_doh);
        assert_eq!(parsed.custom_doh_blocklist, config.custom_doh_blocklist);
        assert_eq!(parsed.ipv6_mode, config.ipv6_mode);
        assert_eq!(parsed.health_check_endpoints, config.health_check_endpoints);
    }

    #[test]
    fn test_config_partial_parse() {
        // Test that missing fields use defaults (backward compatibility)
        let partial_toml = r#"
            auto_reconnect = false
        "#;

        let config: Config = toml::from_str(partial_toml).unwrap();
        assert!(!config.auto_reconnect);
        assert!(config.last_server.is_none()); // default
        assert_eq!(config.health_check_interval_secs, 30); // default
        assert!(config.health_check_endpoints.is_empty()); // default
        assert_eq!(config.dns_mode, DnsMode::Tunnel); // default
        assert!(config.block_doh);
        assert!(config.custom_doh_blocklist.is_empty());
        assert_eq!(config.ipv6_mode, Ipv6Mode::Block); // default
    }

    #[test]
    fn test_dns_mode_serialize() {
        // Test serialization via a config struct
        let config = Config {
            dns_mode: DnsMode::Tunnel,
            ..Default::default()
        };
        let toml_str = toml::to_string(&config).unwrap();
        assert!(toml_str.contains("dns_mode = \"tunnel\""));

        let config = Config {
            dns_mode: DnsMode::Localhost,
            ..Default::default()
        };
        let toml_str = toml::to_string(&config).unwrap();
        assert!(toml_str.contains("dns_mode = \"localhost\""));

        let config = Config {
            dns_mode: DnsMode::Any,
            ..Default::default()
        };
        let toml_str = toml::to_string(&config).unwrap();
        assert!(toml_str.contains("dns_mode = \"any\""));

        let config = Config {
            dns_mode: DnsMode::Strict,
            ..Default::default()
        };
        let toml_str = toml::to_string(&config).unwrap();
        assert!(toml_str.contains("dns_mode = \"strict\""));
    }

    #[test]
    fn test_ipv6_mode_serialize() {
        // Test serialization via a config struct
        let config = Config {
            ipv6_mode: Ipv6Mode::Block,
            ..Default::default()
        };
        let toml_str = toml::to_string(&config).unwrap();
        assert!(toml_str.contains("ipv6_mode = \"block\""));

        let config = Config {
            ipv6_mode: Ipv6Mode::Tunnel,
            ..Default::default()
        };
        let toml_str = toml::to_string(&config).unwrap();
        assert!(toml_str.contains("ipv6_mode = \"tunnel\""));

        let config = Config {
            ipv6_mode: Ipv6Mode::Off,
            ..Default::default()
        };
        let toml_str = toml::to_string(&config).unwrap();
        assert!(toml_str.contains("ipv6_mode = \"off\""));
    }

    #[test]
    fn test_unknown_fields_ignored() {
        // Unknown fields should not cause parse failure
        let toml_with_unknown = r#"
            version = 1
            auto_reconnect = true
            some_future_field = "value"
            another_unknown = 42
        "#;

        let config: Config = toml::from_str(toml_with_unknown).unwrap();
        assert!(config.auto_reconnect);
        assert_eq!(config.version, 1);
    }

    // === NEW IO TESTS ===

    #[test]
    fn test_load_returns_defaults_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("nonexistent").join("config.toml");
        let manager = ConfigManager::with_path(config_path);

        let config = manager.load();

        assert_eq!(config.version, 1);
        assert!(config.auto_reconnect);
        assert_eq!(config.dns_mode, DnsMode::Tunnel);
        assert_eq!(config.ipv6_mode, Ipv6Mode::Block);
    }

    #[test]
    fn test_save_creates_directory_and_writes_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("subdir").join("config.toml");
        let manager = ConfigManager::with_path(config_path.clone());

        let config = Config::default();
        let result = manager.save(&config);

        assert!(result.is_ok());
        assert!(config_path.exists());

        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert!(contents.contains("version = 1"));
        assert!(contents.contains("auto_reconnect = true"));
    }

    #[test]
    fn test_save_atomic_no_temp_file_remains() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let temp_path = dir.path().join("config.toml.tmp");
        let manager = ConfigManager::with_path(config_path.clone());

        let config = Config::default();
        manager.save(&config).unwrap();

        assert!(config_path.exists());
        assert!(
            !temp_path.exists(),
            "Temp file should not remain after save"
        );
    }

    #[test]
    fn test_save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let manager = ConfigManager::with_path(config_path);

        let original = Config {
            version: 1,
            auto_reconnect: false,
            auto_connect: false,
            last_server: Some("test-server".to_string()),
            health_check_interval_secs: 45,
            health_degraded_threshold_ms: 1500,
            health_check_endpoints: vec!["https://example.com".to_string()],
            max_reconnect_attempts: 5,
            kill_switch_enabled: true,
            dns_mode: DnsMode::Localhost,
            block_doh: false,
            custom_doh_blocklist: vec!["1.1.1.1".to_string()],
            ipv6_mode: Ipv6Mode::Tunnel,
            headless: HeadlessConfig::default(),
            killswitch: KillSwitchConfig::default(),
            notifications: Default::default(),
        };

        manager.save(&original).unwrap();
        let loaded = manager.load();

        assert_eq!(loaded.auto_reconnect, original.auto_reconnect);
        assert_eq!(loaded.last_server, original.last_server);
        assert_eq!(
            loaded.health_check_interval_secs,
            original.health_check_interval_secs
        );
        assert_eq!(loaded.kill_switch_enabled, original.kill_switch_enabled);
        assert_eq!(loaded.dns_mode, original.dns_mode);
        assert_eq!(loaded.ipv6_mode, original.ipv6_mode);
        assert_eq!(
            loaded.health_check_endpoints,
            original.health_check_endpoints
        );
    }

    #[test]
    fn test_migration_from_version_0_adds_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let manager = ConfigManager::with_path(config_path.clone());

        // Write a version 0 config (missing version, dns_mode, ipv6_mode)
        let old_config = r#"
auto_reconnect = false
last_server = "old-server"
health_check_interval_secs = 60
max_reconnect_attempts = 3
kill_switch_enabled = true
"#;
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, old_config).unwrap();

        let loaded = manager.load();

        // Should have migrated to version 1 with defaults
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.dns_mode, DnsMode::Tunnel);
        assert_eq!(loaded.ipv6_mode, Ipv6Mode::Block);
        // Original values preserved
        assert!(!loaded.auto_reconnect);
        assert_eq!(loaded.last_server, Some("old-server".to_string()));
        assert!(loaded.kill_switch_enabled);

        // Config file should be rewritten with version
        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert!(contents.contains("version = 1"));
        assert!(contents.contains("dns_mode"));
        assert!(contents.contains("ipv6_mode"));
    }

    #[test]
    fn test_load_with_invalid_toml_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let manager = ConfigManager::with_path(config_path.clone());

        // Write invalid TOML
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, "this is not valid toml {{{{").unwrap();

        let loaded = manager.load();

        // Should return defaults, not panic
        assert_eq!(loaded.version, 1);
        assert!(loaded.auto_reconnect);
    }

    #[cfg(unix)]
    #[test]
    fn test_save_sets_secure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let manager = ConfigManager::with_path(config_path.clone());

        manager.save(&Config::default()).unwrap();

        let metadata = std::fs::metadata(&config_path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "Config file should have 600 permissions");
    }

    // --- DnsMode Display ---

    #[test]
    fn test_dns_mode_display() {
        assert_eq!(DnsMode::Tunnel.to_string(), "tunnel");
        assert_eq!(DnsMode::Strict.to_string(), "strict");
        assert_eq!(DnsMode::Localhost.to_string(), "localhost");
        assert_eq!(DnsMode::Any.to_string(), "any");
    }

    #[test]
    fn test_dns_mode_default() {
        assert_eq!(DnsMode::default(), DnsMode::Tunnel);
    }

    // --- Config validation ---

    #[test]
    fn test_validate_valid_config() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_valid_last_server() {
        let config = Config {
            last_server: Some("my-vpn".into()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_empty_last_server() {
        let config = Config {
            last_server: Some("".into()),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_no_last_server() {
        let config = Config {
            last_server: None,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    // --- load_validated ---

    #[test]
    fn test_load_validated_returns_defaults_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("nonexistent").join("config.toml");
        let manager = ConfigManager::with_path(config_path);
        let config = manager.load_validated();
        assert!(config.auto_reconnect);
    }

    #[test]
    fn test_load_validated_rejects_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let manager = ConfigManager::with_path(config_path.clone());

        // Write config with invalid last_server (empty name)
        let bad_config = r#"
            version = 1
            auto_reconnect = false
            last_server = ""
        "#;
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, bad_config).unwrap();

        let loaded = manager.load_validated();
        // Should fall back to defaults because validation fails
        assert!(loaded.auto_reconnect);
        assert!(loaded.last_server.is_none());
    }

    // --- HeadlessConfig ---

    #[test]
    fn test_headless_config_default() {
        let hc = HeadlessConfig::default();
        assert!(!hc.auto_connect);
        assert!(hc.startup_server.is_none());
        assert_eq!(hc.max_reconnect_attempts, 0);
        assert!(hc.kill_switch_on_boot);
        assert!(hc.require_kill_switch);
        assert!(!hc.persist_kill_switch);
    }

    #[test]
    fn test_headless_config_serialize_roundtrip() {
        let hc = HeadlessConfig {
            auto_connect: true,
            startup_server: Some("vpn1".into()),
            max_reconnect_attempts: 5,
            reconnect_delay_secs: 10,
            kill_switch_on_boot: false,
            require_kill_switch: false,
            persist_kill_switch: true,
        };
        let s = toml::to_string(&hc).unwrap();
        let parsed: HeadlessConfig = toml::from_str(&s).unwrap();
        assert!(parsed.auto_connect);
        assert_eq!(parsed.startup_server, Some("vpn1".into()));
        assert_eq!(parsed.max_reconnect_attempts, 5);
        assert!(parsed.persist_kill_switch);
    }

    // --- KillSwitchConfig ---

    #[test]
    fn test_killswitch_config_default() {
        let kc = KillSwitchConfig::default();
        assert!(kc.allow_lan);
    }

    #[test]
    fn test_killswitch_config_roundtrip() {
        let kc = KillSwitchConfig {
            allow_lan: false,
            ..Default::default()
        };
        let s = toml::to_string(&kc).unwrap();
        let parsed: KillSwitchConfig = toml::from_str(&s).unwrap();
        assert!(!parsed.allow_lan);
    }

    // --- ConfigError Display ---

    #[test]
    fn test_config_error_display() {
        let err = ConfigError::Write(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied",
        ));
        assert!(err.to_string().contains("write"));

        let err = ConfigError::Directory(std::io::Error::other("missing"));
        assert!(err.to_string().contains("directory"));

        let err = ConfigError::Rename(std::io::Error::other("rename fail"));
        assert!(err.to_string().contains("atomic rename"));
    }

    // --- Config validation ---

    #[test]
    fn test_validate_health_interval_too_low() {
        let config = Config {
            health_check_interval_secs: 5,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_health_interval_zero_ok() {
        let config = Config {
            health_check_interval_secs: 0,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_health_interval_too_high() {
        let config = Config {
            health_check_interval_secs: 999,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_health_endpoint_http_rejected() {
        let config = Config {
            health_check_endpoints: vec!["http://evil.com".to_string()],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_health_endpoint_https_ok() {
        let config = Config {
            health_check_endpoints: vec!["https://1.1.1.1/cdn-cgi/trace".to_string()],
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_health_endpoints_too_many() {
        let config = Config {
            health_check_endpoints: (0..11).map(|i| format!("https://ep{}.com", i)).collect(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_reconnect_attempts_too_high() {
        let config = Config {
            max_reconnect_attempts: 200,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_degraded_threshold_bounds() {
        let config_low = Config {
            health_degraded_threshold_ms: 50,
            ..Default::default()
        };
        assert!(config_low.validate().is_err());
        let config_high = Config {
            health_degraded_threshold_ms: 50000,
            ..Default::default()
        };
        assert!(config_high.validate().is_err());
        let config_ok = Config {
            health_degraded_threshold_ms: 5000,
            ..Default::default()
        };
        assert!(config_ok.validate().is_ok());
    }

    #[test]
    fn test_validate_custom_doh_blocklist_valid() {
        let config = Config {
            custom_doh_blocklist: vec!["1.1.1.1".to_string(), "8.8.8.8".to_string()],
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_custom_doh_blocklist_injection_rejected() {
        // SHROUD-VULN-022: nft injection via custom_doh_blocklist
        let config = Config {
            custom_doh_blocklist: vec![
                "1.1.1.1 tcp dport 443 drop\n}\n}\ntable inet evil".to_string()
            ],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_custom_doh_blocklist_non_ip_rejected() {
        let config = Config {
            custom_doh_blocklist: vec!["not-an-ip".to_string()],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
