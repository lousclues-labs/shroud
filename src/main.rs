//! NetworkManager VPN Supervisor with System Tray
//!
//! A production-ready system tray application for managing VPN connections via NetworkManager
//! with auto-reconnect capabilities for Arch Linux / KDE Plasma.
//!
//! # Architecture
//!
//! - `state/` - State machine types and transitions (formal state machine)
//! - `nm/` - NetworkManager interface (nmcli, future: D-Bus)
//! - `tray/` - System tray UI (ksni)
//!
//! # State Machine
//!
//! The supervisor uses a formal state machine that processes events:
//! - User events: UserEnable, UserDisable
//! - NM events: NmVpnUp, NmVpnDown, NmVpnChanged
//! - System events: Wake (from sleep)
//! - Internal events: Timeout
//!
//! All state transitions go through StateMachine::handle_event() which logs
//! every transition with its reason.

mod config;
mod dbus;
mod health;
mod killswitch;
mod nm;
mod state;
mod tray;

use log::{debug, error, info, warn};
use notify_rust::Notification;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, Duration};

use crate::config::{Config, ConfigManager};
use crate::dbus::{NmEvent, NmMonitor};
use crate::health::{HealthChecker, HealthResult};
use crate::killswitch::KillSwitch;
use crate::nm::{
    connect as nm_connect, disconnect as nm_disconnect, get_active_vpn as nm_get_active_vpn,
    get_active_vpn_with_state as nm_get_active_vpn_with_state, get_all_active_vpns as nm_get_all_active_vpns,
    get_vpn_state as nm_get_vpn_state, kill_orphan_openvpn_processes, list_vpn_connections as nm_list_vpn_connections,
};
use crate::state::{Event, NmVpnState, StateMachine, StateMachineConfig, TransitionReason, VpnState};
use crate::tray::{SharedState, VpnCommand, VpnTray};

// ============================================================================
// Configuration Constants
// ============================================================================

/// Poll NetworkManager state every 2 seconds
const NM_POLL_INTERVAL_SECS: u64 = 2;

/// Health check interval when connected (seconds)
const HEALTH_CHECK_INTERVAL_SECS: u64 = 30;

/// Wait after nmcli con up before verifying connection
const CONNECTION_VERIFY_DELAY_SECS: u64 = 5;

/// Maximum number of reconnection attempts before giving up
const MAX_RECONNECT_ATTEMPTS: u32 = 10;

/// Maximum number of connection attempts during handle_connect
const MAX_CONNECT_ATTEMPTS: u32 = 3;

/// Base delay for exponential backoff in seconds
const RECONNECT_BASE_DELAY_SECS: u64 = 2;

/// Cap on reconnect delay in seconds
const RECONNECT_MAX_DELAY_SECS: u64 = 30;

/// Grace period after intentional disconnect to prevent false drop detection
const POST_DISCONNECT_GRACE_SECS: u64 = 5;

/// Maximum attempts to verify disconnect completion
const DISCONNECT_VERIFY_MAX_ATTEMPTS: u32 = 30;

/// Maximum attempts to verify connection after nmcli con up
const CONNECTION_MONITOR_MAX_ATTEMPTS: u32 = 60;

/// Interval between connection monitoring attempts in milliseconds
const CONNECTION_MONITOR_INTERVAL_MS: u64 = 500;

/// Interval between disconnect verification attempts in milliseconds
const DISCONNECT_VERIFY_INTERVAL_MS: u64 = 500;

/// Settle time after disconnect is verified before connecting to new VPN
const POST_DISCONNECT_SETTLE_SECS: u64 = 3;

// ============================================================================
// VPN Supervisor
// ============================================================================

/// VPN Supervisor that manages VPN connections via NetworkManager
/// 
/// Uses a formal state machine for all state transitions, ensuring:
/// - Every transition is logged with reason
/// - Predictable behavior based on current state + event
/// - Clean separation between state logic and I/O
pub struct VpnSupervisor {
    /// The formal state machine (owns the canonical VPN state)
    machine: StateMachine,
    /// Shared state for the tray (view of the machine state + UI state)
    shared_state: Arc<RwLock<SharedState>>,
    /// Channel receiver for commands from the tray
    rx: mpsc::Receiver<VpnCommand>,
    /// Channel receiver for D-Bus events from NetworkManager
    dbus_rx: mpsc::Receiver<NmEvent>,
    /// Tray handle for updating the icon
    tray_handle: Arc<std::sync::Mutex<Option<ksni::blocking::Handle<VpnTray>>>>,
    /// Timestamp of last intentional disconnect (for grace period)
    last_disconnect_time: Option<Instant>,
    /// Timestamp of last polling tick (for detecting sleep/wake)
    last_poll_time: Instant,
    /// Health checker for VPN connectivity verification
    health_checker: HealthChecker,
    /// Configuration manager for persistent settings
    config_manager: ConfigManager,
    /// Current configuration
    app_config: Config,
    /// Kill switch for blocking non-VPN traffic
    kill_switch: KillSwitch,
    /// Flag to indicate a VPN switch is in progress (prevents D-Bus event interference)
    switching_in_progress: bool,
    /// The target server we're switching TO (to ignore deactivation events for old VPN)
    switching_target: Option<String>,
    /// The server we're switching FROM (to ignore late deactivation events)
    switching_from: Option<String>,
    /// Timestamp when switch completed (to ignore late D-Bus events)
    switch_completed_time: Option<Instant>,
}

