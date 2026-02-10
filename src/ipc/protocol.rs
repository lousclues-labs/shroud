//! IPC protocol definitions.
//!
//! Defines the command and response types used for communication between
//! the Shroud daemon and CLI clients.
//!
//! # Protocol
//!
//! Communication uses JSON-serialized messages over a Unix domain socket.
//! Each message is a single line (newline-delimited JSON).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::cli::validation::validate_vpn_name;

/// Current IPC protocol version.
/// Bump this when adding/removing/changing command or response variants.
pub const PROTOCOL_VERSION: u32 = 1;

/// Path to the IPC Unix domain socket.
///
/// Uses XDG_RUNTIME_DIR for proper user isolation.
/// Falls back to /tmp if XDG_RUNTIME_DIR is not set.
pub fn socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("shroud.sock")
    } else {
        PathBuf::from("/tmp").join(format!("shroud-{}.sock", unsafe { libc::getuid() }))
    }
}

/// Legacy constant for compatibility - prefer socket_path() function
#[allow(dead_code)]
pub const SOCKET_PATH: &str = "/tmp/shroud.sock";

/// Commands sent from CLI client to daemon.
///
/// Each variant represents an action the client wants the daemon to perform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data")]
pub enum IpcCommand {
    /// Protocol version handshake. Must be the first message sent by a client.
    Hello {
        version: u32,
    },

    /// Report daemon binary version and protocol version.
    Version,

    /// Connect to a VPN by name.
    Connect {
        /// Name of the VPN connection (as shown in NetworkManager)
        name: String,
    },

    /// Disconnect from the current VPN.
    Disconnect,

    /// Switch to another VPN (Disconnect + Connect).
    Switch {
        name: String,
    },

    /// Query current connection status.
    Status,

    /// List available VPN connections.
    List {
        /// Optional VPN type filter (wireguard/openvpn/all)
        vpn_type: Option<String>,
    },

    /// Reconnect to the last used VPN.
    Reconnect,

    /// Enable or disable the kill switch.
    KillSwitch {
        /// Whether to enable (true) or disable (false) the kill switch
        enable: bool,
    },
    KillSwitchToggle,
    KillSwitchStatus,

    // Auto-reconnect
    AutoReconnect {
        enable: bool,
    },
    AutoReconnectToggle,
    AutoReconnectStatus,

    // Debug
    Debug {
        enable: bool,
    },
    DebugLogPath,
    DebugDump,

    // Daemon control
    Ping,
    Refresh,
    Quit,
    Restart,
    Reload,
}

/// Responses sent from daemon to CLI client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data")]
pub enum IpcResponse {
    /// Handshake response confirming accepted protocol version.
    HelloOk { version: u32 },

    /// Protocol version mismatch — client should upgrade/downgrade.
    VersionMismatch {
        server_version: u32,
        client_version: u32,
    },

    /// Operation completed successfully.
    Ok,

    /// Operation completed with a message
    OkMessage { message: String },

    /// Operation failed with an error message.
    Error {
        /// Human-readable error description
        message: String,
    },

    /// Status response.
    Status {
        /// Whether a VPN is currently connected
        connected: bool,
        /// Name of connected VPN (if any)
        vpn_name: Option<String>,
        /// VPN type (wireguard/openvpn) if connected
        vpn_type: Option<String>,
        /// Current state description
        state: String,
        /// Kill switch status
        kill_switch_enabled: bool,
    },

    /// List of available VPN connections.
    Connections {
        /// VPN connection entries
        connections: Vec<VpnConnectionInfo>,
    },

    /// Kill switch status
    KillSwitchStatus { enabled: bool },

    /// Auto-reconnect status
    AutoReconnectStatus { enabled: bool },

    /// Debug info
    DebugInfo {
        log_path: Option<String>,
        debug_enabled: bool,
    },

    /// Version info
    VersionInfo {
        binary_version: String,
        protocol_version: u32,
    },

    /// Pong response
    Pong,
}

