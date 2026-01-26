//! IPC command and response types
//!
//! Defines the JSON-based protocol for CLI to daemon communication.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// CLI command sent to the daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
pub enum CliCommand {
    // Connection management
    Connect { name: String },
    Disconnect,
    Reconnect,
    Switch { name: String },

    // Status and information
    Status,
    List,

    // Kill switch
    KillSwitchOn,
    KillSwitchOff,
    KillSwitchToggle,
    KillSwitchStatus,

    // Auto-reconnect
    AutoReconnectOn,
    AutoReconnectOff,
    AutoReconnectToggle,
    AutoReconnectStatus,

    // Debug
    DebugOn,
    DebugOff,
    DebugLogPath,
    DebugDump,

    // Daemon control
    Ping,
    Refresh,
    Quit,
    Restart,
}

/// Request sent over the socket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliRequest {
    #[serde(flatten)]
    pub command: CliCommand,
    pub request_id: String,
}

/// Response from the daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliResponse {
    pub success: bool,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CliErrorInfo>,
}

/// Error information in a response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliErrorInfo {
    pub code: String,
    pub message: String,
}

impl CliResponse {
    /// Create a successful response
    pub fn success(request_id: String, data: Value) -> Self {
        Self {
            success: true,
            request_id,
            data: Some(data),
            error: None,
        }
    }

    /// Create a successful response with no data
    #[allow(dead_code)]
    pub fn ok(request_id: String) -> Self {
        Self {
            success: true,
            request_id,
            data: None,
            error: None,
        }
    }

    /// Create an error response
    pub fn error(request_id: String, code: &str, message: &str) -> Self {
        Self {
            success: false,
            request_id,
            data: None,
            error: Some(CliErrorInfo {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }
}

impl CliRequest {
    /// Create a new request with auto-generated ID
    pub fn new(command: CliCommand) -> Self {
        Self {
            command,
            request_id: generate_request_id(),
        }
    }
}

/// Generate a unique request ID
pub fn generate_request_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{}", duration.as_secs(), duration.subsec_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_connect_command() {
        let req = CliRequest::new(CliCommand::Connect {
            name: "ireland-42".to_string(),
        });
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"command\":\"connect\""));
        assert!(json.contains("\"name\":\"ireland-42\""));
    }

    #[test]
    fn test_serialize_simple_command() {
        let req = CliRequest::new(CliCommand::Status);
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"command\":\"status\""));
    }

    #[test]
    fn test_deserialize_connect() {
        let json = r#"{"command":"connect","args":{"name":"test"},"request_id":"123"}"#;
        let req: CliRequest = serde_json::from_str(json).unwrap();
        assert!(matches!(req.command, CliCommand::Connect { name } if name == "test"));
    }

    #[test]
    fn test_response_success() {
        let resp =
            CliResponse::success("123".to_string(), serde_json::json!({"state": "Connected"}));
        assert!(resp.success);
        assert!(resp.error.is_none());
        assert!(resp.data.is_some());
    }

    #[test]
    fn test_response_error() {
        let resp = CliResponse::error("123".to_string(), "not_found", "Connection not found");
        assert!(!resp.success);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, "not_found");
    }
}