impl VpnSupervisor {
    /// Create a new VPN supervisor with formal state machine
    pub fn new(
        shared_state: Arc<RwLock<SharedState>>,
        rx: mpsc::Receiver<VpnCommand>,
        dbus_rx: mpsc::Receiver<NmEvent>,
        tray_handle: Arc<std::sync::Mutex<Option<ksni::blocking::Handle<VpnTray>>>>,
    ) -> Self {
        // Load persistent configuration
        let config_manager = ConfigManager::new();
        let app_config = config_manager.load();
        info!("Loaded config: auto_reconnect={}, last_server={:?}", 
              app_config.auto_reconnect, app_config.last_server);

        let sm_config = StateMachineConfig {
            max_retries: app_config.max_reconnect_attempts,
            base_delay_secs: RECONNECT_BASE_DELAY_SECS,
            max_delay_secs: RECONNECT_MAX_DELAY_SECS,
        };
        
        Self {
            machine: StateMachine::with_config(sm_config),
            shared_state,
            rx,
            dbus_rx,
            tray_handle,
            last_disconnect_time: None,
            last_poll_time: Instant::now(),
            health_checker: HealthChecker::new(),
            config_manager,
            app_config,
            kill_switch: KillSwitch::new(),
            switching_in_progress: false,
            switching_target: None,
            switching_from: None,
            switch_completed_time: None,
        }
    }

    /// Dispatch an event to the state machine and sync the shared state
    fn dispatch(&mut self, event: Event) -> Option<TransitionReason> {
        let reason = self.machine.handle_event(event);
        
        // Reset health checker when we successfully connect
        if let VpnState::Connected { ref server } = self.machine.state {
            self.health_checker.reset();
            
            // Save last connected server to config
            if self.app_config.last_server.as_ref() != Some(server) {
                self.app_config.last_server = Some(server.clone());
                if let Err(e) = self.config_manager.save(&self.app_config) {
                    warn!("Failed to save last_server to config: {}", e);
                }
            }
        }
        
        // Always sync shared state after event processing
        if let Ok(mut state) = self.shared_state.try_write() {
            state.state = self.machine.state.clone();
        }
        
        reason
    }

    /// Update kill switch based on current VPN state (call after state transitions)
    async fn update_kill_switch_for_state(&mut self) {
        // Only act if kill switch is enabled in config
        if !self.app_config.kill_switch_enabled {
            return;
        }
        
        match &self.machine.state {
            VpnState::Connected { .. } | VpnState::Degraded { .. } => {
                // Enable/update kill switch when connected
                if !self.kill_switch.is_enabled() {
                    info!("VPN connected - enabling kill switch");
                    if let Err(e) = self.kill_switch.enable().await {
                        warn!("Failed to enable kill switch: {}", e);
                    }
                } else if let Err(e) = self.kill_switch.update().await {
                    warn!("Failed to update kill switch: {}", e);
                }
            }
            VpnState::Disconnected => {
                // Keep kill switch enabled when disconnected (blocks all traffic)
                // This is the core kill switch behavior - prevent leaks when VPN drops
                if self.kill_switch.is_enabled() {
                    debug!("Kill switch active: blocking non-VPN traffic until VPN reconnects");
                }
            }
            _ => {
                // Connecting/Reconnecting/Failed - keep current rules
            }
        }
    }