impl IpcCommand {
    /// Validate command contents after deserialization
    pub fn validate(&self) -> Result<(), String> {
        match self {
            IpcCommand::Connect { name } => {
                validate_vpn_name(name).map_err(|e| e.to_string())?;
                Ok(())
            }
            IpcCommand::Switch { name } => {
                validate_vpn_name(name).map_err(|e| e.to_string())?;
                Ok(())
            }
            IpcCommand::List { vpn_type } => {
                if let Some(value) = vpn_type {
                    let normalized = value.to_lowercase();
                    if normalized != "wireguard" && normalized != "openvpn" {
                        return Err("Invalid VPN type filter".to_string());
                    }
                }
                Ok(())
            }
            IpcCommand::Hello { version } => {
                if *version == 0 {
                    return Err("Invalid protocol version".to_string());
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Returns a human-readable description of the command.
    #[allow(dead_code)]
    pub fn description(&self) -> &'static str {
        match self {
            IpcCommand::Hello { .. } => "handshake",
            IpcCommand::Version => "daemon version",
            IpcCommand::Connect { .. } => "connect to VPN",
            IpcCommand::Disconnect => "disconnect from VPN",
            IpcCommand::Switch { .. } => "switch VPN",
            IpcCommand::Status => "query status",
            IpcCommand::List { .. } => "list connections",
            IpcCommand::Reconnect => "reconnect to last VPN",
            IpcCommand::KillSwitch { enable: true } => "enable kill switch",
            IpcCommand::KillSwitch { enable: false } => "disable kill switch",
            IpcCommand::KillSwitchToggle => "toggle kill switch",
            IpcCommand::KillSwitchStatus => "query kill switch status",
            IpcCommand::AutoReconnect { enable: true } => "enable auto-reconnect",
            IpcCommand::AutoReconnect { enable: false } => "disable auto-reconnect",
            IpcCommand::AutoReconnectToggle => "toggle auto-reconnect",
            IpcCommand::AutoReconnectStatus => "query auto-reconnect status",
            IpcCommand::Debug { enable: true } => "enable debug mode",
            IpcCommand::Debug { enable: false } => "disable debug mode",
            IpcCommand::DebugLogPath => "get log path",
            IpcCommand::DebugDump => "dump debug info",
            IpcCommand::Ping => "ping daemon",
            IpcCommand::Refresh => "refresh connections",
            IpcCommand::Quit => "shutdown daemon",
            IpcCommand::Restart => "restart daemon",
            IpcCommand::Reload => "reload configuration",
        }
    }
}

impl IpcResponse {
    /// Returns true if this is a success response.
    pub fn is_ok(&self) -> bool {
        !matches!(self, IpcResponse::Error { .. })
    }

    /// Returns the error message if this is an error response.
    #[allow(dead_code)]
    pub fn error_message(&self) -> Option<&str> {
        match self {
            IpcResponse::Error { message } => Some(message),
            _ => None,
        }
    }
}

/// VPN connection info for list responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VpnConnectionInfo {
    pub name: String,
    pub vpn_type: String,
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- Serialization -----

    #[test]
    fn test_command_serialize_connect() {
        let cmd = IpcCommand::Connect {
            name: "my-vpn".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("Connect"));
        assert!(json.contains("my-vpn"));
    }

    #[test]
    fn test_command_deserialize_connect() {
        let json = r#"{"type":"Connect","data":{"name":"test-vpn"}}"#;
        let cmd: IpcCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, IpcCommand::Connect { name } if name == "test-vpn"));
    }

