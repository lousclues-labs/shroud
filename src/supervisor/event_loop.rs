// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Supervisor event loop

use std::time::{Instant, SystemTime};
use tokio::time::{Duration, MissedTickBehavior};
use tracing::{debug, info, instrument, warn};

use crate::state::{Event, VpnState};
use crate::tray::VpnCommand;

/// Poll NetworkManager state every 30 seconds.
///
/// This is a *backstop*, not the primary signal: `dbus::monitor` delivers NM
/// state changes in real time, and the poll only exists to recover from a
/// missed or dropped signal. It previously ran every 2 seconds, which issued
/// ~43,000 `nmcli` subprocesses a day — the daemon spent roughly 29x more CPU
/// spawning processes than doing its own work — to re-learn something D-Bus had
/// already told it.
pub const NM_POLL_INTERVAL_SECS: u64 = 30;

/// Health check interval when connected (seconds)
pub const HEALTH_CHECK_INTERVAL_SECS: u64 = 30;

/// Wall-clock/monotonic skew above which we treat the gap as a real time jump
/// (suspend/resume, or an NTP step) rather than a busy event loop.
///
/// Deliberately *not* derived from the poll interval. The old
/// `NM_POLL_INTERVAL_SECS * 3` rule measured only monotonic elapsed time, so
/// any slow operation inside the loop looked identical to a suspend: a health
/// check stalling 5s on a blocked endpoint produced a permanent stream of
/// "Time jump detected (7.1s)" warnings and a full NM resync every 60s.
pub const TIME_JUMP_THRESHOLD_SECS: u64 = 10;

/// Cooldown period after a time jump event (prevents thrashing)
/// Only one wake event per cooldown window
pub const TIME_JUMP_COOLDOWN_SECS: u64 = 5;

// D-Bus is the primary signal for NM state; the poll is only a backstop.
// Dropping it back to a few seconds reintroduces the subprocess storm that
// cost ~29x the daemon's own CPU. Enforced at compile time so it cannot be
// tuned back down by accident.
const _: () = assert!(
    NM_POLL_INTERVAL_SECS >= 30,
    "NM_POLL_INTERVAL_SECS is a backstop; frequent polling reintroduces the nmcli subprocess storm"
);

// Detection must stay decoupled from the poll interval. The original
// `NM_POLL_INTERVAL_SECS * 3` rule is what made a slow event loop
// indistinguishable from a suspend.
const _: () = assert!(
    TIME_JUMP_THRESHOLD_SECS != NM_POLL_INTERVAL_SECS * 3,
    "TIME_JUMP_THRESHOLD_SECS must not be derived from the poll interval"
);

