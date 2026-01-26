//! CLI server for receiving commands from clients
//!
//! Runs as part of the daemon, listening on a Unix socket for CLI commands.

use log::{debug, error, info, warn};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, RwLock};

use crate::cli::commands::{CliCommand, CliRequest, CliResponse};
use crate::config::Config;
use crate::logging;
use crate::tray::{SharedState, VpnCommand};

/// CLI server that listens for commands
pub struct CliServer {
    listener: UnixListener,
    socket_path: PathBuf,
}

/// Get the socket path
pub fn get_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("shroud.sock")
    } else {
        // Fallback using UID
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/tmp/shroud-{}.sock", uid))
    }
}

impl CliServer {
    /// Create a new CLI server
    pub async fn new() -> std::io::Result<Self> {
        let socket_path = get_socket_path();

        // Remove stale socket if exists
        if socket_path.exists() {
            // Try to check if another instance is running
            if is_socket_alive(&socket_path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "Another Shroud instance is already running",
                ));
            }
            std::fs::remove_file(&socket_path)?;
        }

        // Create parent directory if needed
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(&socket_path)?;

        // Set socket permissions (owner only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
        }

        info!("CLI server listening on {:?}", socket_path);

        Ok(Self {
            listener,
            socket_path,
        })
    }

    /// Accept a new connection
    pub async fn accept(&self) -> std::io::Result<UnixStream> {
        let (stream, _) = self.listener.accept().await?;
        Ok(stream)
    }

    /// Get the socket path
    #[allow(dead_code)]
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    /// Clean up the socket file
    pub fn cleanup(&self) {
        if self.socket_path.exists() {
            if let Err(e) = std::fs::remove_file(&self.socket_path) {
                warn!("Failed to remove socket file: {}", e);
            } else {
                debug!("Removed socket file");
            }
        }
    }
}

impl Drop for CliServer {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Check if a socket is alive (another instance is running)
fn is_socket_alive(path: &PathBuf) -> bool {
    use std::os::unix::net::UnixStream;
    UnixStream::connect(path).is_ok()
}

/// Handle a CLI connection
pub async fn handle_cli_connection(
    stream: UnixStream,
    cmd_tx: mpsc::Sender<VpnCommand>,
    shared_state: Arc<RwLock<SharedState>>,
    config: Arc<RwLock<Config>>,
    start_time: Instant,
) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Read request (one line of JSON)
    match reader.read_line(&mut line).await {
        Ok(0) => return, // Connection closed
        Ok(_) => {}
        Err(e) => {
            warn!("Failed to read CLI request: {}", e);
            return;
        }
    }

    debug!("CLI request: {}", line.trim());

    // Parse and handle request
    let response = match serde_json::from_str::<CliRequest>(&line) {
        Ok(request) => handle_request(request, cmd_tx, shared_state, config, start_time).await,
        Err(e) => CliResponse::error(
            String::new(),
            "parse_error",
            &format!("Invalid request: {}", e),
        ),
    };

    // Send response
    let response_json = match serde_json::to_string(&response) {
        Ok(json) => json + "\n",
        Err(e) => {
            error!("Failed to serialize response: {}", e);
            return;
        }
    };

    debug!("CLI response: {}", response_json.trim());

    if let Err(e) = writer.write_all(response_json.as_bytes()).await {
        warn!("Failed to send CLI response: {}", e);
    }
}

