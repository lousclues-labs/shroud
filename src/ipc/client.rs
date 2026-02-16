// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! IPC client for CLI communication with daemon.
//!
//! Provides functions for connecting to the Shroud daemon and sending commands.
//!
//! # Example
//!
//! ```ignore
//! use shroud::ipc::client;
//! use shroud::ipc::protocol::IpcCommand;
//!
//! let response = client::send_command(IpcCommand::Status).await?;
//! println!("Status: {:?}", response);
//! ```

use std::io;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tracing::debug;

use super::protocol::{socket_path, IpcCommand, IpcResponse, PROTOCOL_VERSION};

use thiserror::Error;

/// Error type for IPC client operations.
#[derive(Error, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum ClientError {
    /// Failed to connect to daemon socket
    #[error("Failed to connect to daemon: {0}")]
    Connection(#[source] io::Error),
    /// Failed to send command
    #[error("Failed to send command: {0}")]
    Send(#[source] io::Error),
    /// Failed to receive response
    #[error("Failed to receive response: {0}")]
    Receive(#[source] io::Error),
    /// Failed to parse response
    #[error("Failed to parse response: {0}")]
    Parse(#[from] serde_json::Error),
    /// IPC protocol version mismatch
    #[error("IPC protocol version mismatch: daemon is v{server_version}, client is v{client_version}. Please restart the daemon.")]
    VersionMismatch {
        server_version: u32,
        client_version: u32,
    },
    /// Daemon is not running
    #[error("Daemon is not running. Start it with: shroud")]
    DaemonNotRunning,
}

/// Connect to the Shroud daemon.
///
/// Returns a connected Unix stream, or an error if the daemon is not running.
///
/// # Errors
///
/// Returns [`ClientError::DaemonNotRunning`] if the socket does not exist or the connection is refused.
///
/// Returns [`ClientError::Connection`] for other I/O errors establishing the socket.
pub async fn connect_to_daemon() -> Result<UnixStream, ClientError> {
    let path = socket_path();

    if !path.exists() {
        return Err(ClientError::DaemonNotRunning);
    }

    UnixStream::connect(&path).await.map_err(|e| {
        if e.kind() == io::ErrorKind::ConnectionRefused {
            ClientError::DaemonNotRunning
        } else {
            ClientError::Connection(e)
        }
    })
}

/// Send a command to the daemon and receive the response.
///
/// # Arguments
///
/// * `command` - The command to send
///
/// # Returns
///
/// The response from the daemon, or an error if communication failed.
/// Send a command to the daemon (creates a new connection).
///
/// # Errors
///
/// Returns [`ClientError::DaemonNotRunning`] if the IPC socket is missing or unavailable.
///
/// Returns [`ClientError::Connection`] for connection I/O errors.
///
/// Returns [`ClientError::Send`] if writing the request fails (broken pipe).
///
/// Returns [`ClientError::Receive`] if the response is empty or the connection closes prematurely.
///
/// Returns [`ClientError::Parse`] if the response cannot be parsed as JSON.
pub async fn send_command(command: IpcCommand) -> Result<IpcResponse, ClientError> {
    let stream = connect_to_daemon().await?;
    send_command_on_stream(stream, command).await
}

/// Send a command on an existing stream.
///
/// This is useful when you want to reuse a connection for multiple commands.
///
/// # Errors
///
/// Returns [`ClientError::Send`] if writing the request fails (broken pipe).
///
/// Returns [`ClientError::Receive`] if the response is empty or the connection closes prematurely.
///
/// Returns [`ClientError::Parse`] if the response cannot be parsed as JSON.
pub async fn send_command_on_stream(
    stream: UnixStream,
    command: IpcCommand,
) -> Result<IpcResponse, ClientError> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // Handshake
    let hello = IpcCommand::Hello {
        version: PROTOCOL_VERSION,
    };
    let hello_json = serde_json::to_string(&hello).map_err(ClientError::Parse)?;
    debug!("Sending hello: {}", hello_json);
    writer
        .write_all(hello_json.as_bytes())
        .await
        .map_err(ClientError::Send)?;
    writer.write_all(b"\n").await.map_err(ClientError::Send)?;
    writer.flush().await.map_err(ClientError::Send)?;

    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .await
        .map_err(ClientError::Receive)?;

    debug!("Received hello response: {}", response_line.trim());

    if !response_line.trim().is_empty() {
        if let Ok(resp) = serde_json::from_str::<IpcResponse>(response_line.trim()) {
            match resp {
                IpcResponse::HelloOk { version } => {
                    if version != PROTOCOL_VERSION {
                        return Err(ClientError::VersionMismatch {
                            server_version: version,
                            client_version: PROTOCOL_VERSION,
                        });
                    }
                }
                IpcResponse::VersionMismatch {
                    server_version,
                    client_version,
                } => {
                    return Err(ClientError::VersionMismatch {
                        server_version,
                        client_version,
                    });
                }
                _ => {
                    // Legacy daemon or unexpected response - proceed
                }
            }
        } else {
            // Legacy daemon - proceed
        }
    }

    response_line.clear();

    // Serialize and send command
    let command_json = serde_json::to_string(&command).map_err(ClientError::Parse)?;

    debug!("Sending command: {}", command_json);

    writer
        .write_all(command_json.as_bytes())
        .await
        .map_err(ClientError::Send)?;
    writer.write_all(b"\n").await.map_err(ClientError::Send)?;
    writer.flush().await.map_err(ClientError::Send)?;

    // Read response
    reader
        .read_line(&mut response_line)
        .await
        .map_err(ClientError::Receive)?;

    debug!("Received response: {}", response_line.trim());

    if response_line.trim().is_empty() {
        if matches!(command, IpcCommand::Restart | IpcCommand::Quit) {
            return Ok(IpcResponse::Ok);
        }
        return Err(ClientError::Receive(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Empty response from daemon",
        )));
    }

    // Parse response
    serde_json::from_str(response_line.trim()).map_err(ClientError::Parse)
}

/// Check if the daemon is running.
///
/// Returns `true` if the daemon socket exists and is connectable.
#[allow(dead_code)]
pub async fn is_daemon_running() -> bool {
    connect_to_daemon().await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_error_display() {
        let err = ClientError::DaemonNotRunning;
        assert!(err.to_string().contains("not running"));
    }

    #[test]
    fn test_client_error_connection() {
        let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "refused");
        let err = ClientError::Connection(io_err);
        assert!(err.to_string().contains("connect"));
    }

    #[test]
    fn test_client_error_send_display() {
        let io_err = io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe");
        let err = ClientError::Send(io_err);
        assert!(err.to_string().contains("send"));
    }

    #[test]
    fn test_client_error_receive_display() {
        let io_err = io::Error::new(io::ErrorKind::UnexpectedEof, "eof");
        let err = ClientError::Receive(io_err);
        assert!(err.to_string().contains("receive"));
    }

    #[test]
    fn test_client_error_parse_display() {
        let parse_err = serde_json::from_str::<IpcResponse>("not-json").unwrap_err();
        let err = ClientError::Parse(parse_err);
        assert!(err.to_string().contains("parse"));
    }

    #[tokio::test]
    async fn test_daemon_not_running() {
        let result = is_daemon_running().await;
        // Don't assert value as it depends on system state, just ensure it runs
        let _ = result;
    }

    #[tokio::test]
    async fn test_send_command_when_daemon_not_running() {
        let result = send_command(IpcCommand::Ping).await;
        match result {
            Ok(IpcResponse::Pong) | Ok(IpcResponse::Ok) => {}
            Err(ClientError::DaemonNotRunning) | Err(ClientError::Connection(_)) => {}
            other => panic!("Unexpected result: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_send_command_status_when_not_running() {
        let result = send_command(IpcCommand::Status).await;
        match result {
            Ok(_) => {} // Daemon is actually running
            Err(ClientError::DaemonNotRunning) | Err(ClientError::Connection(_)) => {}
            other => panic!("Unexpected result: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_connect_to_daemon_error() {
        // With no daemon running, connect should fail gracefully
        let result = connect_to_daemon().await;
        match result {
            Ok(_) => {} // Daemon happens to be running
            Err(ClientError::DaemonNotRunning) | Err(ClientError::Connection(_)) => {}
            other => panic!("Unexpected error: {:?}", other),
        }
    }

    #[test]
    fn test_all_client_error_variants_display() {
        let errors: Vec<ClientError> = vec![
            ClientError::Connection(io::Error::other("test")),
            ClientError::Send(io::Error::other("test")),
            ClientError::Receive(io::Error::other("test")),
            ClientError::Parse(serde_json::from_str::<IpcResponse>("x").unwrap_err()),
            ClientError::DaemonNotRunning,
        ];
        for err in &errors {
            let display = err.to_string();
            assert!(!display.is_empty(), "Error display should not be empty");
        }
        assert_eq!(errors.len(), 5);
    }
}
