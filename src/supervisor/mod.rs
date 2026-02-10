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
//! - `reconnect.rs` - Reconnection logic with exponential backoff

#[allow(dead_code)]
pub mod command_validation;
mod config_store;
#[allow(dead_code)]
pub mod connection_stats;
mod event_loop;
mod handlers;
mod reconnect;
#[allow(dead_code)]
pub mod reconnect_logic;
#[allow(dead_code)]
pub mod response_builder;
mod state_sync;
mod tray_bridge;

pub(crate) use config_store::ConfigStore;
pub(crate) use tray_bridge::TrayBridge;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};

use crate::dbus::NmEvent;
use crate::health::HealthChecker;
use crate::ipc::{IpcCommand, IpcResponse};
use crate::killswitch::KillSwitch;
use crate::nm::{NmCliClient, NmClient};
use crate::notifications::NotificationManager;
use crate::state::{StateMachine, StateMachineConfig};
use crate::tray::{SharedState, VpnCommand, VpnTray};

// Re-export constants that may be needed elsewhere

/// Base delay for exponential backoff in seconds
pub(crate) const RECONNECT_BASE_DELAY_SECS: u64 = 2;

/// Cap on reconnect delay in seconds
pub(crate) const RECONNECT_MAX_DELAY_SECS: u64 = 30;

/// Grace period after intentional disconnect to prevent false drop detection
pub(crate) const POST_DISCONNECT_GRACE_SECS: u64 = 5;

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

#[allow(dead_code)]
impl SwitchContext {
    pub(crate) fn start(&mut self, from: &str, to: &str) {
        self.in_progress = true;
        self.from = Some(from.to_string());
        self.target = Some(to.to_string());
        self.completed_time = None;
    }

    pub(crate) fn complete(&mut self) {
        self.in_progress = false;
        self.completed_time = Some(Instant::now());
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
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
    pub(crate) last_wake_event: Option<Instant>,
    pub(crate) last_reconnect_time: Option<Instant>,
    pub(crate) reconnect_cancelled: bool,
}

impl Default for TimingState {
    fn default() -> Self {
        Self {
            last_disconnect_time: None,
            last_poll_time: Instant::now(),
            last_wake_event: None,
            last_reconnect_time: None,
            reconnect_cancelled: false,
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
        use log::info;

        // Load persistent configuration
        let config_store = ConfigStore::load();

        let sm_config = StateMachineConfig {
            max_retries: config_store.config.max_reconnect_attempts,
            base_delay_secs: RECONNECT_BASE_DELAY_SECS,
            max_delay_secs: RECONNECT_MAX_DELAY_SECS,
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

        Self {
            machine: StateMachine::with_config(sm_config),
            shared_state,
            rx,
            ipc_rx,
            dbus_rx,
            tray,
            config_store,
            nm,
            health_checker: HealthChecker::new(),
            kill_switch,
            timing: TimingState::default(),
            switch_ctx: SwitchContext::default(),
            exit_state: ExitState::default(),
        }
    }
}
