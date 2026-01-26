//! CLI error types

use std::fmt;

/// Errors that can occur in CLI operations
#[derive(Debug)]
#[allow(dead_code)]
pub enum CliError {
    /// Daemon is not running
    DaemonNotRunning,
    /// Communication timeout
    Timeout,
    /// Connection closed unexpectedly
    ConnectionClosed,
    /// I/O error
    Io(std::io::Error),
    /// JSON serialization/deserialization error
    Json(serde_json::Error),
    /// Invalid command
    InvalidCommand(String),
    /// Invalid argument
    InvalidArgument(String),
    /// Command failed on server side
    CommandFailed { code: String, message: String },
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DaemonNotRunning => {
                write!(f, "Shroud daemon is not running. Start it with: shroud")
            }
            Self::Timeout => write!(f, "Timeout waiting for response from daemon"),
            Self::ConnectionClosed => write!(f, "Connection to daemon closed unexpectedly"),
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::Json(e) => write!(f, "Protocol error: {}", e),
            Self::InvalidCommand(cmd) => {
                write!(
                    f,
                    "Unknown command: '{}'. Run 'shroud --help' for usage.",
                    cmd
                )
            }
            Self::InvalidArgument(msg) => write!(f, "{}", msg),
            Self::CommandFailed { message, .. } => write!(f, "{}", message),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl CliError {
    /// Get the appropriate exit code for this error
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::DaemonNotRunning => 2,
            Self::Timeout => 3,
            _ => 1,
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