    /// Sync the shared state with current machine state (for async contexts)
    async fn sync_shared_state(&self) {
        let mut state = self.shared_state.write().await;
        state.state = self.machine.state.clone();
    }

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
        let mut nm_poll_interval = tokio::time::interval(Duration::from_secs(NM_POLL_INTERVAL_SECS));
        
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
                        VpnCommand::RefreshConnections => {
                            self.refresh_connections().await;
                        }
                    }
                }

                // Handle D-Bus events from NetworkManager (real-time)
                Some(event) = self.dbus_rx.recv() => {
                    self.handle_dbus_event(event).await;
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

    /// Handle D-Bus event from NetworkManager
    async fn handle_dbus_event(&mut self, event: NmEvent) {
        debug!("Received D-Bus event: {:?}", event);

        // CRITICAL: Ignore ALL D-Bus events while a VPN switch is in progress
        // handle_connect manages everything during a switch - D-Bus events only cause interference
        if self.switching_in_progress {
            debug!("Ignoring D-Bus event during VPN switch: {:?}", event);
            return;
        }

        // CRITICAL: Ignore late deactivation events from VPN we recently switched FROM
        // D-Bus events can arrive after we've already connected to the new VPN
        if let Some(ref from_server) = self.switching_from {
            if let NmEvent::VpnDeactivated { ref name } = event {
                if name == from_server {
                    // Check if we're within the grace window after switch completed
                    if let Some(completed) = self.switch_completed_time {
                        if completed.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                            info!("Ignoring late deactivation event for switched-from VPN: {}", name);
                            return;
                        }
                    }
                    // Clear the switching_from after processing
                    self.switching_from = None;
                    self.switch_completed_time = None;
                }
            }
        }

        // Check if we're in grace period after intentional disconnect
        if let Some(disconnect_time) = self.last_disconnect_time {
            if disconnect_time.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                debug!("Ignoring D-Bus event during grace period");
                return;
            } else {
                self.last_disconnect_time = None;
            }
        }

        let auto_reconnect = self.shared_state.read().await.auto_reconnect;

        match event {
            NmEvent::VpnActivated { name } => {
                info!("D-Bus: VPN '{}' activated", name);
                
                // CRITICAL: If we already have a different VPN connected, disconnect the OLD one
                // Policy: newest VPN wins (the one that just activated)
                if let Some(current) = self.machine.state.server_name() {
                    if current != &name {
                        info!("External VPN '{}' activated while connected to '{}' - disconnecting old VPN", name, current);
                        let old_vpn = current.to_string();
                        // Update our state to the new VPN first
                        self.dispatch(Event::NmVpnUp { server: name.clone() });
                        self.sync_shared_state().await;
                        self.update_tray();
                        // Then disconnect the old one
                        if let Err(e) = nm_disconnect(&old_vpn).await {
                            warn!("Failed to disconnect old VPN '{}': {}", old_vpn, e);
                        }
                        self.show_notification("VPN Switched", &format!("Now connected to {}", name));
                        return;
                    }
                }
                
                // Also check for any other active VPNs in NetworkManager
                let all_active = nm_get_all_active_vpns().await;
                if all_active.len() > 1 {
                    info!("Multiple VPNs detected ({}) - cleaning up extras", all_active.len());
                    for vpn in &all_active {
                        if vpn.name != name {
                            info!("Disconnecting extra VPN: {}", vpn.name);
                            let _ = nm_disconnect(&vpn.name).await;
                        }
                    }
                }
                
                self.dispatch(Event::NmVpnUp { server: name });
                self.sync_shared_state().await;
                self.update_tray();
            }
            NmEvent::VpnActivating { name } => {
                // Only update if we're not already aware of this activation
                if !matches!(&self.machine.state, VpnState::Connecting { server } if server == &name) {
                    info!("D-Bus: VPN '{}' activating (external)", name);
                    self.dispatch(Event::UserEnable { server: name });
                    self.sync_shared_state().await;
                    self.update_tray();
                }
            }
            NmEvent::VpnDeactivated { name } => {
                info!("D-Bus: VPN '{}' deactivated", name);
                
                // Check if this was our connected VPN
                if let Some(current) = self.machine.state.server_name() {
                    if current == name {
                        if auto_reconnect && matches!(self.machine.state, VpnState::Connected { .. }) {
                            let server = name.clone();
                            self.dispatch(Event::NmVpnDown);
                            self.sync_shared_state().await;
                            self.update_tray();
                            self.show_notification("VPN Disconnected", "Connection dropped, reconnecting...");
                            self.attempt_reconnect(&server).await;
                        } else {
                            self.dispatch(Event::NmVpnDown);
                            self.sync_shared_state().await;
                            self.update_tray();
                        }
                    }
                }
            }
            NmEvent::VpnFailed { name, reason } => {
                warn!("D-Bus: VPN '{}' failed: {}", name, reason);
                
                if auto_reconnect {
                    self.dispatch(Event::NmVpnDown);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification("VPN Failed", &format!("{}: {}", name, reason));
                    self.attempt_reconnect(&name).await;
                } else {
                    self.machine.set_state(
                        VpnState::Failed { server: name, reason },
                        TransitionReason::VpnLost,
                    );
                    self.sync_shared_state().await;
                    self.update_tray();
                }
            }
            NmEvent::ConnectivityChanged { connected } => {
                debug!("D-Bus: Connectivity changed: {}", connected);
                // Could trigger health check here
            }
        }
    }

    /// Initial sync with NetworkManager on startup
    async fn initial_nm_sync(&mut self) {
        // First, check for and clean up multiple simultaneous VPNs
        let all_vpns = nm_get_all_active_vpns().await;
        if all_vpns.len() > 1 {
            warn!("Found {} VPNs active on startup, cleaning up extras", all_vpns.len());
            for extra_vpn in &all_vpns[1..] {
                warn!("Disconnecting extra VPN: {}", extra_vpn.name);
                let _ = nm_disconnect(&extra_vpn.name).await;
            }
            // Wait a moment for disconnect to complete
            sleep(Duration::from_secs(1)).await;
        }

        let active_vpn_info = nm_get_active_vpn_with_state().await;
        
        if let Some(info) = active_vpn_info {
            match info.state {
                NmVpnState::Activated => {
                    info!("Initial sync: VPN {} is active", info.name);
                    self.dispatch(Event::NmVpnUp { server: info.name });
                }
                NmVpnState::Activating => {
                    info!("Initial sync: VPN {} is activating", info.name);
                    self.dispatch(Event::UserEnable { server: info.name });
                }
                _ => {}
            }
        }
        
        self.sync_shared_state().await;
        self.update_tray();
    }

    /// Poll NetworkManager state and dispatch appropriate events
    async fn poll_nm_state(&mut self) {
        // CRITICAL: Skip polling entirely while a VPN switch is in progress
        if self.switching_in_progress {
            debug!("Skipping NM poll during VPN switch");
            return;
        }
        
        // Check if we're in grace period after intentional disconnect
        if let Some(disconnect_time) = self.last_disconnect_time {
            if disconnect_time.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                debug!("In grace period after intentional disconnect");
                return;
            } else {
                self.last_disconnect_time = None;
            }
        }

        // CRITICAL: Detect multiple simultaneous VPNs and clean up extras
        let all_vpns = nm_get_all_active_vpns().await;
        if all_vpns.len() > 1 {
            warn!("Poll detected {} VPNs active: {:?}", all_vpns.len(), 
                  all_vpns.iter().map(|v| &v.name).collect::<Vec<_>>());
            
            // Determine which VPN to keep:
            // 1. If our state says we're connected to one of them, keep that one
            // 2. Otherwise keep the first one (most recently activated)
            let keep_vpn = if let Some(our_server) = self.machine.state.server_name() {
                if all_vpns.iter().any(|v| v.name == our_server) {
                    our_server.to_string()
                } else {
                    all_vpns[0].name.clone()
                }
            } else {
                all_vpns[0].name.clone()
            };
            
            info!("Keeping VPN '{}', disconnecting others", keep_vpn);
            for vpn in &all_vpns {
                if vpn.name != keep_vpn {
                    warn!("Disconnecting extra VPN: {}", vpn.name);
                    let _ = nm_disconnect(&vpn.name).await;
                }
            }
            
            // Update our state to match the kept VPN
            if self.machine.state.server_name() != Some(&keep_vpn) {
                info!("Updating state to match kept VPN: {}", keep_vpn);
                self.dispatch(Event::NmVpnUp { server: keep_vpn });
                self.sync_shared_state().await;
                self.update_tray();
            }
            return; // Don't run the rest of the poll logic
        }

        let active_vpn_info = nm_get_active_vpn_with_state().await;
        let current_state = self.machine.state.clone();
        let auto_reconnect = self.shared_state.read().await.auto_reconnect;

        // Determine what event to dispatch based on NM state vs our state
        match (&current_state, &active_vpn_info) {
            // We think we're connected, but NM shows nothing -> VPN dropped
            (VpnState::Connected { server }, None) => {
                warn!("Connection to {} dropped unexpectedly", server);
                if auto_reconnect {
                    info!("Auto-reconnect enabled, will attempt reconnection");
                    let server_clone = server.clone();
                    self.dispatch(Event::NmVpnDown);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification("VPN Disconnected", "Connection dropped, reconnecting...");
                    self.attempt_reconnect(&server_clone).await;
                } else {
                    self.dispatch(Event::NmVpnDown);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification("VPN Disconnected", "Connection dropped");
                }
            }

            // We think we're connected to X, but NM shows Y -> external switch
            (VpnState::Connected { server: our_server }, Some(info)) 
                if info.state == NmVpnState::Activated && &info.name != our_server => 
            {
                info!("VPN changed externally from {} to {}", our_server, info.name);
                self.dispatch(Event::NmVpnChanged { server: info.name.clone() });
                self.sync_shared_state().await;
                self.update_tray();
            }

            // We're disconnected but NM shows a VPN -> external connection
            (VpnState::Disconnected, Some(info)) if info.state == NmVpnState::Activated => {
                info!("Detected external VPN connection: {}", info.name);
                self.dispatch(Event::NmVpnUp { server: info.name.clone() });
                self.sync_shared_state().await;
                self.update_tray();
            }

            // We're disconnected but NM shows activating -> external activation
            (VpnState::Disconnected, Some(info)) if info.state == NmVpnState::Activating => {
                info!("Detected external VPN activation: {}", info.name);
                self.dispatch(Event::UserEnable { server: info.name.clone() });
                self.sync_shared_state().await;
                self.update_tray();
            }

            // We're connecting and NM confirms it's up -> success
            (VpnState::Connecting { server: target }, Some(info)) 
                if info.state == NmVpnState::Activated && &info.name == target =>
            {
                info!("Connection to {} confirmed by NM poll", target);
                self.dispatch(Event::NmVpnUp { server: info.name.clone() });
                self.sync_shared_state().await;
                self.update_tray();
            }

            // We're in Failed state but NM shows connected -> recovered
            (VpnState::Failed { .. }, Some(info)) if info.state == NmVpnState::Activated => {
                info!("VPN recovered, now connected to {}", info.name);
                self.dispatch(Event::NmVpnUp { server: info.name.clone() });
                self.sync_shared_state().await;
                self.update_tray();
            }

            // Everything else: no event needed
            _ => {}
        }
    }

    /// Force a complete state resync with NetworkManager (after wake from sleep)
    async fn force_state_resync(&mut self) {
        info!("Forcing complete state resync with NetworkManager");
        self.last_disconnect_time = None;
        self.refresh_connections().await;

        let active_vpn_info = nm_get_active_vpn_with_state().await;

        // Force set the state based on what NM reports
        match active_vpn_info {
            Some(info) => match info.state {
                NmVpnState::Activated => {
                    info!("Resync: VPN {} is fully active", info.name);
                    self.machine.set_state(
                        VpnState::Connected { server: info.name },
                        TransitionReason::WakeResync,
                    );
                }
                NmVpnState::Activating => {
                    info!("Resync: VPN {} is activating", info.name);
                    self.machine.set_state(
                        VpnState::Connecting { server: info.name },
                        TransitionReason::WakeResync,
                    );
                }
                _ => {
                    info!("Resync: No active VPN");
                    self.machine.set_state(VpnState::Disconnected, TransitionReason::WakeResync);
                }
            },
            None => {
                if !self.machine.state.is_busy() {
                    info!("Resync: No VPN detected");
                    self.machine.set_state(VpnState::Disconnected, TransitionReason::WakeResync);
                }
            }
        }

        self.sync_shared_state().await;
        self.update_tray();
    }

    /// Run health check when connected
    async fn run_health_check(&mut self) {
        // Only run health checks when in Connected or Degraded state
        let server = match &self.machine.state {
            VpnState::Connected { server } => server.clone(),
            VpnState::Degraded { server } => server.clone(),
            _ => return,
        };

        debug!("Running health check for {}", server);
        
        let result = self.health_checker.check().await;
        
        match result {
            HealthResult::Healthy => {
                // If we were degraded, transition back to connected
                if matches!(self.machine.state, VpnState::Degraded { .. }) {
                    info!("Health check passed, VPN recovered from degraded state");
                    self.dispatch(Event::HealthOk);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification("VPN Recovered", "Connection is healthy again");
                } else {
                    debug!("Health check passed");
                }
            }
            HealthResult::Degraded { latency_ms } => {
                if matches!(self.machine.state, VpnState::Connected { .. }) {
                    warn!("Health check degraded: {}ms latency", latency_ms);
                    self.dispatch(Event::HealthDegraded);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification("VPN Degraded", &format!("High latency: {}ms", latency_ms));
                }
            }
            HealthResult::Dead { reason } => {
                error!("Health check failed: {}", reason);
                let auto_reconnect = self.shared_state.read().await.auto_reconnect;
                
                if auto_reconnect {
                    self.dispatch(Event::HealthDead);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification("VPN Dead", "Connection lost, reconnecting...");
                    self.attempt_reconnect(&server).await;
                } else {
                    self.dispatch(Event::HealthDead);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification("VPN Dead", &reason);
                }
            }
        }
    }

    /// Handle user request to connect to a server
    async fn handle_connect(&mut self, connection_name: &str) {
        info!("Connect requested: {}", connection_name);

        // CRITICAL: Set switching flag to prevent D-Bus events from interfering
        self.switching_in_progress = true;
        self.switching_target = Some(connection_name.to_string());
        
        // Track the VPN we're switching FROM (to ignore late D-Bus events)
        if let Some(current) = self.machine.state.server_name() {
            if current != connection_name {
                self.switching_from = Some(current.to_string());
            }
        }
        
        // Set grace period immediately to block any D-Bus deactivation events
        self.last_disconnect_time = Some(Instant::now());

        // NOTE: We do NOT disable kill switch during VPN switch anymore.
        // The kill switch rules already whitelist all VPN server IPs from NetworkManager,
        // so VPN connections should work even with kill switch enabled.

        // STEP 1: ALWAYS check NM for active VPNs first (don't trust our state machine)
        // This catches VPNs that NM still has active even if our state is wrong
        let all_active = nm_get_all_active_vpns().await;
        info!("NM reports {} active VPN(s): {:?}", all_active.len(), 
              all_active.iter().map(|v| &v.name).collect::<Vec<_>>());
        
        // Also track any active VPNs as "switching from" to ignore their deactivation events
        for vpn in &all_active {
            if vpn.name != connection_name && self.switching_from.is_none() {
                self.switching_from = Some(vpn.name.clone());
            }
        }
        
        // Disconnect ALL VPNs that aren't the one we're connecting to
        for vpn in &all_active {
            if vpn.name != connection_name {
                info!("Disconnecting VPN before switch: {}", vpn.name);
                if let Err(e) = nm_disconnect(&vpn.name).await {
                    warn!("Failed to disconnect {}: {}", vpn.name, e);
                }
            }
        }
        
        // STEP 2: Wait for ALL disconnects to complete (with verification)
        if all_active.iter().any(|v| v.name != connection_name) {
            info!("Waiting for VPN disconnection(s) to complete...");
            for attempt in 1..=DISCONNECT_VERIFY_MAX_ATTEMPTS {
                sleep(Duration::from_millis(DISCONNECT_VERIFY_INTERVAL_MS)).await;
                let remaining = nm_get_all_active_vpns().await;
                let others: Vec<_> = remaining.iter().filter(|v| v.name != connection_name).collect();
                if others.is_empty() {
                    info!("All other VPNs disconnected after {} attempts", attempt);
                    break;
                }
                if attempt == DISCONNECT_VERIFY_MAX_ATTEMPTS {
                    warn!("Disconnect verification timed out after {} attempts", attempt);
                    // Force cleanup
                    for other in &others {
                        warn!("Forcing disconnect of stuck VPN: {}", other.name);
                        let _ = nm_disconnect(&other.name).await;
                    }
                }
                debug!("Still have {} other active VPN(s), attempt {}", others.len(), attempt);
            }
            
            kill_orphan_openvpn_processes().await;
            sleep(Duration::from_secs(POST_DISCONNECT_SETTLE_SECS)).await;
        }
        
        // Final verification before connect
        let final_check = nm_get_all_active_vpns().await;
        let other_vpns: Vec<_> = final_check.iter().filter(|v| v.name != connection_name).collect();
        if !other_vpns.is_empty() {
            error!("CRITICAL: Still have {} other VPN(s) active before connect: {:?}", 
                   other_vpns.len(), other_vpns.iter().map(|v| &v.name).collect::<Vec<_>>());
        }
        
        // Dispatch connecting event for new server
        self.dispatch(Event::UserEnable { server: connection_name.to_string() });
        self.sync_shared_state().await;
        self.update_tray();

        self.show_notification("VPN", &format!("Connecting to {}...", connection_name));

        // Attempt connection with retries
        let mut connection_succeeded = false;
        for attempt in 1..=MAX_CONNECT_ATTEMPTS {
            debug!("Connection attempt {} of {} for {}", attempt, MAX_CONNECT_ATTEMPTS, connection_name);

            match nm_connect(connection_name).await {
                Ok(_) => {
                    // Monitor connection state
                    for _ in 1..=CONNECTION_MONITOR_MAX_ATTEMPTS {
                        sleep(Duration::from_millis(CONNECTION_MONITOR_INTERVAL_MS)).await;
                        
                        match nm_get_vpn_state(connection_name).await {
                            Some(NmVpnState::Activated) => {
                                info!("VPN '{}' successfully activated", connection_name);
                                self.dispatch(Event::NmVpnUp { server: connection_name.to_string() });
                                self.sync_shared_state().await;
                                self.update_tray();
                                self.show_notification("VPN Connected", &format!("Connected to {}", connection_name));
                                connection_succeeded = true;
                                break;
                            }
                            Some(NmVpnState::Activating) => {
                                // Still connecting
                            }
                            Some(NmVpnState::Deactivating) | Some(NmVpnState::Inactive) | None => {
                                break;
                            }
                        }
                    }
                    
                    if connection_succeeded {
                        break;
                    }
                    warn!("Connection monitoring timed out");
                }
                Err(e) => {
                    warn!("Connection attempt {} failed: {}", attempt, e);
                }
            }

            if attempt < MAX_CONNECT_ATTEMPTS {
                sleep(Duration::from_secs(2)).await;
            }
        }

        // NOTE: Kill switch stays enabled throughout - no need to re-enable
        // VPN server IPs are already whitelisted in the rules

        // CRITICAL: Clear switching flags - we're done with the switch
        // BUT keep switching_from and set switch_completed_time to ignore late D-Bus events
        self.switching_in_progress = false;
        self.switching_target = None;
        self.last_disconnect_time = None;
        // Set completion time so late D-Bus events for the old VPN are ignored
        self.switch_completed_time = Some(Instant::now());

        if !connection_succeeded {
            // All attempts failed - also clear switching_from since there's nothing to ignore
            self.switching_from = None;
            self.switch_completed_time = None;
            error!("Failed to connect to {} after {} attempts", connection_name, MAX_CONNECT_ATTEMPTS);
            self.dispatch(Event::Timeout);
            self.sync_shared_state().await;
            self.update_tray();
            self.show_notification("VPN Failed", &format!("Could not connect to {}", connection_name));
        }
    }

    /// Handle user request to disconnect
    async fn handle_disconnect(&mut self) {
        info!("Disconnect requested");

        let connection_name = match self.machine.state.server_name() {
            Some(name) => name.to_string(),
            None => {
                info!("Not connected, nothing to disconnect");
                return;
            }
        };

        self.last_disconnect_time = Some(Instant::now());

        match nm_disconnect(&connection_name).await {
            Ok(_) => {
                info!("Disconnected successfully");
                self.dispatch(Event::UserDisable);
                self.sync_shared_state().await;
                self.update_tray();
                self.show_notification("VPN Disconnected", "VPN connection closed");
            }
            Err(e) => {
                error!("Failed to disconnect: {}", e);
            }
        }
    }

    /// Attempt to reconnect with exponential backoff (triggered by connection drop)
    async fn attempt_reconnect(&mut self, connection_name: &str) {
        let max_attempts = self.machine.max_retries();
        
        // NOTE: Kill switch stays enabled - VPN server IPs are already whitelisted
        // No need to disable/re-enable which would require pkexec prompts
        
        let mut reconnect_succeeded = false;
        
        for attempt in 1..=max_attempts {
            info!("Reconnection attempt {}/{} for {}", attempt, max_attempts, connection_name);

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

            // Calculate backoff delay
            let delay = std::cmp::min(
                RECONNECT_BASE_DELAY_SECS * (attempt as u64),
                RECONNECT_MAX_DELAY_SECS,
            );
            sleep(Duration::from_secs(delay)).await;

            // Attempt connection
            match nm_connect(connection_name).await {
                Ok(_) => {
                    sleep(Duration::from_secs(CONNECTION_VERIFY_DELAY_SECS)).await;
                    
                    if let Some(active) = nm_get_active_vpn().await {
                        if active == connection_name {
                            info!("Successfully reconnected to {}", connection_name);
                            self.dispatch(Event::NmVpnUp { server: connection_name.to_string() });
                            self.sync_shared_state().await;
                            self.update_tray();
                            self.show_notification("VPN Reconnected", &format!("Reconnected to {}", connection_name));
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

    /// Toggle auto-reconnect setting
    async fn toggle_auto_reconnect(&mut self) {
        let new_value = {
            let mut state = self.shared_state.write().await;
            state.auto_reconnect = !state.auto_reconnect;
            state.auto_reconnect
        };
        
        // Save to persistent config
        self.app_config.auto_reconnect = new_value;
        if let Err(e) = self.config_manager.save(&self.app_config) {
            warn!("Failed to save config: {}", e);
        }
        
        info!("Auto-reconnect toggled to: {}", new_value);
        self.update_tray();
        self.show_notification("Auto-Reconnect", if new_value { "Enabled" } else { "Disabled" });
    }

    /// Toggle kill switch (nftables firewall rules that block non-VPN traffic)
    async fn toggle_kill_switch(&mut self) {
        let current_enabled = self.app_config.kill_switch_enabled;
        let new_enabled = !current_enabled;
        
        let result = if new_enabled {
            self.kill_switch.enable().await
        } else {
            self.kill_switch.disable().await
        };
        
        match result {
            Ok(()) => {
                // Update shared state for tray
                {
                    let mut state = self.shared_state.write().await;
                    state.kill_switch = new_enabled;
                }
                
                // Save to persistent config
                self.app_config.kill_switch_enabled = new_enabled;
                if let Err(e) = self.config_manager.save(&self.app_config) {
                    warn!("Failed to save config: {}", e);
                }
                
                info!("Kill switch toggled to: {}", new_enabled);
                self.update_tray();
                self.show_notification(
                    "Kill Switch",
                    if new_enabled { "Enabled - Non-VPN traffic blocked" } else { "Disabled" }
                );
            }
            Err(e) => {
                error!("Failed to toggle kill switch: {}", e);
                self.show_notification("Kill Switch Error", &format!("Failed: {}", e));
            }
        }
    }

    /// Refresh the list of available VPN connections
    async fn refresh_connections(&mut self) {
        info!("Refreshing VPN connections");
        let connections = nm_list_vpn_connections().await;
        {
            let mut state = self.shared_state.write().await;
            state.connections = connections;
        }
        self.update_tray();
    }

    /// Update the tray icon with current state
    fn update_tray(&self) {
        let current_state = match self.shared_state.try_read() {
            Ok(guard) => guard.clone(),
            Err(_) => return,
        };

        let tray_handle = self.tray_handle.clone();
        std::thread::spawn(move || {
            if let Ok(handle_guard) = tray_handle.lock() {
                if let Some(handle) = handle_guard.as_ref() {
                    handle.update(move |tray: &mut VpnTray| {
                        if let Ok(mut cached) = tray.cached_state.write() {
                            *cached = current_state.clone();
                        }
                    });
                }
            }
        });
    }

    /// Show a desktop notification
    fn show_notification(&self, title: &str, body: &str) {
        let title = title.to_string();
        let body = body.to_string();
        std::thread::spawn(move || {
            let _ = Notification::new()
                .summary(&title)
                .body(&body)
                .timeout(5000)
                .show();
        });
    }
}

// ============================================================================
// Instance Lock
// ============================================================================

fn get_lock_file_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .expect("XDG_RUNTIME_DIR not set - cannot safely create lock file");
    PathBuf::from(runtime_dir).join("openvpn-tray.lock")
}

fn acquire_instance_lock() -> Result<File, String> {
    let lock_path = get_lock_file_path();

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| format!("Failed to open lock file: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600));
    }

    let fd = file.as_raw_fd();
    let result = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };

    if result != 0 {
        let errno = std::io::Error::last_os_error();
        if errno.raw_os_error() == Some(libc::EWOULDBLOCK) {
            let mut contents = String::new();
            if let Ok(mut f) = File::open(&lock_path) {
                let _ = f.read_to_string(&mut contents);
            }
            let pid_info = contents.trim().parse::<u32>()
                .map(|pid| format!(" (PID {})", pid))
                .unwrap_or_default();
            return Err(format!("Another instance is already running{}", pid_info));
        }
        return Err(format!("Failed to acquire lock: {}", errno));
    }

    let truncate_result = unsafe { libc::ftruncate(fd, 0) };
    if truncate_result != 0 {
        return Err(format!("Failed to truncate lock file: {}", std::io::Error::last_os_error()));
    }

    use std::io::Seek;
    let mut file = file;
    file.seek(std::io::SeekFrom::Start(0)).map_err(|e| format!("Failed to seek: {}", e))?;
    write!(file, "{}", std::process::id()).map_err(|e| format!("Failed to write PID: {}", e))?;
    file.sync_all().map_err(|e| format!("Failed to sync: {}", e))?;

    info!("Acquired instance lock (PID {})", std::process::id());
    Ok(file)
}

fn release_instance_lock() {
    let lock_path = get_lock_file_path();
    if let Err(e) = fs::remove_file(&lock_path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            warn!("Failed to remove lock file: {}", e);
        }
    } else {
        info!("Released instance lock");
    }
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() {
    env_logger::init();

    let _lock_file = match acquire_instance_lock() {
        Ok(file) => file,
        Err(msg) => {
            eprintln!("{}", msg);
            std::process::exit(1);
        }
    };

    ctrlc::set_handler(move || {
        info!("Shutdown signal received, cleaning up...");
        release_instance_lock();
        std::process::exit(0);
    })
    .expect("Error setting Ctrl-C handler");

    info!("Starting NetworkManager VPN Supervisor (Phase 4: D-Bus Events)");

    let shared_state = Arc::new(RwLock::new(SharedState::default()));
    let (tx, rx) = mpsc::channel(16);
    let (dbus_tx, dbus_rx) = mpsc::channel(32);
    let tray_handle = Arc::new(std::sync::Mutex::new(None));

    // Start D-Bus monitor for real-time NetworkManager events
    let nm_monitor = NmMonitor::new(dbus_tx);
    tokio::spawn(async move {
        if let Err(e) = nm_monitor.run().await {
            error!("D-Bus monitor failed: {}. Falling back to polling only.", e);
        }
    });

    let supervisor = VpnSupervisor::new(shared_state.clone(), rx, dbus_rx, tray_handle.clone());
    tokio::spawn(supervisor.run());

    let tray_service = VpnTray::new(tx);

    info!("Starting system tray");
    let tray_handle_clone = tray_handle.clone();
    std::thread::spawn(move || {
        use ksni::blocking::TrayMethods;
        match tray_service.spawn() {
            Ok(handle) => {
                if let Ok(mut guard) = tray_handle_clone.lock() {
                    *guard = Some(handle);
                }
            }
            Err(e) => {
                error!("Failed to spawn tray: {}", e);
                std::process::exit(1);
            }
        }
    });

    std::future::pending::<()>().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vpn_state_server_name() {
        let state = VpnState::Connected { server: "test".to_string() };
        assert_eq!(state.server_name(), Some("test"));

        let state = VpnState::Disconnected;
        assert_eq!(state.server_name(), None);
    }
}
