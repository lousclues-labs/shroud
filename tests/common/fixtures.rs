//! Test fixtures and data generators
//!
//! Provides reusable test data, configuration builders, and fixture factories
//! for consistent testing across the test suite.

use std::path::PathBuf;
use tempfile::TempDir;

/// Test configuration fixture
#[derive(Debug, Clone)]
pub struct TestConfig {
    pub auto_reconnect: bool,
    pub kill_switch_enabled: bool,
    pub last_server: Option<String>,
    pub health_check_interval_secs: u64,
    pub dns_mode: TestDnsMode,
    pub ipv6_mode: TestIpv6Mode,
}

/// DNS mode for tests
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestDnsMode {
    Tunnel,
    Strict,
    Localhost,
    Any,
}

/// IPv6 mode for tests
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestIpv6Mode {
    Block,
    Tunnel,
    Off,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            auto_reconnect: false,
            kill_switch_enabled: false,
            last_server: None,
            health_check_interval_secs: 30,
            dns_mode: TestDnsMode::Tunnel,
            ipv6_mode: TestIpv6Mode::Block,
        }
    }
}

impl TestConfig {
    /// Create config with auto-reconnect enabled
    pub fn with_auto_reconnect(mut self) -> Self {
        self.auto_reconnect = true;
        self
    }

    /// Create config with kill switch enabled
    pub fn with_kill_switch(mut self) -> Self {
        self.kill_switch_enabled = true;
        self
    }

    /// Create config with last server
    pub fn with_last_server(mut self, server: &str) -> Self {
        self.last_server = Some(server.to_string());
        self
    }
}

/// Test environment that provides temporary directories and cleanup
pub struct TestEnv {
    temp_dir: TempDir,
    socket_path: PathBuf,
    config_path: PathBuf,
    log_path: PathBuf,
}

impl TestEnv {
    /// Create a new isolated test environment
    pub fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let base = temp_dir.path();

        Self {
            socket_path: base.join("shroud.sock"),
            config_path: base.join("shroud.toml"),
            log_path: base.join("shroud.log"),
            temp_dir,
        }
    }

    /// Get the socket path for this test
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    /// Get the config path for this test
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    /// Get the log path for this test
    pub fn log_path(&self) -> &PathBuf {
        &self.log_path
    }

    /// Get the temp directory path
    pub fn temp_dir(&self) -> &std::path::Path {
        self.temp_dir.path()
    }

    /// Create a file in the temp directory
    pub fn create_file(&self, name: &str, content: &str) -> PathBuf {
        let path = self.temp_dir.path().join(name);
        std::fs::write(&path, content).expect("Failed to write file");
        path
    }

    /// Write a test config file
    pub fn write_config(&self, config: &TestConfig) {
        let toml = format!(
            r#"
[general]
auto_reconnect = {}
last_server = {}
health_check_interval_secs = {}

[killswitch]
enabled = {}
dns_mode = "{:?}"
ipv6_mode = "{:?}"
"#,
            config.auto_reconnect,
            config
                .last_server
                .as_ref()
                .map(|s| format!("\"{}\"", s))
                .unwrap_or_else(|| "null".to_string()),
            config.health_check_interval_secs,
            config.kill_switch_enabled,
            config.dns_mode,
            config.ipv6_mode
        );
        std::fs::write(&self.config_path, toml).expect("Failed to write config");
    }
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// Sample .ovpn file content for import tests
pub fn sample_ovpn_config() -> &'static str {
    r#"
client
dev tun
proto udp
remote vpn.example.com 1194
resolv-retry infinite
nobind
persist-key
persist-tun
remote-cert-tls server
cipher AES-256-GCM
verb 3
<ca>
-----BEGIN CERTIFICATE-----
MIIBkjCB/AIJAKHBfpegPjMGMAoGCCqGSM49BAMCMBIxEDAOBgNVBAMMB3Rlc3Qt
Y2EwHhcNMjQwMTAxMDAwMDAwWhcNMjUwMTAxMDAwMDAwWjASMRAwDgYDVQQDDAd0
ZXN0LWNhMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEtest...
-----END CERTIFICATE-----
</ca>
<cert>
-----BEGIN CERTIFICATE-----
MIIBkjCB/AIJAKHBfpegPjMHMAoGCCqGSM49BAMCMBIxEDAOBgNVBAMMB3Rlc3Qt
Y2EwHhcNMjQwMTAxMDAwMDAwWhcNMjUwMTAxMDAwMDAwWjAUMRIwEAYDVQQDDAl0
ZXN0LXVzZXIwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAAStest...
-----END CERTIFICATE-----
</cert>
<key>
-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgtest...
-----END PRIVATE KEY-----
</key>
"#
}

/// Sample WireGuard config for import tests
pub fn sample_wireguard_config() -> &'static str {
    r#"
[Interface]
PrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=
Address = 10.200.200.2/32
DNS = 10.200.200.1

[Peer]
PublicKey = xTIBA5rboUvnH4htodjb60Y7YAf21J7YQMlNGC8HQ14=
AllowedIPs = 0.0.0.0/0
Endpoint = demo.wireguard.com:51820
PersistentKeepalive = 25
"#
}

/// Sample VPN list for mock NetworkManager
pub fn sample_vpn_list() -> Vec<(&'static str, &'static str)> {
    vec![
        ("vpn-us-east", "openvpn"),
        ("vpn-eu-west", "openvpn"),
        ("vpn-asia", "openvpn"),
        ("wg-home", "wireguard"),
        ("wg-office", "wireguard"),
    ]
}

/// Sample iptables -L output
pub fn sample_iptables_list() -> &'static str {
    r#"
Chain INPUT (policy ACCEPT)
target     prot opt source               destination

Chain FORWARD (policy ACCEPT)
target     prot opt source               destination

Chain OUTPUT (policy ACCEPT)
target     prot opt source               destination
SHROUD_KILLSWITCH  all  --  anywhere             anywhere

Chain SHROUD_KILLSWITCH (1 references)
target     prot opt source               destination
ACCEPT     all  --  anywhere             anywhere             state RELATED,ESTABLISHED
ACCEPT     all  --  anywhere             localhost
ACCEPT     all  --  anywhere             anywhere             owner UID match root
DROP       all  --  anywhere             anywhere
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_test_env_creation() {
        let env = TestEnv::new();
        assert!(env.temp_dir().exists());
        assert!(!env.socket_path().exists()); // Not created yet
    }

    #[test]
    fn test_test_config_builder() {
        let config = TestConfig::default()
            .with_auto_reconnect()
            .with_kill_switch()
            .with_last_server("vpn-us");

        assert!(config.auto_reconnect);
        assert!(config.kill_switch_enabled);
        assert_eq!(config.last_server, Some("vpn-us".to_string()));
    }

    #[test]
    fn test_env_file_creation() {
        let env = TestEnv::new();
        let path = env.create_file("test.txt", "hello world");
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    }
}
