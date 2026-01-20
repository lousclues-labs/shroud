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

mod health;
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

use crate::health::{HealthChecker, HealthResult};
use crate::nm::{
    connect as nm_connect, disconnect as nm_disconnect, get_active_vpn as nm_get_active_vpn,
    get_active_vpn_with_state as nm_get_active_vpn_with_state, get_vpn_state as nm_get_vpn_state,
    kill_orphan_openvpn_processes, list_vpn_connections as nm_list_vpn_connections,
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
    /// Tray handle for updating the icon
    tray_handle: Arc<std::sync::Mutex<Option<ksni::blocking::Handle<VpnTray>>>>,
    /// Timestamp of last intentional disconnect (for grace period)
    last_disconnect_time: Option<Instant>,
    /// Timestamp of last polling tick (for detecting sleep/wake)
    last_poll_time: Instant,
    /// Health checker for VPN connectivity verification
    health_checker: HealthChecker,
}

impl VpnSupervisor {
    /// Create a new VPN supervisor with formal state machine
    pub fn new(
        shared_state: Arc<RwLock<SharedState>>,
        rx: mpsc::Receiver<VpnCommand>,
        tray_handle: Arc<std::sync::Mutex<Option<ksni::blocking::Handle<VpnTray>>>>,
    ) -> Self {
        let config = StateMachineConfig {
            max_retries: MAX_RECONNECT_ATTEMPTS,
            base_delay_secs: RECONNECT_BASE_DELAY_SECS,
            max_delay_secs: RECONNECT_MAX_DELAY_SECS,
        };
        
        Self {
            machine: StateMachine::with_config(config),
            shared_state,
            rx,
            tray_handle,
            last_disconnect_time: None,
            last_poll_time: Instant::now(),
            health_checker: HealthChecker::new(),
        }
    }

    /// Dispatch an event to the state machine and sync the shared state
    fn dispatch(&mut self, event: Event) -> Option<TransitionReason> {
        let reason = self.machine.handle_event(event);
        
        // Reset health checker when we successfully connect
        if matches!(self.machine.state, VpnState::Connected { .. }) {
            self.health_checker.reset();
        }
        
        // Always sync shared state after event processing
        if let Ok(mut state) = self.shared_state.try_write() {
            state.state = self.machine.state.clone();
        }
        
        reason
    }

    /// Sync the shared state with current machine state (for async contexts)
    async fn sync_shared_state(&self) {
        let mut state = self.shared_state.write().await;
        state.state = self.machine.state.clone();
    }

    /// Run the supervisor's main loop
    pub async fn run(mut self) {
        info!("VPN supervisor starting with formal state machine");

        // Initial connection refresh and state sync
        self.refresh_connections().await;
        self.initial_nm_sync().await;
        self.last_poll_time = Instant::now();

        // Create an interval for NM polling
        let mut nm_poll_interval = tokio::time::interval(Duration::from_secs(NM_POLL_INTERVAL_SECS));
        
        // Create an interval for health checks (only runs when connected)
        let mut health_check_interval = tokio::time::interval(Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS));

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
                        VpnCommand::RefreshConnections => {
                            self.refresh_connections().await;
                        }
                    }
                }

                // Poll NetworkManager state periodically
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
                        debug!("Polling NetworkManager state");
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

    /// Initial sync with NetworkManager on startup
    async fn initial_nm_sync(&mut self) {
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
        // Check if we're in grace period after intentional disconnect
        if let Some(disconnect_time) = self.last_disconnect_time {
            if disconnect_time.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                debug!("In grace period after intentional disconnect");
                return;
            } else {
                self.last_disconnect_time = None;
            }
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

        // Check if already connected to this server
        if let Some(current) = self.machine.state.server_name() {
            if current == connection_name {
                info!("Already connected to {}", connection_name);
                return;
            }
            
            // Need to disconnect first
            info!("Disconnecting from {} before connecting to {}", current, connection_name);
            let current_owned = current.to_string();
            
            // Dispatch connecting event for new server
            self.dispatch(Event::UserEnable { server: connection_name.to_string() });
            self.sync_shared_state().await;
            self.update_tray();
            
            // Perform disconnect
            if let Err(e) = nm_disconnect(&current_owned).await {
                warn!("Disconnect command failed (continuing anyway): {}", e);
            }
            
            // Wait for disconnect to complete
            let mut disconnected = false;
            for attempt in 1..=DISCONNECT_VERIFY_MAX_ATTEMPTS {
                sleep(Duration::from_millis(DISCONNECT_VERIFY_INTERVAL_MS)).await;
                match nm_get_vpn_state(&current_owned).await {
                    Some(NmVpnState::Activated | NmVpnState::Deactivating | NmVpnState::Activating) => {
                        debug!("VPN '{}' still active (attempt {})", current_owned, attempt);
                    }
                    _ => {
                        info!("Previous VPN '{}' disconnected", current_owned);
                        disconnected = true;
                        break;
                    }
                }
            }
            
            if !disconnected {
                warn!("Disconnect verification timed out");
            }
            kill_orphan_openvpn_processes().await;
            sleep(Duration::from_secs(POST_DISCONNECT_SETTLE_SECS)).await;
        } else {
            // Not connected, just dispatch the enable event
            self.dispatch(Event::UserEnable { server: connection_name.to_string() });
            self.sync_shared_state().await;
            self.update_tray();
        }

        self.show_notification("VPN", &format!("Connecting to {}...", connection_name));

        // Attempt connection with retries
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
                                return;
                            }
                            Some(NmVpnState::Activating) => {
                                // Still connecting
                            }
                            Some(NmVpnState::Deactivating) | Some(NmVpnState::Inactive) | None => {
                                break;
                            }
                        }
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

        // All attempts failed
        error!("Failed to connect to {} after {} attempts", connection_name, MAX_CONNECT_ATTEMPTS);
        self.dispatch(Event::Timeout);
        self.sync_shared_state().await;
        self.update_tray();
        self.show_notification("VPN Failed", &format!("Could not connect to {}", connection_name));
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
                            return;
                        }
                    }
                    warn!("Reconnection verification failed");
                }
                Err(e) => {
                    error!("Reconnection attempt {} failed: {}", attempt, e);
                }
            }
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
        info!("Auto-reconnect toggled to: {}", new_value);
        self.update_tray();
        self.show_notification("Auto-Reconnect", if new_value { "Enabled" } else { "Disabled" });
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

    info!("Starting NetworkManager VPN Supervisor (Phase 2: Formal State Machine)");

    let shared_state = Arc::new(RwLock::new(SharedState::default()));
    let (tx, rx) = mpsc::channel(16);
    let tray_handle = Arc::new(std::sync::Mutex::new(None));

    let supervisor = VpnSupervisor::new(shared_state.clone(), rx, tray_handle.clone());
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
