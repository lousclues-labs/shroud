//! Supervisor event loop

use log::{debug, info, warn};
use std::time::Instant;
use tokio::time::Duration;

use crate::state::{Event, VpnState};
use crate::tray::VpnCommand;

/// Poll NetworkManager state every 2 seconds
pub const NM_POLL_INTERVAL_SECS: u64 = 2;

/// Health check interval when connected (seconds)
pub const HEALTH_CHECK_INTERVAL_SECS: u64 = 30;

impl super::VpnSupervisor {
    /// Run the supervisor's main loop
    pub async fn run(mut self) {
        info!("VPN supervisor starting with formal state machine");

        // Sync config to shared state on startup
        {
            let mut state = self.shared_state.write().await;
            state.auto_reconnect = self.app_config.auto_reconnect;
            state.kill_switch = self.app_config.kill_switch_enabled;
        }

        // Initial connection refresh and state sync - do this BEFORE enabling kill switch
        self.refresh_connections().await;
        self.initial_nm_sync().await;
        self.last_poll_time = Instant::now();

        // Only restore kill switch if VPN is already connected (avoid blocking VPN connection on startup)
        if self.app_config.kill_switch_enabled {
            if matches!(self.machine.state, VpnState::Connected { .. }) {
                info!("Restoring kill switch from config (VPN already connected)");
                if let Err(e) = self.kill_switch.enable().await {
                    warn!("Failed to enable kill switch on startup: {}", e);
                }
            } else {
                info!("Kill switch enabled in config but VPN not connected - will enable when VPN connects");
            }
        }

        // Use health check interval from config
        let health_interval = if self.app_config.health_check_interval_secs > 0 {
            self.app_config.health_check_interval_secs
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
                }

                // Poll NetworkManager state periodically (fallback/backup)
                _ = nm_poll_interval.tick() => {
                    let elapsed = self.last_poll_time.elapsed();
                    if elapsed > Duration::from_secs(NM_POLL_INTERVAL_SECS * 3) {
                        // Time jump detected - dispatch Wake event
                        warn!(
                            "Time jump detected ({:.1}s since last poll), dispatching Wake event",
                            elapsed.as_secs_f32()
                        );
                        self.dispatch(Event::Wake);
                        self.force_state_resync().await;
                    } else {
                        // Regular poll - check for multiple VPNs and sync state
                        self.poll_nm_state().await;
                    }
                    self.last_poll_time = Instant::now();
                }

                // Run health checks when connected
                _ = health_check_interval.tick() => {
                    self.run_health_check().await;
                }
            }
        }
    }
}
