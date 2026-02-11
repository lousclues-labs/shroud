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

/// Delay before dispatching wake event (allows system to stabilize)
pub const WAKE_EVENT_DELAY_MS: u64 = 2000;

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

        // Update tray with initial state
        self.tray.update(&self.shared_state);

        if self.config_store.is_first_run && !crate::autostart::Autostart::is_enabled() {
            info!("First run detected and autostart not enabled");
            self.tray.notify(
                "Shroud",
                "Tip: Run 'shroud autostart on' to start automatically on login",
            );
        }

        // Use health check interval from config
        let health_interval = if self.config_store.config.health_check_interval_secs > 0 {
            self.config_store.config.health_check_interval_secs
        } else {
            HEALTH_CHECK_INTERVAL_SECS
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

                            // Delay before dispatching to let system stabilize
                            tokio::time::sleep(Duration::from_millis(WAKE_EVENT_DELAY_MS)).await;

                            // Suspend health checks during wake to avoid false positives
                            self.health_checker.suspend(Duration::from_secs(10));

                            self.dispatch(Event::Wake);
                            self.timing.last_wake_event = Some(Instant::now());
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

                // Run health checks when connected
                _ = health_check_interval.tick() => {
                    self.run_health_check().await;
                }
            }
        }
    }
}
