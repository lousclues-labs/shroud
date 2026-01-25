//! Configuration settings
//!
//! Persistent storage for user preferences using TOML format.
//! Config file is stored in XDG_CONFIG_HOME/openvpn-tray/config.toml

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto_reconnect: true,
            last_server: None,
            health_check_interval_secs: 30,
            health_degraded_threshold_ms: 2000,
            max_reconnect_attempts: 10,
            kill_switch_enabled: false,
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
    /// Uses XDG_CONFIG_HOME/openvpn-tray/config.toml or ~/.config/openvpn-tray/config.toml
    pub fn new() -> Self {
        let config_dir = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").expect("HOME not set");
                PathBuf::from(home).join(".config")
            })
            .join("openvpn-tray");

        Self {
            config_path: config_dir.join("config.toml"),
        }
    }

    /// Get the config file path
    #[allow(dead_code)]
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    /// Load configuration from disk
    /// 
    /// Returns default config if file doesn't exist or can't be parsed
    pub fn load(&self) -> Config {
        if !self.config_path.exists() {
            debug!("Config file not found, using defaults");
            return Config::default();
        }

        match fs::read_to_string(&self.config_path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => {
                    info!("Loaded config from {:?}", self.config_path);
                    config
                }
                Err(e) => {
                    warn!("Failed to parse config file: {}. Using defaults.", e);
                    Config::default()
                }
            },
            Err(e) => {
                warn!("Failed to read config file: {}. Using defaults.", e);
                Config::default()
            }
        }
    }

    /// Save configuration to disk
    /// 
    /// Creates the config directory if it doesn't exist
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

        let contents = toml::to_string_pretty(config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        fs::write(&self.config_path, &contents)
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        // Set file permissions to 600
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.config_path, fs::Permissions::from_mode(0o600));
        }

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
        assert!(config.auto_reconnect);
        assert!(config.last_server.is_none());
        assert_eq!(config.health_check_interval_secs, 30);
        assert_eq!(config.max_reconnect_attempts, 10);
    }

    #[test]
    fn test_config_serialize_deserialize() {
        let config = Config {
            auto_reconnect: false,
            last_server: Some("us-east-1".to_string()),
            health_check_interval_secs: 60,
            health_degraded_threshold_ms: 3000,
            max_reconnect_attempts: 5,
            kill_switch_enabled: true,
        };

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.auto_reconnect, config.auto_reconnect);
        assert_eq!(parsed.last_server, config.last_server);
        assert_eq!(parsed.health_check_interval_secs, config.health_check_interval_secs);
        assert_eq!(parsed.max_reconnect_attempts, config.max_reconnect_attempts);
        assert_eq!(parsed.kill_switch_enabled, config.kill_switch_enabled);
    }

    #[test]
    fn test_config_partial_parse() {
        // Test that missing fields use defaults
        let partial_toml = r#"
            auto_reconnect = false
        "#;

        let config: Config = toml::from_str(partial_toml).unwrap();
        assert!(!config.auto_reconnect);
        assert!(config.last_server.is_none()); // default
        assert_eq!(config.health_check_interval_secs, 30); // default
    }
}