/// Handle a parsed CLI request
async fn handle_request(
    request: CliRequest,
    cmd_tx: mpsc::Sender<VpnCommand>,
    shared_state: Arc<RwLock<SharedState>>,
    config: Arc<RwLock<Config>>,
    start_time: Instant,
) -> CliResponse {
    let request_id = request.request_id.clone();

    match request.command {
        // Connection management
        CliCommand::Connect { name } => {
            if let Err(e) = cmd_tx.send(VpnCommand::Connect(name.clone())).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            CliResponse::success(
                request_id,
                json!({"message": format!("Connecting to {}", name)}),
            )
        }

        CliCommand::Disconnect => {
            if let Err(e) = cmd_tx.send(VpnCommand::Disconnect).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            CliResponse::success(request_id, json!({"message": "Disconnecting"}))
        }

        CliCommand::Reconnect => {
            let state = shared_state.read().await;
            if let Some(server) = state.state.server_name() {
                let server = server.to_string();
                drop(state);
                // Disconnect then connect
                let _ = cmd_tx.send(VpnCommand::Disconnect).await;
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                let _ = cmd_tx.send(VpnCommand::Connect(server.clone())).await;
                CliResponse::success(
                    request_id,
                    json!({"message": format!("Reconnecting to {}", server)}),
                )
            } else {
                CliResponse::error(request_id, "not_connected", "Not connected to any VPN")
            }
        }

        CliCommand::Switch { name } => {
            let state = shared_state.read().await;
            if state.state.server_name().is_some() {
                drop(state);
                // Disconnect then connect to new server
                let _ = cmd_tx.send(VpnCommand::Disconnect).await;
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                let _ = cmd_tx.send(VpnCommand::Connect(name.clone())).await;
                CliResponse::success(
                    request_id,
                    json!({"message": format!("Switching to {}", name)}),
                )
            } else {
                // Not connected, just connect
                let _ = cmd_tx.send(VpnCommand::Connect(name.clone())).await;
                CliResponse::success(
                    request_id,
                    json!({"message": format!("Connecting to {}", name)}),
                )
            }
        }

        // Status and information
        CliCommand::Status => {
            let state = shared_state.read().await;
            let config = config.read().await;
            CliResponse::success(
                request_id,
                json!({
                    "state": state.state.name(),
                    "connection": state.state.server_name(),
                    "kill_switch": state.kill_switch,
                    "auto_reconnect": state.auto_reconnect,
                    "debug_logging": state.debug_logging,
                    "dns_mode": format!("{:?}", config.dns_mode).to_lowercase(),
                    "ipv6_mode": format!("{:?}", config.ipv6_mode).to_lowercase(),
                }),
            )
        }

        CliCommand::List => {
            let state = shared_state.read().await;
            CliResponse::success(
                request_id,
                json!({
                    "connections": state.connections,
                    "current": state.state.server_name(),
                }),
            )
        }

        // Kill switch
        CliCommand::KillSwitchOn => {
            let state = shared_state.read().await;
            if state.kill_switch {
                return CliResponse::success(
                    request_id,
                    json!({"message": "Kill switch already enabled"}),
                );
            }
            drop(state);
            if let Err(e) = cmd_tx.send(VpnCommand::ToggleKillSwitch).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            CliResponse::success(request_id, json!({"message": "Kill switch enabled"}))
        }

        CliCommand::KillSwitchOff => {
            let state = shared_state.read().await;
            if !state.kill_switch {
                return CliResponse::success(
                    request_id,
                    json!({"message": "Kill switch already disabled"}),
                );
            }
            drop(state);
            if let Err(e) = cmd_tx.send(VpnCommand::ToggleKillSwitch).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            CliResponse::success(request_id, json!({"message": "Kill switch disabled"}))
        }

        CliCommand::KillSwitchToggle => {
            if let Err(e) = cmd_tx.send(VpnCommand::ToggleKillSwitch).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            let state = shared_state.read().await;
            let new_state = !state.kill_switch;
            CliResponse::success(
                request_id,
                json!({"message": format!("Kill switch {}", if new_state { "enabled" } else { "disabled" })}),
            )
        }

        CliCommand::KillSwitchStatus => {
            let state = shared_state.read().await;
            let config = config.read().await;
            CliResponse::success(
                request_id,
                json!({
                    "enabled": state.kill_switch,
                    "feature": "Kill switch",
                    "dns_mode": format!("{:?}", config.dns_mode).to_lowercase(),
                    "ipv6_mode": format!("{:?}", config.ipv6_mode).to_lowercase(),
                }),
            )
        }

        // Auto-reconnect
        CliCommand::AutoReconnectOn => {
            let state = shared_state.read().await;
            if state.auto_reconnect {
                return CliResponse::success(
                    request_id,
                    json!({"message": "Auto-reconnect already enabled"}),
                );
            }
            drop(state);
            if let Err(e) = cmd_tx.send(VpnCommand::ToggleAutoReconnect).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            CliResponse::success(request_id, json!({"message": "Auto-reconnect enabled"}))
        }

        CliCommand::AutoReconnectOff => {
            let state = shared_state.read().await;
            if !state.auto_reconnect {
                return CliResponse::success(
                    request_id,
                    json!({"message": "Auto-reconnect already disabled"}),
                );
            }
            drop(state);
            if let Err(e) = cmd_tx.send(VpnCommand::ToggleAutoReconnect).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            CliResponse::success(request_id, json!({"message": "Auto-reconnect disabled"}))
        }

        CliCommand::AutoReconnectToggle => {
            if let Err(e) = cmd_tx.send(VpnCommand::ToggleAutoReconnect).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            let state = shared_state.read().await;
            let new_state = !state.auto_reconnect;
            CliResponse::success(
                request_id,
                json!({"message": format!("Auto-reconnect {}", if new_state { "enabled" } else { "disabled" })}),
            )
        }

        CliCommand::AutoReconnectStatus => {
            let state = shared_state.read().await;
            CliResponse::success(
                request_id,
                json!({
                    "enabled": state.auto_reconnect,
                    "feature": "Auto-reconnect",
                }),
            )
        }

        // Debug
        CliCommand::DebugOn => {
            let state = shared_state.read().await;
            if state.debug_logging {
                return CliResponse::success(
                    request_id,
                    json!({"message": "Debug logging already enabled"}),
                );
            }
            drop(state);
            if let Err(e) = cmd_tx.send(VpnCommand::ToggleDebugLogging).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            CliResponse::success(
                request_id,
                json!({
                    "message": "Debug logging enabled",
                    "log_path": logging::log_directory().join("debug.log").display().to_string(),
                }),
            )
        }

        CliCommand::DebugOff => {
            let state = shared_state.read().await;
            if !state.debug_logging {
                return CliResponse::success(
                    request_id,
                    json!({"message": "Debug logging already disabled"}),
                );
            }
            drop(state);
            if let Err(e) = cmd_tx.send(VpnCommand::ToggleDebugLogging).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            CliResponse::success(request_id, json!({"message": "Debug logging disabled"}))
        }

        CliCommand::DebugLogPath => {
            let log_path = logging::log_directory().join("debug.log");
            CliResponse::success(
                request_id,
                json!({"log_path": log_path.display().to_string()}),
            )
        }

        CliCommand::DebugDump => {
            let state = shared_state.read().await;
            let config = config.read().await;
            CliResponse::success(
                request_id,
                json!({
                    "state": {
                        "vpn_state": state.state.name(),
                        "server": state.state.server_name(),
                        "kill_switch": state.kill_switch,
                        "auto_reconnect": state.auto_reconnect,
                        "debug_logging": state.debug_logging,
                        "connections": state.connections,
                    },
                    "config": {
                        "version": config.version,
                        "auto_reconnect": config.auto_reconnect,
                        "kill_switch_enabled": config.kill_switch_enabled,
                        "dns_mode": format!("{:?}", config.dns_mode).to_lowercase(),
                        "ipv6_mode": format!("{:?}", config.ipv6_mode).to_lowercase(),
                        "health_check_interval_secs": config.health_check_interval_secs,
                        "max_reconnect_attempts": config.max_reconnect_attempts,
                        "last_server": config.last_server,
                    },
                    "daemon": {
                        "pid": std::process::id(),
                        "version": env!("CARGO_PKG_VERSION"),
                        "uptime_seconds": start_time.elapsed().as_secs(),
                    },
                }),
            )
        }

        // Daemon control
        CliCommand::Ping => CliResponse::success(
            request_id,
            json!({
                "status": "running",
                "pid": std::process::id(),
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_seconds": start_time.elapsed().as_secs(),
            }),
        ),

        CliCommand::Refresh => {
            if let Err(e) = cmd_tx.send(VpnCommand::RefreshConnections).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            CliResponse::success(request_id, json!({"message": "Refreshing VPN connections"}))
        }

        CliCommand::Quit => {
            if let Err(e) = cmd_tx.send(VpnCommand::Quit).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            CliResponse::success(request_id, json!({"message": "Shutting down"}))
        }

        CliCommand::Restart => {
            // Send restart command instead of quit
            if let Err(e) = cmd_tx.send(VpnCommand::Restart).await {
                return CliResponse::error(request_id, "internal_error", &e.to_string());
            }
            CliResponse::success(request_id, json!({"message": "Restarting..."}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_socket_path() {
        let path = get_socket_path();
        assert!(path.to_string_lossy().contains("shroud.sock"));
    }
}
