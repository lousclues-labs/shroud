// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! VPN Supervisor module
//!
//! The VpnSupervisor is the core orchestrator of the Shroud VPN manager.
//! It coordinates:
//! - VPN connection state management via a formal state machine
//! - NetworkManager interaction (via nmcli and D-Bus events)
//! - Kill switch management (iptables firewall rules)
//! - Health monitoring of VPN connections
//! - System tray updates
//! - CLI command handling
//!
//! ## Module Structure
//!
//! - `mod.rs` - VpnSupervisor struct definition and constructor
//! - `event_loop.rs` - Main tokio::select! event loop (run method)
//! - `handlers.rs` - Command and event handlers
//! - `state_sync.rs` - State synchronization utilities
//! - `reconnect.rs` - Reconnection logic with linear backoff

mod config_store;
mod event_loop;
mod handlers;
mod reconnect;
mod state_sync;
mod tray_bridge;

pub(crate) use config_store::ConfigStore;
pub(crate) use tray_bridge::TrayBridge;

#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use tokio::sync::{mpsc, RwLock};

use crate::dbus::NmEvent;
use crate::health::checker::{HealthConfig, ProbeOutcome};
use crate::health::HealthChecker;
use crate::ipc::{IpcCommand, IpcResponse};
use crate::killswitch::KillSwitch;
use crate::nm::{NmCliClient, NmClient};
use crate::notifications::NotificationManager;
use crate::state::{StateMachine, StateMachineConfig};
use crate::tray::{SharedState, VpnCommand, VpnTray};

// Re-export constants that may be needed elsewhere

/// Base delay for linear backoff in seconds
pub(crate) const RECONNECT_BASE_DELAY_SECS: u64 = 2;

/// Cap on reconnect delay in seconds
pub(crate) const RECONNECT_MAX_DELAY_SECS: u64 = 30;

/// Grace period after intentional disconnect to prevent false drop detection
pub(crate) const POST_DISCONNECT_GRACE_SECS: u64 = 5;

/// Window after a wake resync during which replayed D-Bus events are ignored.
///
/// NetworkManager delivers the transitions buffered during a suspend over the
/// following tens of milliseconds; they predate the resync and would otherwise
/// flap the state machine against a tunnel NM has already confirmed healthy.
pub(crate) const WAKE_RESYNC_GRACE_SECS: u64 = 2;

/// Maximum attempts to verify disconnect completion
pub(crate) const DISCONNECT_VERIFY_MAX_ATTEMPTS: u32 = 30;

/// Maximum attempts to verify connection after nmcli con up
pub(crate) const CONNECTION_MONITOR_MAX_ATTEMPTS: u32 = 60;

/// Interval between connection monitoring attempts in milliseconds
pub(crate) const CONNECTION_MONITOR_INTERVAL_MS: u64 = 500;

/// Interval between disconnect verification attempts in milliseconds
pub(crate) const DISCONNECT_VERIFY_INTERVAL_MS: u64 = 500;

/// Settle time after disconnect is verified before connecting to new VPN
pub(crate) const POST_DISCONNECT_SETTLE_SECS: u64 = 3;

/// Maximum number of connection attempts during handle_connect
pub(crate) const MAX_CONNECT_ATTEMPTS: u32 = 3;

/// Delay between failed connection attempts in `handle_connect`
pub(crate) const CONNECT_RETRY_DELAY_SECS: u64 = 2;

/// Worst-case wall-clock duration of a full VPN switch: disconnect verification,
/// post-disconnect settle, then every connect attempt with its monitoring window.
///
/// Callers that wait on the supervisor (notably the IPC layer) must allow at
/// least this long, otherwise a legitimately slow switch is misreported as an
/// unresponsive supervisor.
pub(crate) const WORST_CASE_SWITCH_SECS: u64 =
    (DISCONNECT_VERIFY_MAX_ATTEMPTS as u64 * DISCONNECT_VERIFY_INTERVAL_MS / 1000)
        + POST_DISCONNECT_SETTLE_SECS
        + (MAX_CONNECT_ATTEMPTS as u64
            * CONNECTION_MONITOR_MAX_ATTEMPTS as u64
            * CONNECTION_MONITOR_INTERVAL_MS
            / 1000)
        + ((MAX_CONNECT_ATTEMPTS as u64 - 1) * CONNECT_RETRY_DELAY_SECS);

// The CLI's slow-command default must outlast the supervisor, or a healthy
// switch is reported as a client-side timeout.
const _: () = assert!(
    crate::cli::validation::SLOW_COMMAND_TIMEOUT_SECS >= WORST_CASE_SWITCH_SECS,
    "SLOW_COMMAND_TIMEOUT_SECS must cover WORST_CASE_SWITCH_SECS"
);

