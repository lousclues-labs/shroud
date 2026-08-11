// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Supervisor event-handler dispatchers.
//!
//! Method bodies for each event family live in dedicated submodules:
//! - [`nm`]: NetworkManager state/sync handlers
//! - [`health`]: health check handler
//! - [`user`]: user-initiated commands (connect/disconnect/toggles)
//! - [`system`]: lifecycle (quit/shutdown) and the `resolve_restart_path` helper
//!
//! This file keeps only the two dispatchers ([`handle_dbus_event`] for D-Bus
//! events and [`handle_ipc_command`] for IPC commands).

use tracing::{debug, info, instrument, warn};

use crate::daemon::lock::release_instance_lock;
use crate::dbus::NmEvent;
use crate::nm::{
    get_vpn_type as nm_get_vpn_type,
    list_vpn_connections_with_types as nm_list_vpn_connections_with_types,
};
use crate::state::{Event, TransitionReason, VpnState};

use super::{POST_DISCONNECT_GRACE_SECS, WAKE_RESYNC_GRACE_SECS};
use system::resolve_restart_path;

mod health;
mod nm;
mod system;
mod user;

impl super::VpnSupervisor {
    /// Handle D-Bus event from NetworkManager
    #[instrument(skip(self), fields(event = ?event))]
    pub(crate) async fn handle_dbus_event(&mut self, event: NmEvent) {
        debug!("Received D-Bus event: {:?}", event);

        // CRITICAL: Ignore ALL D-Bus events while a VPN switch is in progress
        // handle_connect manages everything during a switch - D-Bus events only cause interference
        if self.switch_ctx.in_progress {
            debug!("Ignoring D-Bus event during VPN switch: {:?}", event);
            return;
        }

        // Ignore the burst NetworkManager replays right after a wake resync.
        // Those events predate the state the resync just read from NM, so acting
        // on them flaps Connected -> Failed -> Connecting against a healthy tunnel.
        if let Some(resynced) = self.timing.last_wake_resync {
            if resynced.elapsed().as_secs() < WAKE_RESYNC_GRACE_SECS {
                debug!("Ignoring stale D-Bus event after wake resync: {:?}", event);
                return;
            }
            self.timing.last_wake_resync = None;
        }

        // CRITICAL: Ignore late deactivation events from VPN we recently switched FROM
        // D-Bus events can arrive after we've already connected to the new VPN
        if let Some(ref from_server) = self.switch_ctx.from {
            if let NmEvent::VpnDeactivated { ref name } = event {
                if name == from_server {
                    // Check if we're within the grace window after switch completed
                    if let Some(completed) = self.switch_ctx.completed_time {
                        if completed.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                            info!(
                                "Ignoring late deactivation event for switched-from VPN: {}",
                                name
                            );
                            return;
                        }
                    }
                    // Clear the switching_from after processing
                    self.switch_ctx.from = None;
                    self.switch_ctx.completed_time = None;
                }
            }
        }