/// Decide whether the gap between two polls represents a genuine time jump.
///
/// `mono_delta` comes from `Instant` (`CLOCK_MONOTONIC`, which pauses while the
/// machine is suspended) and `wall_delta` from `SystemTime` (which does not).
/// A suspend therefore shows up as wall time racing ahead of monotonic time,
/// whereas a merely slow event loop advances both equally and yields ~zero
/// skew. Returns the detected skew when it exceeds `threshold`.
pub fn detect_time_jump(
    wall_delta: Duration,
    mono_delta: Duration,
    threshold: Duration,
) -> Option<Duration> {
    let skew = wall_delta.saturating_sub(mono_delta);
    (skew > threshold).then_some(skew)
}

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
                        .notify("VPN Shroud", &format!("Auto-connecting to {}...", server));
                    self.handle_connect(&server).await;
                }
                None => {
                    warn!("auto_connect enabled but no VPN connections found in NetworkManager");
                    self.tray.notify(
                        "VPN Shroud",
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
                "VPN Shroud",
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

        // Create an interval for NM polling.
        // `Delay` (not the default `Burst`) so a slow cycle does not cause the
        // missed ticks to fire back-to-back immediately afterwards.
        let mut nm_poll_interval =
            tokio::time::interval(Duration::from_secs(NM_POLL_INTERVAL_SECS));
        nm_poll_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        // Create an interval for health checks (only runs when connected)
        let mut health_check_interval = tokio::time::interval(Duration::from_secs(health_interval));
        health_check_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

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
                    let mono_delta = self.timing.last_poll_time.elapsed();
                    let wall_delta = SystemTime::now()
                        .duration_since(self.timing.last_poll_wall)
                        .unwrap_or_default();

                    let jump = detect_time_jump(
                        wall_delta,
                        mono_delta,
                        Duration::from_secs(TIME_JUMP_THRESHOLD_SECS),
                    );

                    if let Some(skew) = jump {
                        // Time jump detected - check if we're in cooldown period
                        let should_dispatch = match self.timing.last_wake_event {
                            Some(last) => last.elapsed().as_secs() >= TIME_JUMP_COOLDOWN_SECS,
                            None => true,
                        };

                        if should_dispatch {
                            warn!(
                                "Time jump detected ({:.1}s of wall-clock skew), dispatching Wake event",
                                skew.as_secs_f32()
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
                    self.timing.last_poll_wall = SystemTime::now();
                }

                // Launch a health check when connected (disabled when
                // health_check_interval_secs = 0). The network probe runs on a
                // detached task; only its result is handled on this loop.
                _ = health_check_interval.tick(), if health_checks_enabled => {
                    if self.health_probe_in_flight {
                        debug!("Skipping health check - previous probe still in flight");
                    } else if let Some(config) = self.begin_health_check().await {
                        self.health_probe_in_flight = true;
                        let tx = self.health_tx.clone();
                        tokio::spawn(async move {
                            let outcome = crate::health::checker::run_probe(&config).await;
                            let _ = tx.send(outcome).await;
                        });
                    }
                }

                // Apply a completed health probe
                Some(outcome) = self.health_rx.recv() => {
                    self.health_probe_in_flight = false;
                    self.finish_health_check(outcome).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod time_jump_tests {
    use super::*;

    const THRESHOLD: Duration = Duration::from_secs(TIME_JUMP_THRESHOLD_SECS);

    /// The exact defect this replaced: a health check stalling ~5s on an
    /// unreachable endpoint advanced monotonic and wall clocks equally, but the
    /// old monotonic-only rule read it as a suspend and forced a full NM resync
    /// roughly once a minute, forever.
    #[test]
    fn test_blocked_event_loop_is_not_a_time_jump() {
        // 7.1s of real elapsed time, no clock skew.
        let elapsed = Duration::from_millis(7100);
        assert_eq!(detect_time_jump(elapsed, elapsed, THRESHOLD), None);
    }

    #[test]
    fn test_slow_loop_far_beyond_threshold_is_still_not_a_jump() {
        let elapsed = Duration::from_secs(120);
        assert_eq!(detect_time_jump(elapsed, elapsed, THRESHOLD), None);
    }

    #[test]
    fn test_suspend_is_detected() {
        // Machine suspended an hour: wall clock advanced, CLOCK_MONOTONIC did not.
        let wall = Duration::from_secs(3600);
        let mono = Duration::from_secs(2);
        let skew = detect_time_jump(wall, mono, THRESHOLD).expect("suspend must be detected");
        assert_eq!(skew, Duration::from_secs(3598));
    }

    #[test]
    fn test_skew_at_threshold_is_not_a_jump() {
        let mono = Duration::from_secs(1);
        let wall = mono + THRESHOLD;
        assert_eq!(detect_time_jump(wall, mono, THRESHOLD), None);
    }

    #[test]
    fn test_skew_just_past_threshold_is_a_jump() {
        let mono = Duration::from_secs(1);
        let wall = mono + THRESHOLD + Duration::from_millis(1);
        assert!(detect_time_jump(wall, mono, THRESHOLD).is_some());
    }

    /// A backwards NTP step makes wall_delta smaller than mono_delta;
    /// `saturating_sub` must not underflow.
    #[test]
    fn test_backwards_clock_step_does_not_panic() {
        let wall = Duration::from_secs(1);
        let mono = Duration::from_secs(60);
        assert_eq!(detect_time_jump(wall, mono, THRESHOLD), None);
    }
}