/// Wait after nmcli con up before verifying connection
pub(crate) const CONNECTION_VERIFY_DELAY_SECS: u64 = 5;

/// Tracks the state of an in-progress VPN switch operation.
#[derive(Debug, Default)]
pub(crate) struct SwitchContext {
    pub(crate) in_progress: bool,
    pub(crate) target: Option<String>,
    pub(crate) from: Option<String>,
    pub(crate) completed_time: Option<Instant>,
}

/// Tracks whether the supervisor should exit and why.
#[derive(Debug, Default)]
pub(crate) struct ExitState {
    pub(crate) should_exit: bool,
    pub(crate) reason: Option<String>,
}

impl ExitState {
    pub(crate) fn request(&mut self, reason: &str) {
        self.should_exit = true;
        self.reason = Some(reason.to_string());
    }
}

/// Timing-sensitive state for debouncing, grace periods, and throttling.
#[derive(Debug)]
pub(crate) struct TimingState {
    pub(crate) last_disconnect_time: Option<Instant>,
    pub(crate) last_poll_time: Instant,
    /// Wall-clock companion to `last_poll_time`.
    ///
    /// `Instant` is `CLOCK_MONOTONIC`, which does not advance while the machine
    /// is suspended; `SystemTime` does. Comparing the two deltas is what lets
    /// the poll loop tell a genuine suspend/resume apart from the loop simply
    /// having been busy — see `event_loop::detect_time_jump`.
    pub(crate) last_poll_wall: SystemTime,
    pub(crate) last_wake_event: Option<Instant>,
    /// When the last wake resync established ground truth from NetworkManager.
    pub(crate) last_wake_resync: Option<Instant>,
    pub(crate) last_reconnect_time: Option<Instant>,
    pub(crate) reconnect_cancelled: bool,
    /// Guard flag: true while a reconnect loop is running. Struct-owned, not
    /// static, so it resets when the supervisor is dropped (e.g., restart).
    ///
    /// # Safety invariant
    ///
    /// This is a plain `bool`, not an `AtomicBool`. It is safe **only** because
    /// `VpnSupervisor` holds `&mut self` in a single-task tokio event loop —
    /// no concurrent access is possible. If the supervisor is ever shared
    /// across tasks, this must be changed to an `AtomicBool`.
    pub(crate) reconnect_in_progress: bool,
}

impl Default for TimingState {
    fn default() -> Self {
        Self {
            last_disconnect_time: None,
            last_poll_time: Instant::now(),
            last_poll_wall: SystemTime::now(),
            last_wake_event: None,
            last_wake_resync: None,
            last_reconnect_time: None,
            reconnect_cancelled: false,
            reconnect_in_progress: false,
        }
    }
}

/// VPN Supervisor that manages VPN connections via NetworkManager
///
/// Uses a formal state machine for all state transitions, ensuring:
/// - Every transition is logged with reason
/// - Predictable behavior based on current state + event
/// - Clean separation between state logic and I/O
pub struct VpnSupervisor {
    /// The formal state machine (owns the canonical VPN state)
    pub(crate) machine: StateMachine,
    /// Shared state for the tray (view of the machine state + UI state)
    pub(crate) shared_state: Arc<RwLock<SharedState>>,
    /// Channel receiver for commands from the tray
    pub(crate) rx: mpsc::Receiver<VpnCommand>,
    /// Channel receiver for IPC commands from CLI
    pub(crate) ipc_rx: mpsc::Receiver<(IpcCommand, mpsc::Sender<IpcResponse>)>,
    /// Channel receiver for D-Bus events from NetworkManager
    pub(crate) dbus_rx: mpsc::Receiver<NmEvent>,
    /// Health checker for VPN connectivity verification
    pub(crate) health_checker: HealthChecker,
    /// Sender handed to detached health probes so they can report back.
    pub(crate) health_tx: mpsc::Sender<ProbeOutcome>,
    /// Receiver for completed health probes, polled by the event loop.
    pub(crate) health_rx: mpsc::Receiver<ProbeOutcome>,
    /// Guard preventing overlapping probes when one runs longer than the
    /// health check interval.
    pub(crate) health_probe_in_flight: bool,
    /// System tray and notifications
    pub(crate) tray: TrayBridge,
    /// Persistent configuration storage
    pub(crate) config_store: ConfigStore,
    /// NetworkManager client (trait object for testability)
    pub(crate) nm: Box<dyn NmClient>,
    /// Kill switch for blocking non-VPN traffic
    pub(crate) kill_switch: KillSwitch,
    /// Timing-sensitive state
    pub(crate) timing: TimingState,
    /// VPN switching context
    pub(crate) switch_ctx: SwitchContext,
    /// Exit state
    pub(crate) exit_state: ExitState,
    /// Commands deferred during reconnect (drained after reconnect completes)
    pub(crate) deferred_commands: VecDeque<VpnCommand>,
}