        // Check if we're in grace period after intentional disconnect
        if let Some(disconnect_time) = self.timing.last_disconnect_time {
            if disconnect_time.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                debug!("Ignoring D-Bus event during grace period");
                return;
            } else {
                self.timing.last_disconnect_time = None;
            }
        }

        let auto_reconnect = self.shared_state.read().await.auto_reconnect;

        match event {
            NmEvent::VpnActivated { name } => {
                info!("D-Bus: VPN '{}' activated", name);

                // CRITICAL: If we already have a different VPN connected, disconnect the OLD one
                // Policy: newest VPN wins (the one that just activated)
                if let Some(current) = self.machine.state.server_name() {
                    if current != name {
                        info!("External VPN '{}' activated while connected to '{}' - disconnecting old VPN", name, current);
                        let old_vpn = current.to_string();
                        // Update our state to the new VPN first
                        self.dispatch(Event::NmVpnUp {
                            server: name.clone(),
                        });
                        self.sync_shared_state().await;
                        self.tray.update(&self.shared_state);
                        // Then disconnect the old one
                        if let Err(e) = self.nm.disconnect(&old_vpn).await {
                            warn!("Failed to disconnect old VPN '{}': {}", old_vpn, e);
                        }
                        self.tray
                            .notify("VPN Switched", &format!("Now connected to {}", name));
                        return;
                    }
                }

                // Also check for any other active VPNs in NetworkManager
                let all_active = self.nm.get_all_active_vpns().await;
                if all_active.len() > 1 {
                    info!(
                        "Multiple VPNs detected ({}) - cleaning up extras",
                        all_active.len()
                    );
                    for vpn in &all_active {
                        if vpn.name != name {
                            info!("Disconnecting extra VPN: {}", vpn.name);
                            let _ = self.nm.disconnect(&vpn.name).await;
                        }
                    }
                }

                self.dispatch(Event::NmVpnUp { server: name });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
            }
            NmEvent::VpnActivating { name } => {
                // Only update if we're not already connecting/connected to this VPN
                let dominated = matches!(
                    &self.machine.state,
                    VpnState::Connecting { server } | VpnState::Connected { server }
                        if server == &name
                );
                if !dominated {
                    info!("D-Bus: VPN '{}' activating (external)", name);
                    self.dispatch(Event::UserEnable { server: name });
                    self.sync_shared_state().await;
                    self.tray.update(&self.shared_state);
                } else {
                    debug!(
                        "D-Bus: ignoring activating event for '{}' (already {})",
                        name,
                        self.machine.state.name()
                    );
                }
            }
            NmEvent::VpnDeactivated { name } => {
                info!("D-Bus: VPN '{}' deactivated", name);

                // Check if this was our connected VPN
                if let Some(current) = self.machine.state.server_name() {
                    if current == name {
                        if auto_reconnect
                            && matches!(
                                self.machine.state,
                                VpnState::Connected { .. } | VpnState::Degraded { .. }
                            )
                        {
                            let server = name.clone();
                            self.dispatch(Event::NmVpnDown);
                            self.sync_shared_state().await;
                            self.tray.update(&self.shared_state);
                            self.tray
                                .notify("VPN Disconnected", "Connection dropped, reconnecting...");
                            self.attempt_reconnect(&server).await;
                        } else {
                            // Auto-reconnect disabled: go directly to Disconnected, not Reconnecting
                            self.machine
                                .set_state(VpnState::Disconnected, TransitionReason::VpnLost);
                            self.sync_shared_state().await;
                            self.tray.update(&self.shared_state);
                            self.tray
                                .notify("VPN Disconnected", &format!("Disconnected from {}", name));
                        }
                    }
                }
            }
            NmEvent::VpnFailed { name, reason } => {
                warn!("D-Bus: VPN '{}' failed: {}", name, reason);

                if auto_reconnect {
                    self.dispatch(Event::NmVpnDown);
                    self.sync_shared_state().await;
                    self.tray.update(&self.shared_state);
                    self.tray
                        .notify("VPN Failed", &format!("{}: {}", name, reason));
                    self.attempt_reconnect(&name).await;
                } else {
                    self.machine.set_state(
                        VpnState::Failed {
                            server: name,
                            reason,
                        },
                        TransitionReason::VpnLost,
                    );
                    self.sync_shared_state().await;
                    self.tray.update(&self.shared_state);
                }
            }
            NmEvent::ConnectivityChanged { connected } => {
                debug!("D-Bus: Connectivity changed: {}", connected);
                // Could trigger health check here
            }
        }
    }

    /// Handle IPC command
    pub(crate) async fn handle_ipc_command(
        &mut self,
        cmd: crate::ipc::IpcCommand,
        response_tx: tokio::sync::mpsc::Sender<crate::ipc::IpcResponse>,
    ) {
        use crate::ipc::{IpcCommand, IpcResponse, PROTOCOL_VERSION};

        let response = match cmd {
            IpcCommand::Hello { .. } => IpcResponse::Error {
                message: "Hello handshake handled by IPC server".to_string(),
            },
            IpcCommand::Version => IpcResponse::VersionInfo {
                binary_version: env!("CARGO_PKG_VERSION").to_string(),
                protocol_version: PROTOCOL_VERSION,
            },
            IpcCommand::Status => {
                let state = self.shared_state.read().await;
                let vpn_type = if let Some(name) = state.state.server_name() {
                    Some(nm_get_vpn_type(name).await.to_string())
                } else {
                    None
                };
                IpcResponse::Status {
                    connected: state.state.server_name().is_some(),
                    vpn_name: state.state.server_name().map(|s| s.to_string()),
                    vpn_type,
                    state: state.state.name().to_string(),
                    kill_switch_enabled: state.kill_switch,
                }
            }
            IpcCommand::List { vpn_type } => {
                let active = self.nm.get_all_active_vpns().await;
                let connections = nm_list_vpn_connections_with_types().await;
                let mut entries = Vec::new();

                for conn in connections {
                    let type_str = conn.vpn_type.to_string();
                    if let Some(filter) = vpn_type.as_deref() {
                        if filter != type_str {
                            continue;
                        }
                    }

                    let status = if active.iter().any(|a| a.name == conn.name) {
                        "connected"
                    } else {
                        "available"
                    };

                    entries.push(crate::ipc::protocol::VpnConnectionInfo {
                        name: conn.name,
                        vpn_type: type_str,
                        status: status.to_string(),
                    });
                }

                IpcResponse::Connections {
                    connections: entries,
                }
            }
            IpcCommand::Connect { name } => {
                self.handle_connect(&name).await;
                // Since connect is async and we don't wait for completion here (state machine does),
                // we return OK. The client can poll status.
                // Ideally we might want to wait, but the architecture seems fire-and-forget for commands
                IpcResponse::Ok // or maybe return "Connecting to X"
            }
            IpcCommand::Disconnect => {
                self.handle_disconnect().await;
                IpcResponse::Ok
            }
            IpcCommand::Switch { name } => {
                // Logic closer to handle_connect but ensuring switch logic
                self.handle_connect(&name).await;
                IpcResponse::Ok
            }
            IpcCommand::Reconnect => {
                // SECURITY: Use handle_connect() directly — it handles disconnecting
                // the old VPN internally while preserving the kill switch.
                // The old disconnect-sleep-connect pattern disabled the kill switch
                // during the 2-second gap (SHROUD-VULN-046).
                let can_reconnect = {
                    let state = self.shared_state.read().await;
                    state.state.server_name().is_some()
                };

                if can_reconnect {
                    let server = self
                        .shared_state
                        .read()
                        .await
                        .state
                        .server_name()
                        .unwrap()
                        .to_string();
                    self.handle_connect(&server).await;
                    IpcResponse::Ok
                } else {
                    let last_server = self.config_store.config.last_server.clone();
                    if let Some(server) = last_server {
                        self.handle_connect(&server).await;
                        IpcResponse::Ok
                    } else {
                        IpcResponse::Error {
                            message: "No VPN to reconnect to".to_string(),
                        }
                    }
                }
            }
            IpcCommand::KillSwitch { enable } => {
                // Skip if already in the desired state
                if self.kill_switch.is_enabled() == enable {
                    debug!(
                        "Kill switch already {}, skipping",
                        if enable { "enabled" } else { "disabled" }
                    );
                    let _ = response_tx
                        .send(IpcResponse::OkMessage {
                            message: format!(
                                "Kill switch already {}",
                                if enable { "enabled" } else { "disabled" }
                            ),
                        })
                        .await;
                    return;
                }

                let result = if enable {
                    self.kill_switch.enable().await
                } else {
                    self.kill_switch.disable().await
                };

                // Read ACTUAL state — don't trust Ok(()) alone
                let actual_enabled = self.kill_switch.is_enabled();

                match result {
                    Ok(()) => {
                        // Sync shared state to actual kill switch state
                        {
                            let mut state = self.shared_state.write().await;
                            state.kill_switch = actual_enabled;
                        }
                        self.config_store.config.kill_switch_enabled = actual_enabled;
                        self.config_store.save();
                        self.sync_shared_state().await;
                        IpcResponse::OkMessage {
                            message: format!(
                                "Kill switch {}",
                                if actual_enabled {
                                    "enabled"
                                } else {
                                    "disabled"
                                }
                            ),
                        }
                    }
                    Err(e) => IpcResponse::Error {
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::KillSwitchToggle => {
                self.toggle_kill_switch().await;
                // toggle_kill_switch updates state
                let state = self.shared_state.read().await;
                IpcResponse::OkMessage {
                    message: format!(
                        "Kill switch {}",
                        if state.kill_switch {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    ),
                }
            }
            IpcCommand::KillSwitchStatus => {
                let state = self.shared_state.read().await;
                IpcResponse::KillSwitchStatus {
                    enabled: state.kill_switch,
                }
            }
            IpcCommand::AutoReconnect { enable } => {
                self.config_store.config.auto_reconnect = enable;
                self.config_store.save();
                self.sync_shared_state().await;
                IpcResponse::Ok
            }
            IpcCommand::AutoReconnectToggle => {
                self.toggle_auto_reconnect().await;
                let state = self.shared_state.read().await;
                IpcResponse::OkMessage {
                    message: format!(
                        "Auto-reconnect {}",
                        if state.auto_reconnect {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    ),
                }
            }
            IpcCommand::AutoReconnectStatus => {
                let state = self.shared_state.read().await;
                IpcResponse::AutoReconnectStatus {
                    enabled: state.auto_reconnect,
                }
            }
            IpcCommand::Debug { enable } => {
                let success = if enable {
                    match crate::logging::enable_debug_logging() {
                        Ok(_) => true,
                        Err(e) => {
                            return {
                                let _ = response_tx.send(IpcResponse::Error { message: e }).await;
                            }
                        }
                    }
                } else {
                    crate::logging::disable_debug_logging();
                    true
                };

                if success {
                    let mut state = self.shared_state.write().await;
                    state.debug_logging = enable;
                    drop(state);
                    self.tray.update(&self.shared_state);
                }
                self.sync_shared_state().await;
                IpcResponse::Ok
            }
            IpcCommand::Ping => IpcResponse::Ok,
            IpcCommand::Refresh => {
                self.refresh_connections().await;
                IpcResponse::Ok
            }
            IpcCommand::Quit => {
                // handle_quit exits process, so we won't return...
                // But we should try to send response first?
                let _ = response_tx.send(IpcResponse::Ok).await;
                self.handle_quit().await;
                return;
            }
            IpcCommand::Restart => {
                use std::os::unix::process::CommandExt;
                info!("Restart requested via IPC");

                // NOTE: Do NOT disable kill switch before restart.
                // The new instance will adopt existing rules via sync_state().

                let exe_path = match resolve_restart_path() {
                    Ok(path) => path,
                    Err(message) => {
                        let _ = response_tx.send(IpcResponse::Error { message }).await;
                        return;
                    }
                };

                info!("Spawning new daemon instance: {:?}", exe_path);

                // SECURITY: Spawn with setsid, matching the tray restart path.
                // Spawn BEFORE releasing lock (SHROUD-VULN-031).
                let mut cmd = std::process::Command::new(&exe_path);
                cmd.stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());

                unsafe {
                    cmd.pre_exec(|| {
                        libc::setsid();
                        Ok(())
                    });
                }

                match cmd.spawn() {
                    Ok(child) => {
                        info!("Spawned new daemon (PID: {})", child.id());
                        // Release lock and socket so child can acquire them
                        release_instance_lock();
                        let sock = crate::ipc::protocol::socket_path();
                        let _ = std::fs::remove_file(&sock);
                        self.exit_state.request("restart");
                        IpcResponse::OkMessage {
                            message: "Restarting daemon...".to_string(),
                        }
                    }
                    Err(e) => IpcResponse::Error {
                        message: format!("Failed to spawn new instance: {}", e),
                    },
                }
            }
            IpcCommand::Reload => {
                info!("Configuration reload requested via IPC");
                match self.reload_configuration("IPC").await {
                    Ok(()) => IpcResponse::OkMessage {
                        message: "Configuration reloaded successfully".to_string(),
                    },
                    Err(e) => IpcResponse::Error {
                        message: format!("Failed to reload configuration: {}", e),
                    },
                }
            }
            IpcCommand::DebugLogPath => {
                let path = crate::logging::default_log_path();
                IpcResponse::DebugInfo {
                    log_path: Some(path.to_string_lossy().to_string()),
                    debug_enabled: crate::logging::is_debug_logging_enabled(),
                }
            }
            IpcCommand::DebugDump => {
                let state = self.shared_state.read().await;
                let machine_state = self.machine.state.name();
                let server = self.machine.state.server_name().map(|s| s.to_string());
                let kill_switch = self.kill_switch.is_enabled();
                let auto_reconnect = state.auto_reconnect;
                let debug_logging = crate::logging::is_debug_logging_enabled();
                let connections = state.connections.clone();
                let switching = self.switch_ctx.in_progress;
                let retries = self.machine.retries();
                drop(state);

                let dump = serde_json::json!({
                    "state": machine_state,
                    "server": server,
                    "kill_switch_enabled": kill_switch,
                    "auto_reconnect": auto_reconnect,
                    "debug_logging": debug_logging,
                    "connections": connections,
                    "switching_in_progress": switching,
                    "reconnect_retries": retries,
                    "reconnect_cancelled": self.timing.reconnect_cancelled,
                    "is_first_run": self.config_store.is_first_run,
                    "config": {
                        "max_reconnect_attempts": self.config_store.config.max_reconnect_attempts,
                        "health_check_interval_secs": self.config_store.config.health_check_interval_secs,
                        "health_degraded_threshold_ms": self.config_store.config.health_degraded_threshold_ms,
                        "health_check_endpoints": self.config_store.config.health_check_endpoints.clone(),
                        "dns_mode": format!("{}", self.config_store.config.dns_mode),
                        "ipv6_mode": format!("{:?}", self.config_store.config.ipv6_mode),
                        "block_doh": self.config_store.config.block_doh,
                    },
                });

                IpcResponse::OkMessage {
                    message: serde_json::to_string_pretty(&dump)
                        .unwrap_or_else(|_| "{}".to_string()),
                }
            }
        };

        let _ = response_tx.send(response).await;
    }
}
