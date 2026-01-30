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
    List,

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
        /// Current state description
        state: String,
        /// Kill switch status
        kill_switch_enabled: bool,
    },

    /// List of available VPN connections.
    Connections {
        /// Names of available VPN connections
        names: Vec<String>,
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
            _ => Ok(()),
        }
    }

    /// Returns a human-readable description of the command.
    #[allow(dead_code)]
    pub fn description(&self) -> &'static str {
        match self {
            IpcCommand::Connect { .. } => "connect to VPN",
            IpcCommand::Disconnect => "disconnect from VPN",
            IpcCommand::Switch { .. } => "switch VPN",
            IpcCommand::Status => "query status",
            IpcCommand::List => "list connections",
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_response_serialize_error() {
        let resp = IpcResponse::Error {
            message: "something failed".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("Error"));
        assert!(json.contains("something failed"));
    }

    #[test]
    fn test_socket_path_uses_xdg() {
        // This test verifies the function doesn't panic
        let path = socket_path();
        assert!(path.to_string_lossy().contains("shroud"));
    }
}
