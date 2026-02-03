//! Test fixtures and builders

use std::io::Write;
use tempfile::NamedTempFile;

/// Create a temporary config file with given TOML content
pub fn create_temp_config(content: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("Failed to create temp file");
    writeln!(file, "{}", content).expect("Failed to write config");
    file
}

/// Minimal valid config TOML
pub fn minimal_config_toml() -> &'static str {
    r#"
version = 1
auto_reconnect = true
kill_switch_enabled = false
"#
}

/// Config with kill switch enabled
pub fn killswitch_config_toml() -> &'static str {
    r#"
version = 1
auto_reconnect = true
kill_switch_enabled = true
last_server = "test-vpn"
"#
}

/// Config for headless mode
pub fn headless_config_toml() -> &'static str {
    r#"
version = 1
auto_reconnect = true
kill_switch_enabled = true

[headless]
auto_connect = true
startup_server = "my-server"
kill_switch_on_boot = true
max_reconnect_attempts = 5
reconnect_delay_secs = 10
"#
}

/// Config with gateway enabled
pub fn gateway_config_toml() -> &'static str {
    r#"
version = 1
auto_reconnect = true

[gateway]
enabled = true
allowed_clients = "all"
"#
}

/// Config with specific allowed clients CIDR
pub fn gateway_cidr_config_toml() -> &'static str {
    r#"
version = 1

[gateway]
enabled = true

[gateway.allowed_clients]
cidr = "192.168.1.0/24"
"#
}

/// Config with allowed clients list
pub fn gateway_list_config_toml() -> &'static str {
    r#"
version = 1

[gateway]
enabled = true

[gateway.allowed_clients]
list = ["10.0.0.1", "10.0.0.2", "10.0.0.3"]
"#
}

/// Invalid TOML content
pub fn invalid_config_toml() -> &'static str {
    "this is {{ not valid toml {{"
}
