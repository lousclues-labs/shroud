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

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};

use crate::config::{Config, ConfigManager};
use crate::dbus::NmEvent;
use crate::health::HealthChecker;
use crate::ipc::{IpcCommand, IpcResponse};
use crate::killswitch::KillSwitch;
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
    /// Tray handle for updating the icon
    pub(crate) tray_handle: Arc<std::sync::Mutex<Option<ksni::blocking::Handle<VpnTray>>>>,
    /// Timestamp of last intentional disconnect (for grace period)
    pub(crate) last_disconnect_time: Option<Instant>,
    /// Timestamp of last polling tick (for detecting sleep/wake)
    pub(crate) last_poll_time: Instant,
    /// Health checker for VPN connectivity verification
    pub(crate) health_checker: HealthChecker,
    /// Configuration manager for persistent settings
    pub(crate) config_manager: ConfigManager,
    /// Current configuration
    pub(crate) app_config: Config,
    /// Kill switch for blocking non-VPN traffic
    pub(crate) kill_switch: KillSwitch,
    /// VPN switching context
    pub(crate) switch_ctx: SwitchContext,
    /// Timestamp of last wake event dispatch (for debounce)
    pub(crate) last_wake_event: Option<Instant>,
    /// Timestamp of last reconnect attempt start (for debounce)
    pub(crate) last_reconnect_time: Option<Instant>,
    /// Flag to cancel ongoing reconnection attempts
    pub(crate) reconnect_cancelled: bool,
    /// Whether this is the first run (config file did not exist)
    pub(crate) is_first_run: bool,
    /// Exit state
    pub(crate) exit_state: ExitState,
    /// Notification manager for categorized, throttled desktop notifications
    pub(crate) notification_manager: NotificationManager,
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
        use log::info;

        // Load persistent configuration
        let config_manager = ConfigManager::new();
        let is_first_run = !config_manager.config_path().exists();
        let app_config = config_manager.load_validated();
        info!(
            "Loaded config: auto_reconnect={}, last_server={:?}",
            app_config.auto_reconnect, app_config.last_server
        );

        let sm_config = StateMachineConfig {
            max_retries: app_config.max_reconnect_attempts,
            base_delay_secs: RECONNECT_BASE_DELAY_SECS,
            max_delay_secs: RECONNECT_MAX_DELAY_SECS,
        };

        // Create kill switch with config-based DNS and IPv6 modes
        let mut kill_switch = KillSwitch::with_config(
            app_config.dns_mode,
            app_config.ipv6_mode,
            app_config.block_doh,
            app_config.custom_doh_blocklist.clone(),
        );

        // Sync with actual system state (detect existing rules)
        kill_switch.sync_state();
        if kill_switch.is_enabled() {
            info!("Kill switch rules detected from previous session");
        }

        let notification_manager = NotificationManager::new(app_config.notifications.clone());

        Self {
            machine: StateMachine::with_config(sm_config),
            shared_state,
            rx,
            ipc_rx,
            dbus_rx,
            tray_handle,
            last_disconnect_time: None,
            last_poll_time: Instant::now(),
            health_checker: HealthChecker::new(),
            config_manager,
            app_config,
            kill_switch,
            switch_ctx: SwitchContext::default(),
            last_wake_event: None,
            last_reconnect_time: None,
            reconnect_cancelled: false,
            is_first_run,
            exit_state: ExitState::default(),
            notification_manager,
        }
    }
}
