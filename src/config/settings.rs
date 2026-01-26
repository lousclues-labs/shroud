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
//! last_server = "us-east-1"  # optional
//! health_check_interval_secs = 30
//! health_degraded_threshold_ms = 2000
//! max_reconnect_attempts = 10
//! kill_switch_enabled = false
//!
//! # DNS leak protection mode: "tunnel" | "localhost" | "any"
//! # - tunnel: DNS only via VPN tunnel interfaces (most secure, default)
//! # - localhost: DNS only to 127.0.0.0/8, ::1, 127.0.0.53 (for local resolvers)
//! # - any: DNS to any destination (legacy, least secure)
//! dns_mode = "tunnel"
//!
//! # IPv6 leak protection: "block" | "tunnel" | "off"
//! # - block: Drop all IPv6 except loopback (most secure, default)
//! # - tunnel: Allow IPv6 only via VPN tunnel interfaces
//! # - off: No special IPv6 handling (legacy)
//! ipv6_mode = "block"
//! ```

use log::{debug, info, warn};
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
    /// DNS only to localhost (127.0.0.0/8, ::1, 127.0.0.53)
    /// For systems using systemd-resolved or local caching resolver
    Localhost,
    /// DNS to any destination (legacy behavior, least secure)
    /// Only use if you have specific requirements
    Any,
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

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Config file version for migration support
    pub version: u32,
    /// Whether auto-reconnect is enabled
    pub auto_reconnect: bool,
    /// Last successfully connected server (for quick reconnect)
    pub last_server: Option<String>,
    /// Health check interval in seconds (0 to disable)
    pub health_check_interval_secs: u64,
    /// Health check latency threshold for degraded state (ms)
    pub health_degraded_threshold_ms: u64,
    /// Maximum reconnection attempts before giving up
    pub max_reconnect_attempts: u32,
    /// Kill switch enabled (blocks non-VPN traffic)
    pub kill_switch_enabled: bool,
    /// DNS leak protection mode
    pub dns_mode: DnsMode,
    /// IPv6 leak protection mode
    pub ipv6_mode: Ipv6Mode,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            auto_reconnect: true,
            last_server: None,
            health_check_interval_secs: 30,
            health_degraded_threshold_ms: 2000,
            max_reconnect_attempts: 10,
            kill_switch_enabled: false,
            dns_mode: DnsMode::default(),
            ipv6_mode: Ipv6Mode::default(),
        }
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
        // Check for migration from old openvpn-tray config location
        self.migrate_from_old_location();

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

    /// Migrate config from old openvpn-tray location if present
    fn migrate_from_old_location(&self) {
        let old_config_dir = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_default();
                PathBuf::from(home).join(".config")
            })
            .join("openvpn-tray");

        let old_config_path = old_config_dir.join("config.toml");
        let migration_marker = old_config_dir.join("MIGRATED_TO_SHROUD.txt");

        // Only migrate if old config exists, new config doesn't, and not already migrated
        if old_config_path.exists() && !self.config_path.exists() && !migration_marker.exists() {
            info!(
                "Found old config at {:?}, migrating to {:?}",
                old_config_path, self.config_path
            );

            // Create new config directory
            if let Some(parent) = self.config_path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    warn!("Failed to create config directory: {}", e);
                    return;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
                }
            }

            // Copy old config to new location
            if let Err(e) = fs::copy(&old_config_path, &self.config_path) {
                warn!("Failed to copy config: {}", e);
                return;
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&self.config_path, fs::Permissions::from_mode(0o600));
            }

            // Leave a marker in the old location
            let marker_content = "This configuration has been migrated to ~/.config/shroud/\n\
                                  You may safely delete this directory.\n";
            let _ = fs::write(&migration_marker, marker_content);

            info!("Configuration migrated from ~/.config/openvpn-tray/ to ~/.config/shroud/");
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

                // Now parse the (possibly migrated) value into Config
                match value.try_into() {
                    Ok(config) => {
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
                warn!("Failed to parse config file: {}. Using defaults.", e);
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

        // Save migrated config
        if let Ok(migrated_str) = toml::to_string_pretty(value) {
            if let Err(e) = fs::write(&self.config_path, &migrated_str) {
                warn!("Failed to save migrated config: {}", e);
            } else {
                info!("Saved migrated config to {:?}", self.config_path);
            }
        }
    }

    /// Save configuration to disk
    ///
    /// Creates the config directory if it doesn't exist.
    /// Uses atomic write (temp file + rename) to prevent corruption on crash.
    pub fn save(&self, config: &Config) -> Result<(), String> {
        // Ensure config directory exists
        if let Some(parent) = self.config_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create config directory: {}", e))?;

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

        let contents = toml::to_string_pretty(&config_to_save)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

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
                .map_err(|e| format!("Failed to create temp config file: {}", e))?;
            file.write_all(contents.as_bytes())
                .map_err(|e| format!("Failed to write temp config file: {}", e))?;
            file.sync_all()
                .map_err(|e| format!("Failed to sync temp config file: {}", e))?;
        }

        #[cfg(not(unix))]
        {
            fs::write(&temp_path, &contents)
                .map_err(|e| format!("Failed to write temp config file: {}", e))?;
        }

        // Atomic rename
        fs::rename(&temp_path, &self.config_path)
            .map_err(|e| format!("Failed to rename temp config file: {}", e))?;

        debug!("Saved config to {:?}", self.config_path);
        Ok(())
    }

    /// Update a single setting and save
    #[allow(dead_code)]
    pub fn update<F>(&self, config: &mut Config, updater: F) -> Result<(), String>
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
        assert_eq!(config.max_reconnect_attempts, 10);
        assert_eq!(config.dns_mode, DnsMode::Tunnel);
        assert_eq!(config.ipv6_mode, Ipv6Mode::Block);
    }

    #[test]
    fn test_config_serialize_deserialize() {
        let config = Config {
            version: 1,
            auto_reconnect: false,
            last_server: Some("us-east-1".to_string()),
            health_check_interval_secs: 60,
            health_degraded_threshold_ms: 3000,
            max_reconnect_attempts: 5,
            kill_switch_enabled: true,
            dns_mode: DnsMode::Localhost,
            ipv6_mode: Ipv6Mode::Tunnel,
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
        assert_eq!(parsed.ipv6_mode, config.ipv6_mode);
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
        assert_eq!(config.dns_mode, DnsMode::Tunnel); // default
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
            last_server: Some("test-server".to_string()),
            health_check_interval_secs: 45,
            health_degraded_threshold_ms: 1500,
            max_reconnect_attempts: 5,
            kill_switch_enabled: true,
            dns_mode: DnsMode::Localhost,
            ipv6_mode: Ipv6Mode::Tunnel,
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
}
