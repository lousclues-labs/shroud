// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! Supervisor reconnection logic

use std::time::Instant;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, instrument, warn};

use crate::state::{Event, TransitionReason, VpnState};
use crate::tray::VpnCommand;

use super::{CONNECTION_VERIFY_DELAY_SECS, RECONNECT_BASE_DELAY_SECS, RECONNECT_MAX_DELAY_SECS};
use crate::util::backoff::linear_backoff_secs;

/// Debounce period between reconnect attempts (seconds)
/// Prevents rapid reconnect thrashing
const RECONNECT_DEBOUNCE_SECS: u64 = 5;

impl super::VpnSupervisor {
    /// Check actual NetworkManager state before reconnecting
    ///
    /// Returns:
    /// - Some(true) if we should proceed with reconnect (no VPN active)
    /// - Some(false) if reconnect is unnecessary (target VPN already active)
    /// - None if a different VPN is active (user switched manually)
    #[instrument(skip(self), fields(target = %target_server))]
    async fn should_attempt_reconnect(&mut self, target_server: &str) -> Option<bool> {
        // Query NetworkManager for actual state
        match self.nm.get_active_vpn().await {
            Some(active) if active == target_server => {
                // Already connected to the target VPN!
                info!(
                    "VPN '{}' is already active, cancelling reconnect",
                    target_server
                );
                // Sync our state to reality
                self.dispatch(Event::NmVpnUp {
                    server: active.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                Some(false)
            }
            Some(active) => {
                // Connected to DIFFERENT VPN - user switched manually
                info!(
                    "Different VPN active ('{}'), user may have switched manually from '{}'",
                    active, target_server
                );
                // Update our state to reflect reality
                self.dispatch(Event::NmVpnUp {
                    server: active.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                self.tray
                    .notify("VPN Switched", &format!("Now connected to {}", active));
                None
            }
            None => {
                // No VPN connected, proceed with reconnect
                Some(true)
            }
        }
    }

    /// Attempt to reconnect with linear backoff (triggered by connection drop)
    #[instrument(skip(self), fields(connection = %connection_name))]
    pub(crate) async fn attempt_reconnect(&mut self, connection_name: &str) {
        // RACE PREVENTION: Check if reconnect is already in progress.
        // Uses struct field instead of static AtomicBool so it resets when
        // the supervisor is dropped and recreated (e.g., during restart).
        if self.timing.reconnect_in_progress {
            debug!("Reconnect already in progress, ignoring duplicate request");
            return;
        }
        self.timing.reconnect_in_progress = true;
        self.attempt_reconnect_inner(connection_name).await;
        self.timing.reconnect_in_progress = false;

        // Drain any commands that arrived during the reconnect loop
        while let Some(cmd) = self.deferred_commands.pop_front() {
            info!("Processing deferred command after reconnect: {:?}", cmd);
            match cmd {
                VpnCommand::Connect(server) => self.handle_connect(&server).await,
                VpnCommand::Disconnect => self.handle_disconnect().await,
                VpnCommand::ToggleAutoReconnect => self.toggle_auto_reconnect().await,
                VpnCommand::ToggleKillSwitch => self.toggle_kill_switch().await,
                VpnCommand::ToggleAutostart => self.toggle_autostart().await,
                VpnCommand::ToggleDebugLogging => self.toggle_debug_logging().await,
                VpnCommand::OpenLogFile => self.open_log_file(),
                VpnCommand::RefreshConnections => self.refresh_connections().await,
                VpnCommand::Restart => self.handle_restart().await,
                VpnCommand::Quit => self.handle_quit().await,
            }
        }
    }

    async fn attempt_reconnect_inner(&mut self, connection_name: &str) {
        // DEBOUNCE: Check if we recently attempted a reconnect
        if let Some(last_time) = self.timing.last_reconnect_time {
            let elapsed = last_time.elapsed().as_secs();
            if elapsed < RECONNECT_DEBOUNCE_SECS {
                debug!(
                    "Reconnect debounce active ({}/{}s), skipping",
                    elapsed, RECONNECT_DEBOUNCE_SECS
                );
                return;
            }
        }
        self.timing.last_reconnect_time = Some(Instant::now());

        // Clear any previous cancellation flag
        self.timing.reconnect_cancelled = false;

        // CRITICAL: Check actual NM state before starting reconnect loop
        match self.should_attempt_reconnect(connection_name).await {
            Some(true) => {
                // No VPN active, proceed with reconnect
            }
            Some(false) => {
                // Target VPN is already active, we're done
                return;
            }
            None => {
                // Different VPN active, user switched manually - don't interfere
                return;
            }
        }

        // Verify the connection still exists in NetworkManager
        let available_connections = self.nm.list_vpn_connections().await;
        if !available_connections.iter().any(|c| c == connection_name) {
            error!(
                "Cannot reconnect: VPN '{}' no longer exists in NetworkManager",
                connection_name
            );
            self.tray.notify(
                "Reconnect Failed",
                &format!("VPN '{}' not found", connection_name),
            );
            // Use ConnectionFailed to go directly to Disconnected
            self.dispatch(Event::ConnectionFailed {
                reason: format!("VPN '{}' no longer exists", connection_name),
            });
            self.sync_shared_state().await;
            self.tray.update(&self.shared_state);
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
            if self.timing.reconnect_cancelled {
                info!("Reconnection cancelled by user");
                self.machine
                    .set_state(VpnState::Disconnected, TransitionReason::UserRequested);
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                return;
            }

            // CRITICAL: Re-check NM state before each attempt
            // User might have connected a VPN externally during our backoff delay
            match self.should_attempt_reconnect(connection_name).await {
                Some(true) => {
                    // Still no VPN, proceed
                }
                Some(false) | None => {
                    // VPN now active (target or different), stop reconnecting
                    info!("Reconnect cancelled - VPN state resolved externally");
                    reconnect_succeeded = true; // Consider it success - connection exists
                    break;
                }
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
            self.tray.update(&self.shared_state);

            // Calculate backoff delay - but check for cancellation during the wait
            let delay =
                linear_backoff_secs(RECONNECT_BASE_DELAY_SECS, RECONNECT_MAX_DELAY_SECS, attempt)
                    .as_secs();

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
                        let _ = self.nm.disconnect(connection_name).await;
                        self.timing.last_disconnect_time = Some(Instant::now());
                        self.machine
                            .set_state(VpnState::Disconnected, TransitionReason::UserRequested);
                        self.sync_shared_state().await;
                        self.tray.update(&self.shared_state);
                        self.tray
                            .notify("VPN Disconnected", "Reconnection cancelled");
                        return;
                    }
                    Ok(other_cmd) => {
                        // Queue commands to be processed after the reconnect loop
                        debug!("Deferring command during reconnect: {:?}", other_cmd);
                        self.deferred_commands.push_back(other_cmd);
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
            match self.nm.connect(connection_name).await {
                Ok(_) => {
                    // Check for disconnect command during verify delay
                    let verify_start = Instant::now();
                    let verify_delay = Duration::from_secs(CONNECTION_VERIFY_DELAY_SECS);
                    while verify_start.elapsed() < verify_delay {
                        if let Ok(VpnCommand::Disconnect) = self.rx.try_recv() {
                            info!(
                                "Disconnect command received during connection verify - cancelling"
                            );
                            let _ = self.nm.disconnect(connection_name).await;
                            self.timing.last_disconnect_time = Some(Instant::now());
                            self.machine
                                .set_state(VpnState::Disconnected, TransitionReason::UserRequested);
                            self.sync_shared_state().await;
                            self.tray.update(&self.shared_state);
                            self.tray.notify("VPN Disconnected", "Connection cancelled");
                            return;
                        }
                        sleep(Duration::from_millis(200)).await;
                    }

                    if let Some(active) = self.nm.get_active_vpn().await {
                        if active == connection_name {
                            info!("Successfully reconnected to {}", connection_name);
                            self.dispatch(Event::NmVpnUp {
                                server: connection_name.to_string(),
                            });
                            self.sync_shared_state().await;
                            self.tray.update(&self.shared_state);
                            self.tray.notify(
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
        self.tray.update(&self.shared_state);
        self.tray.notify(
            "VPN Reconnection Failed",
            &format!("Failed after {} attempts", max_attempts),
        );
    }
}
