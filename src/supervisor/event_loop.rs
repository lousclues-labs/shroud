// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! Supervisor event loop

use std::time::Instant;
use tokio::time::Duration;
use tracing::{debug, info, instrument, warn};

use crate::state::{Event, VpnState};
use crate::tray::VpnCommand;

/// Poll NetworkManager state every 2 seconds
pub const NM_POLL_INTERVAL_SECS: u64 = 2;

/// Health check interval when connected (seconds)
pub const HEALTH_CHECK_INTERVAL_SECS: u64 = 30;

/// Threshold for detecting a time jump (e.g., resume from sleep)
/// If more than 3x the poll interval has passed, we consider it a time jump
pub const TIME_JUMP_THRESHOLD_SECS: u64 = NM_POLL_INTERVAL_SECS * 3;

/// Cooldown period after a time jump event (prevents thrashing)
/// Only one wake event per cooldown window
pub const TIME_JUMP_COOLDOWN_SECS: u64 = 5;

impl super::VpnSupervisor {
    /// Run the supervisor's main loop
    #[instrument(skip(self))]
    pub async fn run(mut self) {
        info!("VPN supervisor starting with formal state machine");

        // Sync config to shared state on startup
        // IMPORTANT: Use actual iptables state for kill_switch, not just config
        {
            let mut state = self.shared_state.write().await;
            state.auto_reconnect = self.config_store.config.auto_reconnect;
            // Use actual kill switch state from iptables, not config
            // The kill_switch was already synced in VpnSupervisor::new()
            state.kill_switch = self.kill_switch.is_enabled();
        }

        // Initial connection refresh and state sync - do this BEFORE enabling kill switch
        self.refresh_connections().await;
        self.initial_nm_sync().await;
        self.timing.last_poll_time = Instant::now();

        // Kill switch reconciliation after NM sync:
        // - If rules already exist (detected by sync_state in constructor), ensure shared state matches
        // - If config says enabled + VPN is connected but no rules, re-enable them
        // - If config says enabled but VPN not connected, defer until VPN connects
        if self.kill_switch.is_enabled() {
            info!("Kill switch rules detected on startup — preserving");
            let mut state = self.shared_state.write().await;
            state.kill_switch = true;
        } else if self.config_store.config.kill_switch_enabled {
            if matches!(self.machine.state, VpnState::Connected { .. }) {
                info!("Restoring kill switch from config (VPN already connected)");
                if let Err(e) = self.kill_switch.enable().await {
                    warn!("Failed to enable kill switch on startup: {}", e);
                } else {
                    let mut state = self.shared_state.write().await;
                    state.kill_switch = true;
                }
            } else {
                info!("Kill switch enabled in config but VPN not connected - will enable when VPN connects");
            }
        }

        // Migration: if autostart is enabled but auto_connect is not, the user
        // upgraded from a version before auto_connect existed. Enable it so their
        // "start on login" actually connects on login.
        if crate::autostart::Autostart::is_enabled() && !self.config_store.config.auto_connect {
            info!("Migration: autostart enabled but auto_connect disabled — enabling auto_connect");
            self.config_store.config.auto_connect = true;
            self.config_store.save();
        }

        // Auto-connect on startup (desktop mode)
        // If auto_connect is enabled, connect to last_server (or first available VPN).
        // This gives "start on login = protect on login" behavior when paired with
        // `shroud autostart on`.
        if matches!(self.machine.state, VpnState::Disconnected)
            && self.config_store.config.auto_connect
        {
            // Wait for NetworkManager to finish loading VPN profiles.
            // On login, NM may still be bringing up interfaces — the initial
            // refresh_connections() above may have returned an empty list.
            tokio::time::sleep(Duration::from_secs(3)).await;
            self.refresh_connections().await;

            let connections = self.shared_state.read().await.connections.clone();

            // Determine target: prefer last_server, fall back to first available VPN
            let target_server = self
                .config_store
                .config
                .last_server
                .as_ref()
                .filter(|s| !s.is_empty() && connections.iter().any(|c| c == *s))
                .cloned()
                .or_else(|| {
                    if connections.is_empty() {
                        None
                    } else {
                        warn!(
                            "auto_connect: last_server not set or not found, using first available VPN: {}",
                            connections[0]
                        );
                        Some(connections[0].clone())
                    }
                });

            match target_server {
                Some(server) => {
                    info!("Auto-connecting to: {}", server);
                    self.tray
                        .notify("Shroud", &format!("Auto-connecting to {}...", server));
                    self.handle_connect(&server).await;
                }
                None => {
                    warn!("auto_connect enabled but no VPN connections found in NetworkManager");
                    self.tray.notify(
                        "Shroud",
                        "Auto-connect enabled but no VPN connections configured",
                    );
                }
            }
        }

        // Update tray with initial state
        self.tray.update(&self.shared_state);

        if self.config_store.is_first_run && !crate::autostart::Autostart::is_enabled() {
            info!("First run detected and autostart not enabled");
            self.tray.notify(
                "Shroud",
                "Tip: Run 'shroud autostart on' to start automatically on login",
            );
        }

        // Use health check interval from config (0 = disabled)
        let health_checks_enabled = self.config_store.config.health_check_interval_secs > 0;
        let health_interval = if health_checks_enabled {
            self.config_store.config.health_check_interval_secs
        } else {
            HEALTH_CHECK_INTERVAL_SECS // interval is created but never fires (guarded below)
        };

        // Create an interval for NM polling
        let mut nm_poll_interval =
            tokio::time::interval(Duration::from_secs(NM_POLL_INTERVAL_SECS));

        // Create an interval for health checks (only runs when connected)
        let mut health_check_interval = tokio::time::interval(Duration::from_secs(health_interval));

        loop {
            tokio::select! {
                // Handle commands from the tray
                Some(cmd) = self.rx.recv() => {
                    debug!("Received command: {:?}", cmd);
                    match cmd {
                        VpnCommand::Connect(server) => {
                            self.handle_connect(&server).await;
                        }
                        VpnCommand::Disconnect => {
                            self.handle_disconnect().await;
                        }
                        VpnCommand::ToggleAutoReconnect => {
                            self.toggle_auto_reconnect().await;
                        }
                        VpnCommand::ToggleKillSwitch => {
                            self.toggle_kill_switch().await;
                        }
                        VpnCommand::ToggleAutostart => {
                            self.toggle_autostart().await;
                        }
                        VpnCommand::ToggleDebugLogging => {
                            self.toggle_debug_logging().await;
                        }
                        VpnCommand::OpenLogFile => {
                            self.open_log_file();
                        }
                        VpnCommand::RefreshConnections => {
                            self.refresh_connections().await;
                        }
                        VpnCommand::Restart => {
                            self.handle_restart().await;
                            if self.exit_state.should_exit {
                                info!("Exiting due to: {:?}", self.exit_state.reason);
                                self.graceful_shutdown().await;
                                return;
                            }
                        }
                        VpnCommand::Quit => {
                            self.handle_quit().await;
                            if self.exit_state.should_exit {
                                info!("Exiting due to: {:?}", self.exit_state.reason);
                                self.graceful_shutdown().await;
                                return;
                            }
                        }
                    }
                }

                // Handle D-Bus events from NetworkManager (real-time)
                Some(event) = self.dbus_rx.recv() => {
                    self.handle_dbus_event(event).await;
                }

                // Handle IPC commands
                Some((cmd, response_tx)) = self.ipc_rx.recv() => {
                    self.handle_ipc_command(cmd, response_tx).await;
                    if self.exit_state.should_exit {
                        info!("Exiting due to: {:?}", self.exit_state.reason);
                        self.graceful_shutdown().await;
                        return;
                    }
                }

                // Poll NetworkManager state periodically (fallback/backup)
                _ = nm_poll_interval.tick() => {
                    let elapsed = self.timing.last_poll_time.elapsed();
                    if elapsed > Duration::from_secs(TIME_JUMP_THRESHOLD_SECS) {
                        // Time jump detected - check if we're in cooldown period
                        let should_dispatch = match self.timing.last_wake_event {
                            Some(last) => last.elapsed().as_secs() >= TIME_JUMP_COOLDOWN_SECS,
                            None => true,
                        };

                        if should_dispatch {
                            warn!(
                                "Time jump detected ({:.1}s since last poll), dispatching Wake event after delay",
                                elapsed.as_secs_f32()
                            );

                            // Suspend health checks during wake to avoid false positives
                            self.health_checker.suspend(Duration::from_secs(10));
                            self.timing.last_wake_event = Some(Instant::now());

                            // Mark that we need a wake resync — handled in the next
                            // poll cycle rather than blocking the event loop with a
                            // 2-second sleep that prevents IPC/tray/D-Bus processing.
                            self.dispatch(Event::Wake);
                            self.force_state_resync().await;
                        } else {
                            debug!(
                                "Time jump detected but in cooldown ({:.1}s since last wake event)",
                                self.timing.last_wake_event.unwrap().elapsed().as_secs_f32()
                            );
                        }
                    } else {
                        // Regular poll - check for multiple VPNs and sync state
                        self.poll_nm_state().await;
                    }
                    self.timing.last_poll_time = Instant::now();
                }

                // Run health checks when connected (disabled when health_check_interval_secs = 0)
                _ = health_check_interval.tick(), if health_checks_enabled => {
                    self.run_health_check().await;
                }
            }
        }
    }
}
