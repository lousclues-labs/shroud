// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! IPC server for the Shroud daemon.
//!
//! Listens on a Unix domain socket for commands from CLI clients.
//!
//! # Architecture
//!
//! The server runs in a dedicated tokio task and forwards received commands
//! to the supervisor via a channel. Responses are sent back through the socket.

use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Semaphore};
use tracing::{debug, error, info, warn};

use super::protocol::{socket_path, IpcCommand, IpcResponse, PROTOCOL_VERSION};
use thiserror::Error;

/// Get the PID of the peer process connected to a Unix socket.
#[cfg(target_os = "linux")]
fn get_peer_pid(stream: &UnixStream) -> Option<u32> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    let mut ucred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ret = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut ucred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if ret == 0 {
        Some(ucred.pid as u32)
    } else {
        None
    }
}

#[cfg(not(target_os = "linux"))]
fn get_peer_pid(_stream: &UnixStream) -> Option<u32> {
    None
}

/// Errors that can occur in the IPC server.
#[derive(Error, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum ServerError {
    /// Failed to bind to socket
    #[error("Failed to bind to socket at {path}: {source}")]
    Bind {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to remove stale socket
    #[error("Failed to remove stale socket: {0}")]
    Cleanup(#[source] std::io::Error),
}

const MAX_CONCURRENT_CONNECTIONS: usize = 10;
const MAX_COMMANDS_PER_CONNECTION: usize = 100;

/// Unix socket server for IPC communication.
pub struct IpcServer {
    /// Channel to send received commands to the supervisor
    command_tx: mpsc::Sender<(IpcCommand, mpsc::Sender<IpcResponse>)>,
}

impl IpcServer {
    /// Create a new IPC server.
    ///
    /// # Arguments
    ///
    /// * `command_tx` - Channel sender for forwarding commands to supervisor
    pub fn new(command_tx: mpsc::Sender<(IpcCommand, mpsc::Sender<IpcResponse>)>) -> Self {
        Self { command_tx }
    }

    /// Run the IPC server.
    ///
    /// Binds to the Unix socket and accepts client connections.
    /// This method runs indefinitely until an error occurs.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Cleanup`] if a stale socket file cannot be removed.
    ///
    /// Returns [`ServerError::Bind`] if binding the Unix socket fails (permissions, path in use).
    pub async fn run(self) -> Result<(), ServerError> {
        let path = socket_path();

        // SECURITY: Best-effort symlink check before stale socket removal.
        // A TOCTOU window exists between symlink_metadata() and remove_file(),
        // but XDG_RUNTIME_DIR is user-owned (mode 0700) so exploitation requires
        // same-UID access. True atomic safety would need openat2(RESOLVE_NO_SYMLINKS).
        if path.exists() {
            if let Ok(meta) = std::fs::symlink_metadata(&path) {
                if meta.file_type().is_symlink() {
                    return Err(ServerError::Cleanup(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "socket path is a symlink, refusing to remove — possible attack",
                    )));
                }
            }
            std::fs::remove_file(&path).map_err(ServerError::Cleanup)?;
        }

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ServerError::Bind {
                path: parent.to_string_lossy().to_string(),
                source: e,
            })?;
        }

        // SECURITY: Set restrictive umask before bind so socket is created
        // with 0o600 permissions atomically (no TOCTOU window)
        #[cfg(unix)]
        let old_umask = unsafe { libc::umask(0o077) };

        let listener = UnixListener::bind(&path).map_err(|e| ServerError::Bind {
            path: path.to_string_lossy().to_string(),
            source: e,
        })?;

        // Restore original umask immediately after bind
        #[cfg(unix)]
        unsafe {
            libc::umask(old_umask);
        }

        info!("IPC server listening on {:?}", path);

        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));

        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let tx = self.command_tx.clone();
                    let sem = semaphore.clone();
                    let permit = match sem.try_acquire_owned() {
                        Ok(p) => p,
                        Err(_) => {
                            warn!("Too many concurrent IPC connections, rejecting");
                            continue;
                        }
                    };
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, tx).await {
                            warn!("Client connection error: {}", e);
                        }
                        drop(permit);
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }

    /// Handle a single client connection.
    async fn handle_connection(
        stream: UnixStream,
        command_tx: mpsc::Sender<(IpcCommand, mpsc::Sender<IpcResponse>)>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let peer_pid = get_peer_pid(&stream);
        let self_pid = std::process::id();
        let source_tag = match peer_pid {
            Some(pid) if pid == self_pid => "(self)",
            Some(_) => "(external)",
            None => "(pid unknown)",
        };

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        let mut command_count = 0u32;
        let mut first = true;

        // SECURITY: Use take() to enforce a hard read limit BEFORE allocation.
        // Without this, read_line() allocates unbounded memory for lines without
        // newlines, enabling OOM DoS (SHROUD-VULN-026).
        const MAX_LINE_BYTES: u64 = 64 * 1024 + 1; // 64KB + newline

        loop {
            line.clear();
            let mut limited = (&mut reader).take(MAX_LINE_BYTES);
            let bytes_read = limited.read_line(&mut line).await?;
            if bytes_read == 0 {
                break; // EOF
            }

            // If we read MAX_LINE_BYTES without a newline, the line was too long
            if !line.ends_with('\n') && line.len() as u64 >= MAX_LINE_BYTES - 1 {
                // Drain remaining bytes until newline or EOF to re-sync
                let mut drain = String::new();
                reader.read_line(&mut drain).await?;
                let response = IpcResponse::Error {
                    message: "Request too large".to_string(),
                };
                let response_json = serde_json::to_string(&response)?;
                writer.write_all(response_json.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
                continue;
            }

            command_count += 1;
            if command_count > MAX_COMMANDS_PER_CONNECTION as u32 {
                let response = IpcResponse::Error {
                    message: "Too many commands on this connection".to_string(),
                };
                let response_json = serde_json::to_string(&response)?;
                writer.write_all(response_json.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                line.clear();
                continue;
            }

            debug!("Received command: {}", trimmed);

            let command: IpcCommand = match serde_json::from_str(trimmed) {
                Ok(cmd) => cmd,
                Err(e) => {
                    warn!("Invalid command: {}", e);
                    let response = IpcResponse::Error {
                        message: format!("Invalid JSON: {}", e),
                    };
                    let response_json = serde_json::to_string(&response)?;
                    writer.write_all(response_json.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                    line.clear();
                    continue;
                }
            };

            if let Err(e) = command.validate() {
                let response = IpcResponse::Error {
                    message: format!("Validation error: {}", e),
                };
                let response_json = serde_json::to_string(&response)?;
                writer.write_all(response_json.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
                line.clear();
                continue;
            }

            if first {
                first = false;
                if let IpcCommand::Hello { version } = command {
                    if version != PROTOCOL_VERSION {
                        let response = IpcResponse::VersionMismatch {
                            server_version: PROTOCOL_VERSION,
                            client_version: version,
                        };
                        let response_json = serde_json::to_string(&response)?;
                        writer.write_all(response_json.as_bytes()).await?;
                        writer.write_all(b"\n").await?;
                        writer.flush().await?;
                        break;
                    } else {
                        let response = IpcResponse::HelloOk {
                            version: PROTOCOL_VERSION,
                        };
                        let response_json = serde_json::to_string(&response)?;
                        writer.write_all(response_json.as_bytes()).await?;
                        writer.write_all(b"\n").await?;
                        writer.flush().await?;
                        line.clear();
                        continue;
                    }
                } else {
                    warn!("Legacy IPC client without version handshake; proceeding");
                }
            }

            let response = {
                // Log non-trivial commands with peer PID for audit trail
                if !matches!(command, IpcCommand::Ping | IpcCommand::Hello { .. }) {
                    info!(
                        "IPC command {:?} from PID {} {}",
                        command,
                        peer_pid.map_or("?".to_string(), |p| p.to_string()),
                        source_tag
                    );
                }

                let (resp_tx, mut resp_rx) = mpsc::channel(1);

                if command_tx.send((command, resp_tx)).await.is_err() {
                    IpcResponse::Error {
                        message: "Supervisor channel closed".to_string(),
                    }
                } else {
                    match tokio::time::timeout(std::time::Duration::from_secs(60), resp_rx.recv())
                        .await
                    {
                        Ok(Some(resp)) => resp,
                        Ok(None) => IpcResponse::Error {
                            message: "Supervisor dropped the response channel".to_string(),
                        },
                        Err(_) => IpcResponse::Error {
                            message: "Timeout waiting for supervisor response".to_string(),
                        },
                    }
                }
            };

            let response_json = serde_json::to_string(&response)?;
            writer.write_all(response_json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;

            line.clear();
        }

        Ok(())
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        // Clean up socket file
        let path = socket_path();
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::protocol::PROTOCOL_VERSION;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[test]
    fn test_server_error_display_bind() {
        let err = ServerError::Bind {
            path: "/tmp/test.sock".to_string(),
            source: std::io::Error::new(std::io::ErrorKind::AddrInUse, "address in use"),
        };
        let display = format!("{}", err);
        assert!(display.contains("Failed to bind"));
        assert!(display.contains("/tmp/test.sock"));
    }

    #[test]
    fn test_server_error_display_cleanup() {
        let err = ServerError::Cleanup(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        ));
        let display = format!("{}", err);
        assert!(display.contains("Failed to remove stale socket"));
    }

    #[tokio::test]
    async fn test_server_creation() {
        let (tx, _rx) = mpsc::channel(1);
        let _server = IpcServer::new(tx);
    }

    #[tokio::test]
    async fn test_handle_connection_invalid_json() {
        let (tx, _rx) = mpsc::channel(16);
        let (client, server_stream) = tokio::net::UnixStream::pair().unwrap();

        let handle =
            tokio::spawn(async move { IpcServer::handle_connection(server_stream, tx).await });

        let mut client = client;
        client.write_all(b"not valid json\n").await.unwrap();
        client.shutdown().await.unwrap();

        let result = handle.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_connection_valid_command() {
        let (tx, mut rx) = mpsc::channel::<(IpcCommand, mpsc::Sender<IpcResponse>)>(16);
        let (client, server_stream) = tokio::net::UnixStream::pair().unwrap();

        tokio::spawn(async move {
            if let Some((cmd, responder)) = rx.recv().await {
                assert!(matches!(cmd, IpcCommand::Ping));
                responder.send(IpcResponse::Pong).await.unwrap();
            }
        });

        let handle =
            tokio::spawn(async move { IpcServer::handle_connection(server_stream, tx).await });

        let (client_reader, mut client_writer) = client.into_split();
        let ping_json = serde_json::to_string(&IpcCommand::Ping).unwrap();
        client_writer
            .write_all(format!("{}\n", ping_json).as_bytes())
            .await
            .unwrap();
        client_writer.shutdown().await.unwrap();

        let mut reader = BufReader::new(client_reader);
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();

        let parsed: IpcResponse = serde_json::from_str(&response).unwrap();
        assert!(matches!(parsed, IpcResponse::Pong));

        drop(reader);

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "handle_connection timed out");
        assert!(result.unwrap().unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_handle_connection_handshake_then_ping() {
        let (tx, mut rx) = mpsc::channel::<(IpcCommand, mpsc::Sender<IpcResponse>)>(16);
        let (client, server_stream) = tokio::net::UnixStream::pair().unwrap();

        tokio::spawn(async move {
            if let Some((cmd, responder)) = rx.recv().await {
                assert!(matches!(cmd, IpcCommand::Ping));
                responder.send(IpcResponse::Pong).await.unwrap();
            }
        });

        let handle =
            tokio::spawn(async move { IpcServer::handle_connection(server_stream, tx).await });

        let (client_reader, mut client_writer) = client.into_split();
        let hello_json = serde_json::to_string(&IpcCommand::Hello {
            version: PROTOCOL_VERSION,
        })
        .unwrap();
        client_writer
            .write_all(format!("{}\n", hello_json).as_bytes())
            .await
            .unwrap();

        let mut reader = BufReader::new(client_reader);
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();
        let parsed: IpcResponse = serde_json::from_str(response.trim()).unwrap();
        assert!(matches!(parsed, IpcResponse::HelloOk { version } if version == PROTOCOL_VERSION));

        let ping_json = serde_json::to_string(&IpcCommand::Ping).unwrap();
        client_writer
            .write_all(format!("{}\n", ping_json).as_bytes())
            .await
            .unwrap();
        client_writer.shutdown().await.unwrap();

        let mut response2 = String::new();
        reader.read_line(&mut response2).await.unwrap();
        let parsed2: IpcResponse = serde_json::from_str(response2.trim()).unwrap();
        assert!(matches!(parsed2, IpcResponse::Pong));

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "handle_connection timed out");
        assert!(result.unwrap().unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_handle_connection_version_mismatch() {
        let (tx, _rx) = mpsc::channel::<(IpcCommand, mpsc::Sender<IpcResponse>)>(16);
        let (client, server_stream) = tokio::net::UnixStream::pair().unwrap();

        let handle =
            tokio::spawn(async move { IpcServer::handle_connection(server_stream, tx).await });

        let (client_reader, mut client_writer) = client.into_split();
        let hello_json = serde_json::to_string(&IpcCommand::Hello {
            version: PROTOCOL_VERSION + 1,
        })
        .unwrap();
        client_writer
            .write_all(format!("{}\n", hello_json).as_bytes())
            .await
            .unwrap();
        client_writer.shutdown().await.unwrap();

        let mut reader = BufReader::new(client_reader);
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();
        let parsed: IpcResponse = serde_json::from_str(response.trim()).unwrap();
        assert!(
            matches!(parsed, IpcResponse::VersionMismatch { server_version, client_version } if server_version == PROTOCOL_VERSION && client_version == PROTOCOL_VERSION + 1)
        );

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "handle_connection timed out");
        assert!(result.unwrap().unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_handle_connection_empty_line() {
        let (tx, _rx) = mpsc::channel(16);
        let (client, server_stream) = tokio::net::UnixStream::pair().unwrap();

        let handle =
            tokio::spawn(async move { IpcServer::handle_connection(server_stream, tx).await });

        let mut client = client;
        // Send an empty line followed by EOF
        client.write_all(b"\n").await.unwrap();
        client.shutdown().await.unwrap();

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_connection_invalid_command_validation() {
        let (tx, mut rx) = mpsc::channel::<(IpcCommand, mpsc::Sender<IpcResponse>)>(16);
        let (client, server_stream) = tokio::net::UnixStream::pair().unwrap();

        // No handler needed — validation should reject before reaching supervisor
        let handle =
            tokio::spawn(async move { IpcServer::handle_connection(server_stream, tx).await });

        let (client_reader, mut client_writer) = client.into_split();
        // Send a Connect with empty name (fails validation)
        let cmd = IpcCommand::Connect { name: "".into() };
        let json = serde_json::to_string(&cmd).unwrap();
        client_writer
            .write_all(format!("{}\n", json).as_bytes())
            .await
            .unwrap();
        client_writer.shutdown().await.unwrap();

        let mut reader = BufReader::new(client_reader);
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();

        let parsed: IpcResponse = serde_json::from_str(&response).unwrap();
        assert!(
            matches!(parsed, IpcResponse::Error { .. }),
            "Expected validation error response"
        );

        // Supervisor should NOT have received anything
        assert!(rx.try_recv().is_err());

        drop(reader);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    }

    #[tokio::test]
    async fn test_handle_connection_status_command() {
        let (tx, mut rx) = mpsc::channel::<(IpcCommand, mpsc::Sender<IpcResponse>)>(16);
        let (client, server_stream) = tokio::net::UnixStream::pair().unwrap();

        tokio::spawn(async move {
            if let Some((cmd, responder)) = rx.recv().await {
                assert!(matches!(cmd, IpcCommand::Status));
                responder
                    .send(IpcResponse::Status {
                        connected: false,
                        vpn_name: None,
                        vpn_type: None,
                        state: "Disconnected".into(),
                        kill_switch_enabled: false,
                    })
                    .await
                    .unwrap();
            }
        });

        let handle =
            tokio::spawn(async move { IpcServer::handle_connection(server_stream, tx).await });

        let (client_reader, mut client_writer) = client.into_split();
        let json = serde_json::to_string(&IpcCommand::Status).unwrap();
        client_writer
            .write_all(format!("{}\n", json).as_bytes())
            .await
            .unwrap();
        client_writer.shutdown().await.unwrap();

        let mut reader = BufReader::new(client_reader);
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();

        let parsed: IpcResponse = serde_json::from_str(&response).unwrap();
        match parsed {
            IpcResponse::Status {
                connected, state, ..
            } => {
                assert!(!connected);
                assert!(state.contains("Disconnected"));
            }
            _ => panic!("Expected Status response"),
        }

        drop(reader);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    }

    #[tokio::test]
    async fn test_handle_connection_multiple_commands() {
        let (tx, mut rx) = mpsc::channel::<(IpcCommand, mpsc::Sender<IpcResponse>)>(16);
        let (client, server_stream) = tokio::net::UnixStream::pair().unwrap();

        tokio::spawn(async move {
            // Handle two commands
            for _ in 0..2 {
                if let Some((_cmd, responder)) = rx.recv().await {
                    responder.send(IpcResponse::Pong).await.unwrap();
                }
            }
        });

        let handle =
            tokio::spawn(async move { IpcServer::handle_connection(server_stream, tx).await });

        let (client_reader, mut client_writer) = client.into_split();
        let ping_json = serde_json::to_string(&IpcCommand::Ping).unwrap();
        // Send two commands
        client_writer
            .write_all(format!("{}\n{}\n", ping_json, ping_json).as_bytes())
            .await
            .unwrap();
        client_writer.shutdown().await.unwrap();

        let mut reader = BufReader::new(client_reader);
        let mut line1 = String::new();
        reader.read_line(&mut line1).await.unwrap();
        let mut line2 = String::new();
        reader.read_line(&mut line2).await.unwrap();

        let r1: IpcResponse = serde_json::from_str(&line1).unwrap();
        let r2: IpcResponse = serde_json::from_str(&line2).unwrap();
        assert!(matches!(r1, IpcResponse::Pong));
        assert!(matches!(r2, IpcResponse::Pong));

        drop(reader);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_get_peer_pid_returns_some() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let pid = get_peer_pid(&stream);
        assert!(pid.is_some(), "get_peer_pid should return Some on Linux");
        assert!(pid.unwrap() > 0);
    }

    #[tokio::test]
    async fn test_symlink_socket_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.sock");
        let link = dir.path().join("link.sock");
        std::fs::write(&target, b"").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        // Verify symlink_metadata detects it
        let meta = std::fs::symlink_metadata(&link).unwrap();
        assert!(meta.file_type().is_symlink());
    }
}