    #[test]
    fn test_response_serialize_ok() {
        let resp = IpcResponse::Ok;
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("Ok"));
    }

    #[test]
    fn test_response_serialize_hello_ok() {
        let resp = IpcResponse::HelloOk {
            version: PROTOCOL_VERSION,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("HelloOk"));
    }

    #[test]
    fn test_response_serialize_error() {
        let resp = IpcResponse::Error {
            message: "something failed".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("Error"));
        assert!(json.contains("something failed"));
    }

    #[test]
    fn test_response_serialize_version_mismatch() {
        let resp = IpcResponse::VersionMismatch {
            server_version: 1,
            client_version: 2,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("VersionMismatch"));
    }

    #[test]
    fn test_roundtrip_version_info() {
        let resp = IpcResponse::VersionInfo {
            binary_version: "1.13.0".into(),
            protocol_version: PROTOCOL_VERSION,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: IpcResponse = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(back, IpcResponse::VersionInfo { protocol_version, .. } if protocol_version == PROTOCOL_VERSION)
        );
    }

    #[test]
    fn test_socket_path_uses_xdg() {
        let path = socket_path();
        assert!(path.to_string_lossy().contains("shroud"));
    }

    // ----- Command roundtrip serialization -----

    #[test]
    fn test_roundtrip_disconnect() {
        let cmd = IpcCommand::Disconnect;
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, IpcCommand::Disconnect);
    }

    #[test]
    fn test_roundtrip_status() {
        let cmd = IpcCommand::Status;
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, IpcCommand::Status);
    }

    #[test]
    fn test_roundtrip_list_no_filter() {
        let cmd = IpcCommand::List { vpn_type: None };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, IpcCommand::List { vpn_type: None }));
    }

    #[test]
    fn test_roundtrip_list_with_filter() {
        let cmd = IpcCommand::List {
            vpn_type: Some("wireguard".into()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            IpcCommand::List { vpn_type: Some(t) } if t == "wireguard"
        ));
    }

    #[test]
    fn test_roundtrip_killswitch_enable() {
        let cmd = IpcCommand::KillSwitch { enable: true };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, IpcCommand::KillSwitch { enable: true });
    }

    #[test]
    fn test_roundtrip_killswitch_disable() {
        let cmd = IpcCommand::KillSwitch { enable: false };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, IpcCommand::KillSwitch { enable: false });
    }

    #[test]
    fn test_roundtrip_ping() {
        let cmd = IpcCommand::Ping;
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, IpcCommand::Ping);
    }

    #[test]
    fn test_validation_hello_zero() {
        let cmd = IpcCommand::Hello { version: 0 };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_validation_hello_valid() {
        let cmd = IpcCommand::Hello {
            version: PROTOCOL_VERSION,
        };
        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_roundtrip_quit() {
        let cmd = IpcCommand::Quit;
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, IpcCommand::Quit);
    }

    #[test]
    fn test_roundtrip_version() {
        let cmd = IpcCommand::Version;
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, IpcCommand::Version);
    }

    #[test]
    fn test_roundtrip_hello() {
        let cmd = IpcCommand::Hello {
            version: PROTOCOL_VERSION,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back,
            IpcCommand::Hello {
                version: PROTOCOL_VERSION
            }
        );
    }

    #[test]
    fn test_roundtrip_reconnect() {
        let cmd = IpcCommand::Reconnect;
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, IpcCommand::Reconnect);
    }

    #[test]
    fn test_roundtrip_switch() {
        let cmd = IpcCommand::Switch {
            name: "vpn2".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: IpcCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, IpcCommand::Switch { name } if name == "vpn2"));
    }

    // ----- Response roundtrip -----

    #[test]
    fn test_roundtrip_response_status() {
        let resp = IpcResponse::Status {
            connected: true,
            vpn_name: Some("my-vpn".into()),
            vpn_type: Some("wireguard".into()),
            state: "Connected".into(),
            kill_switch_enabled: true,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: IpcResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            IpcResponse::Status {
                connected: true,
                ..
            }
        ));
    }

    #[test]
    fn test_roundtrip_response_connections() {
        let resp = IpcResponse::Connections {
            connections: vec![
                VpnConnectionInfo {
                    name: "vpn1".into(),
                    vpn_type: "wireguard".into(),
                    status: "active".into(),
                },
                VpnConnectionInfo {
                    name: "vpn2".into(),
                    vpn_type: "openvpn".into(),
                    status: "inactive".into(),
                },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: IpcResponse = serde_json::from_str(&json).unwrap();
        match back {
            IpcResponse::Connections { connections } => {
                assert_eq!(connections.len(), 2);
                assert_eq!(connections[0].name, "vpn1");
            }
            _ => panic!("Expected Connections"),
        }
    }

    #[test]
    fn test_roundtrip_pong() {
        let resp = IpcResponse::Pong;
        let json = serde_json::to_string(&resp).unwrap();
        let back: IpcResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, IpcResponse::Pong);
    }

    // ----- Validation -----

    #[test]
    fn test_validate_connect_valid() {
        let cmd = IpcCommand::Connect {
            name: "my-vpn".into(),
        };
        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_validate_connect_empty() {
        let cmd = IpcCommand::Connect { name: "".into() };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_validate_switch_valid() {
        let cmd = IpcCommand::Switch {
            name: "vpn2".into(),
        };
        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_validate_switch_empty() {
        let cmd = IpcCommand::Switch { name: "".into() };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_validate_list_valid_type() {
        let cmd = IpcCommand::List {
            vpn_type: Some("wireguard".into()),
        };
        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_validate_list_invalid_type() {
        let cmd = IpcCommand::List {
            vpn_type: Some("invalid".into()),
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_validate_list_no_type() {
        let cmd = IpcCommand::List { vpn_type: None };
        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_validate_other_commands() {
        assert!(IpcCommand::Status.validate().is_ok());
        assert!(IpcCommand::Disconnect.validate().is_ok());
        assert!(IpcCommand::Ping.validate().is_ok());
        assert!(IpcCommand::Quit.validate().is_ok());
    }

    // ----- Response helpers -----

    #[test]
    fn test_response_is_ok() {
        assert!(IpcResponse::Ok.is_ok());
        assert!(IpcResponse::Pong.is_ok());
        assert!(IpcResponse::OkMessage {
            message: "done".into()
        }
        .is_ok());
    }

    #[test]
    fn test_response_is_not_ok_for_error() {
        assert!(!IpcResponse::Error {
            message: "fail".into()
        }
        .is_ok());
    }

    #[test]
    fn test_error_message() {
        let resp = IpcResponse::Error {
            message: "boom".into(),
        };
        assert_eq!(resp.error_message(), Some("boom"));
    }

    #[test]
    fn test_error_message_none_for_ok() {
        assert_eq!(IpcResponse::Ok.error_message(), None);
        assert_eq!(IpcResponse::Pong.error_message(), None);
    }

    // ----- Description -----

    #[test]
    fn test_command_descriptions() {
        assert_eq!(
            IpcCommand::Hello {
                version: PROTOCOL_VERSION
            }
            .description(),
            "handshake"
        );
        assert_eq!(IpcCommand::Version.description(), "daemon version");
        assert_eq!(
            IpcCommand::Connect { name: "x".into() }.description(),
            "connect to VPN"
        );
        assert_eq!(IpcCommand::Disconnect.description(), "disconnect from VPN");
        assert_eq!(IpcCommand::Status.description(), "query status");
        assert_eq!(IpcCommand::Ping.description(), "ping daemon");
        assert_eq!(IpcCommand::Quit.description(), "shutdown daemon");
        assert_eq!(
            IpcCommand::KillSwitch { enable: true }.description(),
            "enable kill switch"
        );
        assert_eq!(
            IpcCommand::KillSwitch { enable: false }.description(),
            "disable kill switch"
        );
    }

    // ----- VpnConnectionInfo -----

    #[test]
    fn test_vpn_connection_info_serialize() {
        let info = VpnConnectionInfo {
            name: "vpn1".into(),
            vpn_type: "wireguard".into(),
            status: "active".into(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("vpn1"));
        assert!(json.contains("wireguard"));
    }

    #[test]
    fn test_vpn_connection_info_roundtrip() {
        let info = VpnConnectionInfo {
            name: "vpn1".into(),
            vpn_type: "openvpn".into(),
            status: "inactive".into(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: VpnConnectionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, info);
    }

    // ----- Deserialize errors -----

    #[test]
    fn test_deserialize_invalid_json() {
        let result = serde_json::from_str::<IpcCommand>("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_empty_object() {
        let result = serde_json::from_str::<IpcCommand>("{}");
        assert!(result.is_err());
    }
}
