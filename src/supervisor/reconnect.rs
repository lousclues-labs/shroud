//! Supervisor reconnection logic

use log::{debug, error, info, warn};
use std::time::Instant;
use tokio::time::{sleep, Duration};

use crate::nm::{
    connect as nm_connect, disconnect as nm_disconnect, get_active_vpn as nm_get_active_vpn,
    list_vpn_connections as nm_list_vpn_connections,
};
use crate::state::{Event, TransitionReason, VpnState};
use crate::tray::VpnCommand;

use super::{CONNECTION_VERIFY_DELAY_SECS, RECONNECT_BASE_DELAY_SECS, RECONNECT_MAX_DELAY_SECS};

impl super::VpnSupervisor {
    /// Attempt to reconnect with exponential backoff (triggered by connection drop)
    pub(crate) async fn attempt_reconnect(&mut self, connection_name: &str) {
        // Clear any previous cancellation flag
        self.reconnect_cancelled = false;

        // First, verify the connection still exists in NetworkManager
        let available_connections = nm_list_vpn_connections().await;
        if !available_connections.iter().any(|c| c == connection_name) {
            error!(
                "Cannot reconnect: VPN '{}' no longer exists in NetworkManager",
                connection_name
            );
            self.show_notification(
                "Reconnect Failed",
                &format!("VPN '{}' not found", connection_name),
            );
            self.dispatch(Event::NmVpnDown);
            self.sync_shared_state().await;
            self.update_tray();
            // Refresh connection list to update the tray menu
            self.refresh_connections().await;
            return;
        }

        let max_attempts = self.machine.max_retries();

        // NOTE: Kill switch stays enabled - VPN server IPs are already whitelisted
        // No need to disable/re-enable which would require sudo prompts

        let mut reconnect_succeeded = false;

        for attempt in 1..=max_attempts {
            // Check for cancellation before each attempt
            if self.reconnect_cancelled {
                info!("Reconnection cancelled by user");
                self.machine
                    .set_state(VpnState::Disconnected, TransitionReason::UserRequested);
                self.sync_shared_state().await;
                self.update_tray();
                return;
            }

            info!(
                "Reconnection attempt {}/{} for {}",
                attempt, max_attempts, connection_name
            );

            // Update state to Reconnecting
            self.machine.set_state(
                VpnState::Reconnecting {
                    server: connection_name.to_string(),
                    attempt,
                    max_attempts,
                },
                TransitionReason::Retrying,
            );
            self.sync_shared_state().await;
            self.update_tray();

            // Calculate backoff delay - but check for cancellation during the wait
            let delay = std::cmp::min(
                RECONNECT_BASE_DELAY_SECS * (attempt as u64),
                RECONNECT_MAX_DELAY_SECS,
            );

            // Wait with periodic checks for user commands
            let check_interval = Duration::from_millis(500);
            let total_delay = Duration::from_secs(delay);
            let start = Instant::now();

            while start.elapsed() < total_delay {
                // Check for pending commands (especially Disconnect)
                match self.rx.try_recv() {
                    Ok(VpnCommand::Disconnect) => {
                        info!("Disconnect command received during reconnect - cancelling");
                        // Disconnect any partial connection
                        let _ = nm_disconnect(connection_name).await;
                        self.last_disconnect_time = Some(Instant::now());
                        self.machine
                            .set_state(VpnState::Disconnected, TransitionReason::UserRequested);
                        self.sync_shared_state().await;
                        self.update_tray();
                        self.show_notification("VPN Disconnected", "Reconnection cancelled");
                        return;
                    }
                    Ok(other_cmd) => {
                        // Queue other commands to be processed later? For now, log and ignore
                        debug!("Ignoring command during reconnect: {:?}", other_cmd);
                    }
                    Err(_) => {
                        // No pending command, continue waiting
                    }
                }

                // Sleep for check interval or remaining time, whichever is shorter
                let remaining = total_delay.saturating_sub(start.elapsed());
                sleep(std::cmp::min(check_interval, remaining)).await;
            }

            // Attempt connection
            match nm_connect(connection_name).await {
                Ok(_) => {
                    // Check for disconnect command during verify delay
                    let verify_start = Instant::now();
                    let verify_delay = Duration::from_secs(CONNECTION_VERIFY_DELAY_SECS);
                    while verify_start.elapsed() < verify_delay {
                        if let Ok(VpnCommand::Disconnect) = self.rx.try_recv() {
                            info!(
                                "Disconnect command received during connection verify - cancelling"
                            );
                            let _ = nm_disconnect(connection_name).await;
                            self.last_disconnect_time = Some(Instant::now());
                            self.machine
                                .set_state(VpnState::Disconnected, TransitionReason::UserRequested);
                            self.sync_shared_state().await;
                            self.update_tray();
                            self.show_notification("VPN Disconnected", "Connection cancelled");
                            return;
                        }
                        sleep(Duration::from_millis(200)).await;
                    }

                    if let Some(active) = nm_get_active_vpn().await {
                        if active == connection_name {
                            info!("Successfully reconnected to {}", connection_name);
                            self.dispatch(Event::NmVpnUp {
                                server: connection_name.to_string(),
                            });
                            self.sync_shared_state().await;
                            self.update_tray();
                            self.show_notification(
                                "VPN Reconnected",
                                &format!("Reconnected to {}", connection_name),
                            );
                            reconnect_succeeded = true;
                            break;
                        }
                    }
                    warn!("Reconnection verification failed");
                }
                Err(e) => {
                    error!("Reconnection attempt {} failed: {}", attempt, e);
                }
            }
        }

        // NOTE: Kill switch stays enabled - no need to re-enable

        if reconnect_succeeded {
            return;
        }

        // All attempts exhausted
        error!("Max reconnection attempts reached for {}", connection_name);
        self.machine.set_state(
            VpnState::Failed {
                server: connection_name.to_string(),
                reason: format!("Max attempts ({}) exceeded", max_attempts),
            },
            TransitionReason::RetriesExhausted,
        );
        self.sync_shared_state().await;
        self.update_tray();
        self.show_notification(
            "VPN Reconnection Failed",
            &format!("Failed after {} attempts", max_attempts),
        );
    }
}