impl VpnSupervisor {
    /// Create a new VPN supervisor with formal state machine
    pub fn new(
        shared_state: Arc<RwLock<SharedState>>,
        rx: mpsc::Receiver<VpnCommand>,
        ipc_rx: mpsc::Receiver<(IpcCommand, mpsc::Sender<IpcResponse>)>,
        dbus_rx: mpsc::Receiver<NmEvent>,
        tray_handle: Arc<std::sync::Mutex<Option<ksni::blocking::Handle<VpnTray>>>>,
    ) -> Self {
        Self::with_nm(
            shared_state,
            rx,
            ipc_rx,
            dbus_rx,
            tray_handle,
            Box::new(NmCliClient),
        )
    }

    /// Constructor that accepts an NM client (for testing)
    pub(crate) fn with_nm(
        shared_state: Arc<RwLock<SharedState>>,
        rx: mpsc::Receiver<VpnCommand>,
        ipc_rx: mpsc::Receiver<(IpcCommand, mpsc::Sender<IpcResponse>)>,
        dbus_rx: mpsc::Receiver<NmEvent>,
        tray_handle: Arc<std::sync::Mutex<Option<ksni::blocking::Handle<VpnTray>>>>,
        nm: Box<dyn NmClient>,
    ) -> Self {
        use tracing::info;

        // Load persistent configuration
        let config_store = ConfigStore::load();

        let sm_config = StateMachineConfig {
            max_retries: config_store.config.max_reconnect_attempts,
        };

        // Create kill switch with config-based DNS and IPv6 modes
        let mut kill_switch = KillSwitch::with_config(
            config_store.config.dns_mode,
            config_store.config.ipv6_mode,
            config_store.config.block_doh,
            config_store.config.custom_doh_blocklist.clone(),
        );

        // Sync with actual system state (detect existing rules)
        kill_switch.sync_state();
        if kill_switch.is_enabled() {
            info!("Kill switch rules detected from previous session");
        }

        let notification_manager =
            NotificationManager::new(config_store.config.notifications.clone());
        let tray = TrayBridge::new(tray_handle, notification_manager);

        // DNS leak check defaults to true when dns_mode is tunnel/strict
        let dns_leak_check = config_store.config.dns_leak_check.unwrap_or(matches!(
            config_store.config.dns_mode,
            crate::config::DnsMode::Tunnel | crate::config::DnsMode::Strict
        ));

        let health_config = {
            let mut health_config = HealthConfig {
                degraded_threshold_ms: config_store.config.health_degraded_threshold_ms,
                expected_exit_ip: config_store.config.expected_exit_ip.clone(),
                dns_leak_check,
                ..Default::default()
            };

            if !config_store.config.health_check_endpoints.is_empty() {
                health_config.endpoints = config_store.config.health_check_endpoints.clone();
            }

            // When `block_doh` is on, the kill switch drops tcp/443 to every
            // DoH provider. Probing one of those addresses would stall until
            // the connect timeout expires and then report a false failure, so
            // drop them rather than let the health checker fight our own
            // firewall rules.
            if config_store.config.block_doh {
                let conflicts = crate::health::checker::doh_conflicts(
                    &health_config.endpoints,
                    &config_store.config.custom_doh_blocklist,
                );

                if !conflicts.is_empty() {
                    tracing::warn!(
                        "Ignoring health endpoint(s) blocked by our own DoH rules: {}",
                        conflicts.join(", ")
                    );
                    health_config.endpoints.retain(|e| !conflicts.contains(e));

                    if health_config.endpoints.is_empty() {
                        health_config.endpoints = HealthConfig::default().endpoints;
                        tracing::warn!(
                            "All configured health endpoints were DoH-blocked; \
                             falling back to defaults"
                        );
                    }
                }
            }

            health_config
        };

        // Depth 1: `health_probe_in_flight` already guarantees at most one
        // outstanding probe, so a deeper queue could only hold stale results.
        let (health_tx, health_rx) = mpsc::channel(1);

        Self {
            machine: StateMachine::with_config(sm_config),
            shared_state,
            rx,
            ipc_rx,
            dbus_rx,
            tray,
            config_store,
            nm,
            health_checker: HealthChecker::with_config(health_config),
            health_tx,
            health_rx,
            health_probe_in_flight: false,
            kill_switch,
            timing: TimingState::default(),
            switch_ctx: SwitchContext::default(),
            exit_state: ExitState::default(),
            deferred_commands: VecDeque::new(),
        }
    }
}
