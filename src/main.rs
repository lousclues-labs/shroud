//! NetworkManager VPN Supervisor with System Tray
//!
//! A production-ready system tray application for managing VPN connections via NetworkManager
//! with auto-reconnect capabilities for Arch Linux / KDE Plasma (X11).
//!
//! # Arch Linux Setup
//!
//! Install required system packages:
//! ```bash
//! sudo pacman -S networkmanager rust
//! ```
//!
//! Configure VPN connections in NetworkManager (via nmcli or KDE settings).
//!
//! # Building
//!
//! ```bash
//! cargo build --release
//! ```

use ksni::{menu::CheckmarkItem, menu::StandardItem, MenuItem, Tray};
use log::{debug, error, info, warn};
use notify_rust::Notification;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;
use tokio::process::Command;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, timeout, Duration};

/// Poll NetworkManager state every 2 seconds
const NM_POLL_INTERVAL_SECS: u64 = 2;

/// Timeout for nmcli commands in seconds
const NMCLI_TIMEOUT_SECS: u64 = 30;

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

/// VPN connection state
#[derive(Debug, Clone, PartialEq)]
pub enum VpnState {
    /// No active connection
    Disconnected,
    /// Currently establishing connection to a server
    Connecting { server: String },
    /// Successfully connected to a server
    Connected { server: String },
    /// Connection dropped, attempting to reconnect
    Reconnecting {
        server: String,
        attempt: u32,
        max_attempts: u32,
    },
    /// Connection failed
    Failed { server: String, reason: String },
}

impl VpnState {
    /// Get the server name if in a connected or connecting state
    fn server_name(&self) -> Option<&str> {
        match self {
            VpnState::Connected { server } | VpnState::Connecting { server } => Some(server),
            VpnState::Reconnecting { server, .. } => Some(server),
            VpnState::Failed { server, .. } => Some(server),
            VpnState::Disconnected => None,
        }
    }
}

/// NetworkManager VPN connection state from nmcli
#[derive(Debug, Clone, PartialEq)]
pub enum NmVpnState {
    /// VPN is activating (connecting)
    Activating,
    /// VPN is fully activated (connected)
    Activated,
    /// VPN is deactivating (disconnecting)
    Deactivating,
    /// VPN is not active
    Inactive,
}

/// Result from querying active VPN with state information
#[derive(Debug, Clone)]
pub struct ActiveVpnInfo {
    /// Connection name
    pub name: String,
    /// Current state
    pub state: NmVpnState,
}

/// Commands that can be sent to the VPN supervisor
#[derive(Debug)]
pub enum VpnCommand {
    /// Connect to a specific server
    Connect(String),
    /// Disconnect from the current server
    Disconnect,
    /// Toggle auto-reconnect feature
    ToggleAutoReconnect,
    /// Refresh the list of available VPN connections
    RefreshConnections,
}

/// Shared state between the tray and the VPN supervisor
#[derive(Clone)]
pub struct SharedState {
    /// Current VPN state
    pub state: VpnState,
    /// Whether auto-reconnect is enabled
    pub auto_reconnect: bool,
    /// List of available VPN connections from NetworkManager
    pub connections: Vec<String>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            state: VpnState::Disconnected,
            auto_reconnect: true,
            connections: Vec::new(),
        }
    }
}

/// VPN Supervisor that manages VPN connections via NetworkManager
pub struct VpnSupervisor {
    /// Shared state accessible by the tray
    state: Arc<RwLock<SharedState>>,
    /// Channel receiver for commands from the tray
    rx: mpsc::Receiver<VpnCommand>,
    /// Tray handle for updating the icon (using std::sync::Mutex for blocking context compatibility)
    tray_handle: Arc<std::sync::Mutex<Option<ksni::blocking::Handle<VpnTray>>>>,
    /// Timestamp of last intentional disconnect
    last_disconnect_time: Option<Instant>,
    /// Timestamp of last polling tick (for detecting time jumps/sleep/wake)
    last_poll_time: Instant,
}

impl VpnSupervisor {
    /// Create a new VPN supervisor
    pub fn new(
        state: Arc<RwLock<SharedState>>,
        rx: mpsc::Receiver<VpnCommand>,
        tray_handle: Arc<std::sync::Mutex<Option<ksni::blocking::Handle<VpnTray>>>>,
    ) -> Self {
        Self {
            state,
            rx,
            tray_handle,
            last_disconnect_time: None,
            last_poll_time: Instant::now(),
        }
    }

    /// Run the supervisor's main loop
    pub async fn run(mut self) {
        info!("VPN supervisor starting");

        // Initial connection refresh and state sync
        self.refresh_connections().await;
        self.sync_with_nm().await;
        self.last_poll_time = Instant::now();

        // Create an interval for NM polling
        let mut nm_poll_interval = tokio::time::interval(Duration::from_secs(NM_POLL_INTERVAL_SECS));

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
                    // Detect time jumps (sleep/wake events)
                    let elapsed = self.last_poll_time.elapsed();
                    if elapsed > Duration::from_secs(NM_POLL_INTERVAL_SECS * 3) {
                        warn!(
                            "Time jump detected ({:.1}s since last poll), forcing state resync",
                            elapsed.as_secs_f32()
                        );
                        self.force_state_resync().await;
                    } else {
                        debug!("Polling NetworkManager state");
                        self.sync_with_nm().await;
                    }
                    self.last_poll_time = Instant::now();
                }
            }
        }
    }

    /// Handle connection to a server
    async fn handle_connect(&mut self, connection_name: &str) {
        info!("Connect requested: {}", connection_name);

        // Step 1: Get current state and determine what to do
        let current_server = {
            let state = self.state.read().await;
            state.state.server_name().map(|s| s.to_string())
        };

        // Step 2: If already connected to this server, do nothing
        if current_server.as_deref() == Some(connection_name) {
            info!("Already connected to {}", connection_name);
            return;
        }

        // Step 3: If connected to different server, disconnect first with verification
        if let Some(ref current) = current_server {
            info!(
                "Disconnecting from {} before connecting to {}",
                current, connection_name
            );

            // Update state to show we're transitioning
            {
                let mut state = self.state.write().await;
                state.state = VpnState::Connecting {
                    server: connection_name.to_string(),
                };
            }
            self.update_tray();

            // Disconnect and wait for it to complete
            debug!("Calling nm_disconnect for: {}", current);
            let disconnect_result = nm_disconnect(current).await;
            if let Err(e) = disconnect_result {
                warn!("Disconnect command failed (continuing anyway): {}", e);
            } else {
                debug!("Disconnect command completed successfully");
            }

            // Wait and verify the SPECIFIC connection is fully disconnected
            // Must check for both "activated" AND "deactivating" states
            let mut disconnected = false;
            for attempt in 1..=DISCONNECT_VERIFY_MAX_ATTEMPTS {
                sleep(Duration::from_millis(DISCONNECT_VERIFY_INTERVAL_MS)).await;
                
                debug!("Disconnect verification attempt {} of {}", attempt, DISCONNECT_VERIFY_MAX_ATTEMPTS);
                
                // Check the precise state of the connection we're trying to disconnect
                match nm_get_vpn_state(current).await {
                    Some(NmVpnState::Activated) => {
                        debug!("VPN '{}' still activated, waiting...", current);
                    }
                    Some(NmVpnState::Deactivating) => {
                        debug!("VPN '{}' is deactivating, waiting for complete disconnect...", current);
                    }
                    Some(NmVpnState::Activating) => {
                        // Unusual state during disconnect, but wait for it
                        debug!("VPN '{}' is activating (unexpected during disconnect), waiting...", current);
                    }
                    Some(NmVpnState::Inactive) | None => {
                        // Connection is no longer in active list - fully disconnected
                        info!("Previous VPN '{}' disconnected successfully (no longer in active connections)", current);
                        disconnected = true;
                        break;
                    }
                }
            }
            
            // Log timeout if disconnect wasn't verified
            if !disconnected {
                warn!("Disconnect verification timed out for '{}' after {} attempts ({} seconds)", 
                      current, DISCONNECT_VERIFY_MAX_ATTEMPTS, 
                      (DISCONNECT_VERIFY_MAX_ATTEMPTS as u64 * DISCONNECT_VERIFY_INTERVAL_MS) / 1000);
            }

            // Clean up any orphan OpenVPN processes before connecting
            // This ensures a clean state even if disconnect verification succeeded
            if !disconnected {
                warn!("Specific VPN '{}' may not have disconnected properly, cleaning up orphan processes", current);
                kill_orphan_openvpn_processes().await;
            } else {
                // Always clean up to ensure no stale processes remain
                debug!("Cleaning up any orphan processes after successful disconnect");
                kill_orphan_openvpn_processes().await;
            }

            // Extra settle time for NetworkManager to fully release resources
            debug!("Waiting {} seconds for NetworkManager to settle after disconnect", POST_DISCONNECT_SETTLE_SECS);
            sleep(Duration::from_secs(POST_DISCONNECT_SETTLE_SECS)).await;
        }

        // Step 4: Update state to Connecting
        {
            let mut state = self.state.write().await;
            state.state = VpnState::Connecting {
                server: connection_name.to_string(),
            };
        }
        self.update_tray();
        self.show_notification("VPN", &format!("Connecting to {}...", connection_name));

        // Step 5: Attempt connection with retries
        let mut attempts = 0;

        while attempts < MAX_CONNECT_ATTEMPTS {
            attempts += 1;
            
            debug!("Connection attempt {} of {} for {}", attempts, MAX_CONNECT_ATTEMPTS, connection_name);

            match nm_connect(connection_name).await {
                Ok(_) => {
                    // Actively monitor the connection state instead of waiting a fixed time
                    debug!("Connection command succeeded, monitoring state transition");
                    
                    let mut saw_activating = false;
                    let mut connection_succeeded = false;
                    let mut failure_reason: Option<String> = None;
                    
                    for monitor_attempt in 1..=CONNECTION_MONITOR_MAX_ATTEMPTS {
                        sleep(Duration::from_millis(CONNECTION_MONITOR_INTERVAL_MS)).await;
                        
                        match nm_get_vpn_state(connection_name).await {
                            Some(NmVpnState::Activated) => {
                                info!("VPN '{}' successfully activated after {} checks", 
                                      connection_name, monitor_attempt);
                                connection_succeeded = true;
                                break;
                            }
                            Some(NmVpnState::Activating) => {
                                if !saw_activating {
                                    debug!("VPN '{}' is activating...", connection_name);
                                    saw_activating = true;
                                }
                                // Connection is in progress, keep waiting
                            }
                            Some(NmVpnState::Deactivating) => {
                                // This indicates the connection attempt failed and is being cleaned up
                                warn!("VPN '{}' entered deactivating state - connection attempt failed", connection_name);
                                failure_reason = Some("Connection entered deactivating state".to_string());
                                break;
                            }
                            Some(NmVpnState::Inactive) | None => {
                                if saw_activating {
                                    // Connection was activating but disappeared - failed
                                    warn!("VPN '{}' disappeared during activation - connection failed", connection_name);
                                    failure_reason = Some("Connection disappeared during activation".to_string());
                                    break;
                                }
                                // Might not have appeared in active list yet, keep waiting briefly
                                if monitor_attempt > 10 {
                                    warn!("VPN '{}' not appearing in active connections after {} checks", 
                                          connection_name, monitor_attempt);
                                    failure_reason = Some("Connection never became active".to_string());
                                    break;
                                }
                            }
                        }
                        
                        // Update tray periodically during connection
                        if monitor_attempt % 5 == 0 {
                            self.update_tray();
                        }
                    }
                    
                    if connection_succeeded {
                        {
                            let mut state = self.state.write().await;
                            state.state = VpnState::Connected {
                                server: connection_name.to_string(),
                            };
                        }
                        // Force immediate sync with NetworkManager after successful connection
                        self.sync_with_nm().await;
                        self.update_tray();
                        self.show_notification(
                            "VPN Connected",
                            &format!("Connected to {}", connection_name),
                        );
                        return;
                    }
                    
                    warn!(
                        "Connection monitoring failed (attempt {}/{}): {}",
                        attempts, MAX_CONNECT_ATTEMPTS,
                        failure_reason.as_deref().unwrap_or("timeout")
                    );
                }
                Err(e) => {
                    warn!("Connection attempt {} failed: {}", attempts, e);
                }
            }

            if attempts < MAX_CONNECT_ATTEMPTS {
                debug!("Waiting 2 seconds before retry");
                sleep(Duration::from_secs(2)).await;
            }
        }

        // Connection failed after all attempts
        error!(
            "Failed to connect to {} after {} attempts",
            connection_name, MAX_CONNECT_ATTEMPTS
        );
        {
            let mut state = self.state.write().await;
            state.state = VpnState::Failed {
                server: connection_name.to_string(),
                reason: "Connection verification failed".to_string(),
            };
        }
        self.update_tray();
        self.show_notification(
            "VPN Failed",
            &format!("Could not connect to {}", connection_name),
        );
    }

    /// Handle disconnect command
    async fn handle_disconnect(&mut self) {
        info!("Disconnecting VPN");

        // Get current connection name
        let connection_name = {
            let state = self.state.read().await;
            state.state.server_name().map(|s| s.to_string())
        };

        if let Some(name) = connection_name {
            // Record intentional disconnect time
            self.last_disconnect_time = Some(Instant::now());

            // Disconnect via NetworkManager
            match nm_disconnect(&name).await {
                Ok(_) => {
                    info!("Disconnected successfully");
                    {
                        let mut state = self.state.write().await;
                        state.state = VpnState::Disconnected;
                    }
                    // Force immediate sync after disconnect to ensure state is accurate
                    self.sync_with_nm().await;
                    self.update_tray();
                    self.show_notification("VPN Disconnected", "VPN connection closed");
                }
                Err(e) => {
                    error!("Failed to disconnect: {}", e);
                }
            }
        }
    }

    /// Sync internal state with NetworkManager
    async fn sync_with_nm(&mut self) {
        // Check if we're in grace period after intentional disconnect
        if let Some(disconnect_time) = self.last_disconnect_time {
            if disconnect_time.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                debug!("In grace period after intentional disconnect");
                return;
            } else {
                self.last_disconnect_time = None;
            }
        }

        // Get active VPN from NetworkManager with state information
        let active_vpn_info = nm_get_active_vpn_with_state().await;

        let (current_state, auto_reconnect) = {
            let state = self.state.read().await;
            (state.state.clone(), state.auto_reconnect)
        };

        let mut needs_tray_update = false;

        // First, check for VPN in "activating" state - should show as Connecting
        if let Some(ref info) = active_vpn_info {
            if info.state == NmVpnState::Activating {
                // A VPN is activating externally, update our state to reflect this
                match &current_state {
                    VpnState::Connecting { server } if server == &info.name => {
                        // Already showing as connecting to this VPN, all good
                        debug!("State synchronized: connecting to {}", info.name);
                    }
                    VpnState::Connected { server } if server == &info.name => {
                        // Unusual: we think connected but it's actually still activating
                        debug!("VPN {} is still activating, updating state", info.name);
                        {
                            let mut state = self.state.write().await;
                            state.state = VpnState::Connecting {
                                server: info.name.clone(),
                            };
                        }
                        needs_tray_update = true;
                    }
                    VpnState::Disconnected => {
                        // External activation started
                        info!("Detected external VPN activation: {}", info.name);
                        {
                            let mut state = self.state.write().await;
                            state.state = VpnState::Connecting {
                                server: info.name.clone(),
                            };
                        }
                        needs_tray_update = true;
                    }
                    _ => {
                        // Other transitional states, let them play out
                        debug!("VPN {} is activating, current state: {:?}", info.name, current_state);
                    }
                }
                
                if needs_tray_update {
                    self.update_tray();
                }
                return;
            }
        }

        // Get the fully activated VPN name for legacy matching logic
        let active_vpn = active_vpn_info
            .filter(|info| info.state == NmVpnState::Activated)
            .map(|info| info.name);

        match (&current_state, &active_vpn) {
            // Case 1: We think we're connected, but NM shows nothing - DROP!
            (VpnState::Connected { server }, None) => {
                warn!("Connection to {} dropped unexpectedly", server);

                if auto_reconnect {
                    info!("Auto-reconnect enabled, attempting to reconnect");
                    let server_clone = server.clone();
                    self.attempt_reconnect(&server_clone, 1).await;
                } else {
                    {
                        let mut state = self.state.write().await;
                        state.state = VpnState::Disconnected;
                    }
                    needs_tray_update = true;
                    self.show_notification("VPN Disconnected", "Connection dropped");
                }
            }

            // Case 2: We think we're connected to X, but NM shows Y - external switch
            (VpnState::Connected { server: our_server }, Some(nm_server))
                if our_server != nm_server =>
            {
                info!(
                    "VPN changed externally from {} to {}",
                    our_server, nm_server
                );
                {
                    let mut state = self.state.write().await;
                    state.state = VpnState::Connected {
                        server: nm_server.clone(),
                    };
                }
                needs_tray_update = true;
            }

            // Case 3: We're disconnected but NM shows a VPN - external connection
            (VpnState::Disconnected, Some(vpn_name)) => {
                info!("Detected external VPN connection: {}", vpn_name);
                {
                    let mut state = self.state.write().await;
                    state.state = VpnState::Connected {
                        server: vpn_name.clone(),
                    };
                }
                needs_tray_update = true;
            }

            // Case 4: We're connecting and NM shows the target connected - success!
            (VpnState::Connecting { server: target }, Some(active)) if target == active => {
                info!("Connection to {} confirmed by NM sync", target);
                {
                    let mut state = self.state.write().await;
                    state.state = VpnState::Connected {
                        server: target.clone(),
                    };
                }
                needs_tray_update = true;
            }

            // Case 5: We're in Failed state but NM shows connected - recovered!
            (VpnState::Failed { .. }, Some(vpn_name)) => {
                info!("VPN recovered, now connected to {}", vpn_name);
                {
                    let mut state = self.state.write().await;
                    state.state = VpnState::Connected {
                        server: vpn_name.clone(),
                    };
                }
                needs_tray_update = true;
            }

            // Case 5b: We're in Failed state and NM shows nothing - transition to Disconnected
            (VpnState::Failed { server, .. }, None) => {
                info!("Failed connection to {} confirmed, transitioning to Disconnected", server);
                {
                    let mut state = self.state.write().await;
                    state.state = VpnState::Disconnected;
                }
                needs_tray_update = true;
            }

            // Case 6: NM shows nothing, we show disconnected - all good
            (VpnState::Disconnected, None) => {
                debug!("State synchronized: disconnected");
            }

            // Case 7: NM shows same as us - all good
            (VpnState::Connected { server: our_server }, Some(nm_server))
                if our_server == nm_server =>
            {
                debug!("State synchronized: connected to {}", our_server);
            }

            // Other cases: transitional states, let them play out
            _ => {
                debug!("Transitional state, not syncing");
            }
        }

        if needs_tray_update {
            self.update_tray();
        }
    }

    /// Force a complete state resync with NetworkManager
    /// Called after detecting sleep/wake or other anomalies
    async fn force_state_resync(&mut self) {
        info!("Forcing complete state resync with NetworkManager");

        // Clear any stale state
        self.last_disconnect_time = None;

        // Refresh connection list (may have changed during sleep)
        self.refresh_connections().await;

        // Get actual current state from NM with state information
        let active_vpn_info = nm_get_active_vpn_with_state().await;

        {
            let mut state = self.state.write().await;
            match active_vpn_info {
                Some(info) => {
                    match info.state {
                        NmVpnState::Activated => {
                            info!("Resync: VPN {} is fully active", info.name);
                            state.state = VpnState::Connected {
                                server: info.name,
                            };
                        }
                        NmVpnState::Activating => {
                            info!("Resync: VPN {} is activating", info.name);
                            state.state = VpnState::Connecting {
                                server: info.name,
                            };
                        }
                        NmVpnState::Deactivating => {
                            info!("Resync: VPN {} is deactivating, treating as disconnected", info.name);
                            state.state = VpnState::Disconnected;
                        }
                        NmVpnState::Inactive => {
                            info!("Resync: No active VPN");
                            state.state = VpnState::Disconnected;
                        }
                    }
                }
                None => {
                    info!("Resync: No VPN active");
                    // Only set to disconnected if we weren't in the middle of something
                    if !matches!(
                        state.state,
                        VpnState::Connecting { .. } | VpnState::Reconnecting { .. }
                    ) {
                        state.state = VpnState::Disconnected;
                    }
                }
            }
        }

        self.update_tray();
    }

    /// Attempt to reconnect with exponential backoff
    async fn attempt_reconnect(&mut self, connection_name: &str, initial_attempt: u32) {
        let mut attempt = initial_attempt;

        loop {
            if attempt > MAX_RECONNECT_ATTEMPTS {
                error!(
                    "Max reconnection attempts ({}) reached for {}",
                    MAX_RECONNECT_ATTEMPTS, connection_name
                );
                {
                    let mut state = self.state.write().await;
                    state.state = VpnState::Failed {
                        server: connection_name.to_string(),
                        reason: format!(
                            "Max reconnection attempts ({}) exceeded",
                            MAX_RECONNECT_ATTEMPTS
                        ),
                    };
                }
                self.update_tray();
                self.show_notification(
                    "VPN Reconnection Failed",
                    &format!(
                        "Failed to reconnect to {} after {} attempts",
                        connection_name, MAX_RECONNECT_ATTEMPTS
                    ),
                );
                return;
            }

            info!(
                "Reconnection attempt {}/{} for {}",
                attempt, MAX_RECONNECT_ATTEMPTS, connection_name
            );

            // Update state to Reconnecting
            {
                let mut state = self.state.write().await;
                state.state = VpnState::Reconnecting {
                    server: connection_name.to_string(),
                    attempt,
                    max_attempts: MAX_RECONNECT_ATTEMPTS,
                };
            }
            self.update_tray();

            // Calculate backoff delay
            let delay = std::cmp::min(
                RECONNECT_BASE_DELAY_SECS * (attempt as u64),
                RECONNECT_MAX_DELAY_SECS,
            );
            info!("Waiting {} seconds before reconnection attempt", delay);
            sleep(Duration::from_secs(delay)).await;

            // Attempt connection
            match nm_connect(connection_name).await {
                Ok(_) => {
                    info!("Reconnection command sent successfully");
                    // Wait for connection to establish
                    sleep(Duration::from_secs(CONNECTION_VERIFY_DELAY_SECS)).await;

                    // Verify connection
                    if let Some(active_vpn) = nm_get_active_vpn().await {
                        if active_vpn == connection_name {
                            info!("Successfully reconnected to {}", connection_name);
                            {
                                let mut state = self.state.write().await;
                                state.state = VpnState::Connected {
                                    server: connection_name.to_string(),
                                };
                            }
                            self.update_tray();
                            self.show_notification(
                                "VPN Reconnected",
                                &format!("Reconnected to {}", connection_name),
                            );
                            return;
                        }
                    }

                    // Verification failed, try again
                    warn!("Reconnection verification failed, retrying");
                    attempt += 1;
                }
                Err(e) => {
                    error!("Reconnection attempt {} failed: {}", attempt, e);
                    attempt += 1;
                }
            }
        }
    }

    /// Toggle auto-reconnect setting
    async fn toggle_auto_reconnect(&mut self) {
        let new_value = {
            let mut state = self.state.write().await;
            state.auto_reconnect = !state.auto_reconnect;
            state.auto_reconnect
        };
        info!("Auto-reconnect toggled to: {}", new_value);
        self.update_tray();
        self.show_notification(
            "Auto-Reconnect",
            if new_value {
                "Auto-reconnect enabled"
            } else {
                "Auto-reconnect disabled"
            },
        );
    }

    /// Refresh the list of available VPN connections
    async fn refresh_connections(&mut self) {
        info!("Refreshing VPN connections");
        let connections = nm_list_vpn_connections().await;
        {
            let mut state = self.state.write().await;
            state.connections = connections;
        }
        self.update_tray();
    }

    /// Update the tray icon with current state
    /// This ensures the cached state is synchronized before triggering the UI refresh
    fn update_tray(&self) {
        // Get the current state synchronously using try_read to avoid blocking
        let current_state = match self.state.try_read() {
            Ok(guard) => guard.clone(),
            Err(_) => return, // Skip update if state is locked
        };
        
        // Clone the handle for use in the thread
        let tray_handle = self.tray_handle.clone();
        
        // Spawn a regular thread (not tokio) to avoid runtime conflicts
        // ksni's handle.update() internally uses block_on which conflicts with tokio
        std::thread::spawn(move || {
            if let Ok(handle_guard) = tray_handle.lock() {
                if let Some(handle) = handle_guard.as_ref() {
                    handle.update(move |tray: &mut VpnTray| {
                        // Immediately update the cached state within the tray
                        if let Ok(mut cached) = tray.cached_state.write() {
                            *cached = current_state.clone();
                        }
                    });
                }
            }
        });
    }

    /// Show a desktop notification
    /// Uses std::thread::spawn because notify-rust uses block_on internally
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

//
// NetworkManager Interface Functions
//

/// Get the active VPN connection name from NetworkManager (legacy compatibility wrapper)
async fn nm_get_active_vpn() -> Option<String> {
    // Use the enhanced function and filter for fully activated VPNs only
    nm_get_active_vpn_with_state().await
        .filter(|info| info.state == NmVpnState::Activated)
        .map(|info| info.name)
}

/// Get the active VPN with detailed state information from NetworkManager
/// This detects VPNs in activating, activated, or deactivating states
async fn nm_get_active_vpn_with_state() -> Option<ActiveVpnInfo> {
    debug!("Querying active VPN with state from NetworkManager");

    let output = match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new("nmcli")
            .args(["-t", "-f", "NAME,TYPE,STATE", "con", "show", "--active"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            warn!("Failed to execute nmcli: {}", e);
            return None;
        }
        Err(_) => {
            warn!("nmcli timed out after {} seconds", NMCLI_TIMEOUT_SECS);
            return None;
        }
    };

    if !output.status.success() {
        debug!("nmcli returned non-zero exit status");
        return None;
    }

    // Collect all active VPNs with their states
    let mut active_vpns: Vec<ActiveVpnInfo> = Vec::new();
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    debug!("nmcli active connections output: {}", stdout.trim());
    
    for line in stdout.lines() {
        // Split on colon, but only take the last 2 fields (TYPE and STATE)
        // This handles connection names that contain colons
        let parts: Vec<&str> = line.rsplitn(3, ':').collect();
        if parts.len() >= 3 {
            let state_str = parts[0];
            let conn_type = parts[1];
            let name = parts[2];
            
            if conn_type == "vpn" {
                let state = match state_str {
                    "activated" => NmVpnState::Activated,
                    "activating" => NmVpnState::Activating,
                    "deactivating" => NmVpnState::Deactivating,
                    _ => {
                        debug!("Unknown VPN state '{}' for connection '{}'", state_str, name);
                        continue;
                    }
                };
                
                debug!("Found VPN '{}' in state '{:?}'", name, state);
                active_vpns.push(ActiveVpnInfo {
                    name: name.to_string(),
                    state,
                });
            }
        }
    }

    // Priority: activated > activating > deactivating
    // Return the "most connected" VPN
    if let Some(activated) = active_vpns.iter().find(|v| v.state == NmVpnState::Activated) {
        debug!("Found activated VPN: {}", activated.name);
        return Some(activated.clone());
    }
    
    if let Some(activating) = active_vpns.iter().find(|v| v.state == NmVpnState::Activating) {
        debug!("Found activating VPN: {}", activating.name);
        return Some(activating.clone());
    }
    
    if let Some(deactivating) = active_vpns.iter().find(|v| v.state == NmVpnState::Deactivating) {
        debug!("Found deactivating VPN: {}", deactivating.name);
        return Some(deactivating.clone());
    }

    debug!("No active VPN found");
    None
}

/// Get the precise state of a specific VPN connection
/// Returns None if the connection is not active
async fn nm_get_vpn_state(connection_name: &str) -> Option<NmVpnState> {
    debug!("Querying state for VPN connection: {}", connection_name);

    let output = match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new("nmcli")
            .args(["-t", "-f", "NAME,TYPE,STATE", "con", "show", "--active"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            warn!("Failed to execute nmcli: {}", e);
            return None;
        }
        Err(_) => {
            warn!("nmcli timed out after {} seconds", NMCLI_TIMEOUT_SECS);
            return None;
        }
    };

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    for line in stdout.lines() {
        let parts: Vec<&str> = line.rsplitn(3, ':').collect();
        if parts.len() >= 3 {
            let state_str = parts[0];
            let conn_type = parts[1];
            let name = parts[2];
            
            if conn_type == "vpn" && name == connection_name {
                let state = match state_str {
                    "activated" => NmVpnState::Activated,
                    "activating" => NmVpnState::Activating,
                    "deactivating" => NmVpnState::Deactivating,
                    _ => return None,
                };
                debug!("VPN '{}' state: {:?}", connection_name, state);
                return Some(state);
            }
        }
    }

    debug!("VPN '{}' not found in active connections", connection_name);
    None
}

/// List all VPN connections configured in NetworkManager
async fn nm_list_vpn_connections() -> Vec<String> {
    debug!("Listing VPN connections from NetworkManager");

    let output = match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new("nmcli")
            .args(["-t", "-f", "NAME,TYPE", "con", "show"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            warn!("Failed to execute nmcli: {}", e);
            return Vec::new();
        }
        Err(_) => {
            warn!("nmcli timed out after {} seconds", NMCLI_TIMEOUT_SECS);
            return Vec::new();
        }
    };

    if !output.status.success() {
        warn!("nmcli returned non-zero exit status");
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut connections = Vec::new();
    for line in stdout.lines() {
        // Split on colon from the right, only split on last colon (TYPE field)
        // This handles connection names that contain colons
        let parts: Vec<&str> = line.rsplitn(2, ':').collect();
        if parts.len() >= 2 {
            let conn_type = parts[0];
            let name = parts[1];
            
            if conn_type == "vpn" {
                connections.push(name.to_string());
            }
        }
    }

    info!("Found {} VPN connection(s)", connections.len());
    connections
}

/// Get the UUID of a VPN connection by name
async fn nm_get_vpn_uuid(connection_name: &str) -> Option<String> {
    debug!("Getting UUID for VPN connection: {}", connection_name);

    let output = match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new("nmcli")
            .args(["-t", "-f", "UUID,NAME,TYPE", "con", "show"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            warn!("Failed to execute nmcli: {}", e);
            return None;
        }
        Err(_) => {
            warn!("nmcli timed out after {} seconds", NMCLI_TIMEOUT_SECS);
            return None;
        }
    };

    if !output.status.success() {
        debug!("nmcli returned non-zero exit status");
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        // Format: UUID:NAME:TYPE
        // Handle connection names that contain colons by splitting from the right
        // rsplitn returns parts in reverse order: [TYPE, NAME, UUID]
        let parts: Vec<&str> = line.rsplitn(3, ':').collect();
        if parts.len() >= 3 {
            let conn_type = parts[0];  // Last field (TYPE)
            let name = parts[1];       // Middle field (NAME)
            let uuid = parts[2];       // First field (UUID)

            if conn_type == "vpn" && name == connection_name {
                debug!("Found UUID for {}: {}", connection_name, uuid);
                return Some(uuid.to_string());
            }
        }
    }

    debug!("No UUID found for connection: {}", connection_name);
    None
}

/// Connect to a VPN via NetworkManager
async fn nm_connect(connection_name: &str) -> Result<(), String> {
    info!("Activating VPN connection: {}", connection_name);

    let output = match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new("nmcli")
            .args(["con", "up", connection_name])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            let msg = format!("Failed to execute nmcli: {}", e);
            error!("{}", msg);
            return Err(msg);
        }
        Err(_) => {
            let msg = format!("nmcli timed out after {} seconds", NMCLI_TIMEOUT_SECS);
            error!("{}", msg);
            return Err(msg);
        }
    };

    if output.status.success() {
        info!("VPN activation successful");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = format!("nmcli failed: {}", stderr.trim());
        error!("{}", msg);
        Err(msg)
    }
}

/// Disconnect a VPN via NetworkManager
async fn nm_disconnect(connection_name: &str) -> Result<(), String> {
    info!("Deactivating VPN connection: {}", connection_name);

    // First, try to get UUID for more reliable disconnection
    let uuid_opt = nm_get_vpn_uuid(connection_name).await;
    
    // Try disconnecting by UUID first (more reliable)
    if let Some(uuid) = uuid_opt {
        debug!("Attempting disconnect by UUID: {}", uuid);
        let output_result = timeout(
            Duration::from_secs(NMCLI_TIMEOUT_SECS),
            Command::new("nmcli")
                .args(["con", "down", "uuid", &uuid])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output(),
        )
        .await;
        
        match output_result {
            Ok(Ok(output)) => {
                if output.status.success() {
                    info!("VPN deactivation by UUID successful");
                    return Ok(());
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    warn!("Disconnect by UUID failed: {}, trying by name", stderr.trim());
                }
            }
            Ok(Err(e)) => {
                warn!("Failed to execute nmcli with UUID: {}, trying by name", e);
            }
            Err(_) => {
                warn!("nmcli timed out after {} seconds with UUID, trying by name", NMCLI_TIMEOUT_SECS);
            }
        }
    }

    // Fallback: Try disconnecting by name
    debug!("Attempting disconnect by name: {}", connection_name);
    let output = match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new("nmcli")
            .args(["con", "down", connection_name])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            let msg = format!("Failed to execute nmcli: {}", e);
            error!("{}", msg);
            return Err(msg);
        }
        Err(_) => {
            let msg = format!("nmcli timed out after {} seconds", NMCLI_TIMEOUT_SECS);
            error!("{}", msg);
            return Err(msg);
        }
    };

    if output.status.success() {
        info!("VPN deactivation by name successful");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!("Disconnect by name also failed: {}", stderr.trim());
        
        // Last resort: Try device-level disconnect
        // First, find the VPN device
        debug!("Attempting device-level disconnect as last resort");
        let dev_output = match timeout(
            Duration::from_secs(NMCLI_TIMEOUT_SECS),
            Command::new("nmcli")
                .args(["-t", "-f", "DEVICE,TYPE", "dev"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output(),
        )
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                let msg = format!("Failed to list devices: {}", e);
                error!("{}", msg);
                return Err(msg);
            }
            Err(_) => {
                let msg = "Device list timed out".to_string();
                error!("{}", msg);
                return Err(msg);
            }
        };

        if dev_output.status.success() {
            let dev_stdout = String::from_utf8_lossy(&dev_output.stdout);
            for line in dev_stdout.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 2 && parts[1] == "tun" {
                    let device = parts[0];
                    debug!("Found VPN device: {}, attempting disconnect", device);
                    
                    let disconnect_output = match timeout(
                        Duration::from_secs(NMCLI_TIMEOUT_SECS),
                        Command::new("nmcli")
                            .args(["dev", "disconnect", device])
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .output(),
                    )
                    .await
                    {
                        Ok(Ok(output)) => output,
                        Ok(Err(e)) => {
                            warn!("Failed to disconnect device: {}", e);
                            continue;
                        }
                        Err(_) => {
                            warn!("Device disconnect timed out");
                            continue;
                        }
                    };
                    
                    if disconnect_output.status.success() {
                        info!("VPN device disconnect successful");
                        return Ok(());
                    }
                }
            }
        }
        
        let msg = format!("All disconnect methods failed for: {}", connection_name);
        error!("{}", msg);
        Err(msg)
    }
}

/// Kill orphan OpenVPN processes that may be blocking new connections
/// This is a cleanup function for cases where nmcli disconnect doesn't fully clean up
async fn kill_orphan_openvpn_processes() {
    debug!("Checking for orphan OpenVPN processes");
    
    // Check if any openvpn processes exist
    // Use more specific pattern to avoid matching unrelated processes
    let pgrep_output = match Command::new("pgrep")
        .args(["-x", "openvpn"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(output) => output,
        Err(e) => {
            debug!("Failed to run pgrep: {}", e);
            return;
        }
    };
    
    let stdout = String::from_utf8_lossy(&pgrep_output.stdout);
    let pids: Vec<&str> = stdout.lines().collect();
    
    if pids.is_empty() {
        debug!("No OpenVPN processes found");
        return;
    }
    
    warn!("Found {} orphan OpenVPN process(es), attempting cleanup", pids.len());
    
    // Try to kill each process individually using its PID
    for pid in pids {
        if let Ok(pid_num) = pid.trim().parse::<i32>() {
            debug!("Killing OpenVPN process with PID {}", pid_num);
            match Command::new("kill")
                .arg(pid_num.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .await
            {
                Ok(output) if !output.status.success() => {
                    warn!("Failed to kill process {}: exit status {}", pid_num, output.status);
                }
                Err(e) => {
                    warn!("Failed to execute kill for process {}: {}", pid_num, e);
                }
                _ => {}
            }
        }
    }
    
    // Give processes time to terminate
    sleep(Duration::from_millis(500)).await;
    
    // Verify cleanup
    let verify_output = Command::new("pgrep")
        .args(["-x", "openvpn"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;
    
    if let Ok(output) = verify_output {
        if output.status.success() && !output.stdout.is_empty() {
            warn!("Some OpenVPN processes still running after cleanup attempt");
        } else {
            info!("Orphan OpenVPN processes cleaned up successfully");
        }
    }
}

//
// System Tray UI
//

/// Extract a short display name from a VPN connection name
/// e.g., "ie211-dublin" -> "ie211" or "us8399-ashburn" -> "us8399"
fn extract_short_name(full_name: &str) -> &str {
    // Take everything before the first hyphen, or the whole name if no hyphen
    full_name.split('-').next().unwrap_or(full_name)
}

/// Icon type for status indication
#[derive(Debug, Clone, Copy)]
enum IconType {
    Connected,
    Connecting,
    Disconnected,
    Failed,
}

/// Create a status icon with a shield shape in ARGB32 format
///
/// Returns icons in common sizes (16x16, 24x24, 32x32, 48x48) for different DPI scales.
/// The data is in ARGB32 format with network byte order (big endian).
/// Each icon type has a distinctive color and symbol:
/// - Connected: Green with checkmark
/// - Connecting: Amber with dots  
/// - Disconnected: Gray with dash
/// - Failed: Red with X
fn create_status_icon(icon_type: IconType) -> Vec<ksni::Icon> {
    let sizes: [i32; 4] = [16, 24, 32, 48];
    
    sizes
        .iter()
        .map(|&size| {
            let mut data = Vec::with_capacity((size * size * 4) as usize);
            
            // Color palette
            let (bg_r, bg_g, bg_b, fg_r, fg_g, fg_b) = match icon_type {
                IconType::Connected => (46u8, 160, 67, 255, 255, 255),     // Green, white
                IconType::Connecting => (245, 158, 11, 255, 255, 255),     // Amber, white
                IconType::Disconnected => (100, 116, 139, 255, 255, 255),  // Slate, white
                IconType::Failed => (239, 68, 68, 255, 255, 255),          // Red, white
            };
            
            let bg_pixel = [255u8, bg_r, bg_g, bg_b]; // ARGB: full alpha, then RGB
            let fg_pixel = [255u8, fg_r, fg_g, fg_b];
            let transparent = [0u8, 0, 0, 0];
            
            let center = size / 2;
            let radius = (size as f32 * 0.42) as i32;
            let radius_sq = radius * radius;
            
            for y in 0..size {
                for x in 0..size {
                    let dx = x - center;
                    let dy = y - center;
                    let dist_sq = dx * dx + dy * dy;
                    
                    if dist_sq <= radius_sq {
                        // Inside circle - draw the symbol
                        let pixel = match icon_type {
                            IconType::Connected => draw_check(x, y, center, size, &fg_pixel, &bg_pixel),
                            IconType::Connecting => draw_dots(x, y, center, size, &fg_pixel, &bg_pixel),
                            IconType::Disconnected => draw_dash(x, y, center, size, &fg_pixel, &bg_pixel),
                            IconType::Failed => draw_x_mark(x, y, center, size, &fg_pixel, &bg_pixel),
                        };
                        data.extend_from_slice(&pixel);
                    } else {
                        data.extend_from_slice(&transparent);
                    }
                }
            }
            
            ksni::Icon {
                width: size,
                height: size,
                data,
            }
        })
        .collect()
}

/// Draw a checkmark symbol
fn draw_check(x: i32, y: i32, center: i32, size: i32, fg: &[u8; 4], bg: &[u8; 4]) -> [u8; 4] {
    let rx = x - center;
    let ry = y - center;
    let s = size as f32 / 32.0;
    
    // Two strokes forming a checkmark
    let on_short = rx >= (-5.0 * s) as i32 && rx <= (-1.0 * s) as i32
        && ry >= (-1.0 * s) as i32 && ry <= (5.0 * s) as i32
        && ((ry as f32) - (rx as f32 + 3.0 * s) * 1.0).abs() < 2.5 * s;
    
    let on_long = rx >= (-2.0 * s) as i32 && rx <= (7.0 * s) as i32
        && ry >= (-6.0 * s) as i32 && ry <= (4.0 * s) as i32
        && ((ry as f32) + (rx as f32) * 0.7 - 1.0 * s).abs() < 2.5 * s;
    
    if on_short || on_long { *fg } else { *bg }
}

/// Draw three horizontal dots
fn draw_dots(x: i32, y: i32, center: i32, size: i32, fg: &[u8; 4], bg: &[u8; 4]) -> [u8; 4] {
    let rx = x - center;
    let ry = y - center;
    let s = size as f32 / 32.0;
    let dot_r = (2.5 * s) as i32;
    let dot_r_sq = dot_r * dot_r;
    
    for dot_offset in [-6, 0, 6] {
        let dot_x = (dot_offset as f32 * s) as i32;
        let dx = rx - dot_x;
        if dx * dx + ry * ry <= dot_r_sq {
            return *fg;
        }
    }
    *bg
}

/// Draw a horizontal dash
fn draw_dash(x: i32, y: i32, center: i32, size: i32, fg: &[u8; 4], bg: &[u8; 4]) -> [u8; 4] {
    let rx = x - center;
    let ry = y - center;
    let s = size as f32 / 32.0;
    
    let half_w = (8.0 * s) as i32;
    let half_h = (2.5 * s) as i32;
    
    if rx.abs() <= half_w && ry.abs() <= half_h { *fg } else { *bg }
}

/// Draw an X mark
fn draw_x_mark(x: i32, y: i32, center: i32, size: i32, fg: &[u8; 4], bg: &[u8; 4]) -> [u8; 4] {
    let rx = x - center;
    let ry = y - center;
    let s = size as f32 / 32.0;
    let thick = (2.5 * s) as i32;
    let arm = (6.0 * s) as i32;
    
    let on_d1 = (rx - ry).abs() <= thick && rx.abs() <= arm && ry.abs() <= arm;
    let on_d2 = (rx + ry).abs() <= thick && rx.abs() <= arm && ry.abs() <= arm;
    
    if on_d1 || on_d2 { *fg } else { *bg }
}

/// System tray interface
pub struct VpnTray {
    /// Cached state for synchronous tray methods
    cached_state: Arc<std::sync::RwLock<SharedState>>,
    /// Command sender to the supervisor
    tx: mpsc::Sender<VpnCommand>,
}

impl VpnTray {
    /// Create a new tray instance
    pub fn new(_state: Arc<RwLock<SharedState>>, tx: mpsc::Sender<VpnCommand>) -> Self {
        // Create initial cached state
        let cached_state = Arc::new(std::sync::RwLock::new(SharedState::default()));
        
        // NOTE: We do NOT spawn a sync task here because it conflicts with ksni's
        // internal use of block_on. Instead, state synchronization is done via
        // update_tray() which directly updates the cached_state.
        
        Self {
            cached_state,
            tx,
        }
    }
}

impl Tray for VpnTray {
    // Enable left-click to open menu (in addition to right-click)
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "openvpn-tray".to_string()
    }

    fn icon_name(&self) -> String {
        // Return empty string to force use of icon_pixmap() colored icons
        // This ensures the colored indicators (green/yellow/red) are visible
        // in Icons-Only Task Manager which may prioritize icon_name over icon_pixmap
        String::new()
    }

    fn title(&self) -> String {
        let state = self.cached_state.read().unwrap();
        match &state.state {
            VpnState::Connected { server } => format!("🔒 {}", extract_short_name(server)),
            VpnState::Connecting { server } => format!("⏳ {}...", extract_short_name(server)),
            VpnState::Reconnecting {
                server,
                attempt,
                max_attempts,
            } => format!(
                "🔄 {} ({}/{})",
                extract_short_name(server),
                attempt,
                max_attempts
            ),
            VpnState::Failed { server, .. } => format!("❌ {}", extract_short_name(server)),
            VpnState::Disconnected => "⭕ VPN".to_string(),
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // Return status icons with recognizable symbols
        let state = self.cached_state.read().unwrap();
        match state.state {
            // Green circle with checkmark for connected
            VpnState::Connected { .. } => create_status_icon(IconType::Connected),
            // Yellow circle with dots for connecting/reconnecting
            VpnState::Connecting { .. } | VpnState::Reconnecting { .. } => {
                create_status_icon(IconType::Connecting)
            }
            // Red circle with X for failed
            VpnState::Failed { .. } => create_status_icon(IconType::Failed),
            // Gray circle with dash for disconnected
            VpnState::Disconnected => create_status_icon(IconType::Disconnected),
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let state = self.cached_state.read().unwrap();
        let (title, description) = match &state.state {
            VpnState::Connected { server } => (
                format!("🔒 Connected: {}", server),
                "VPN tunnel active".to_string(),
            ),
            VpnState::Connecting { server } => (
                format!("Connecting to {}...", server),
                "Establishing connection".to_string(),
            ),
            VpnState::Reconnecting {
                server,
                attempt,
                max_attempts,
            } => (
                format!("Reconnecting: {}", server),
                format!("Attempt {} of {}", attempt, max_attempts),
            ),
            VpnState::Failed { server, reason } => {
                (format!("Failed: {}", server), reason.clone())
            }
            VpnState::Disconnected => (
                "VPN Disconnected".to_string(),
                "Click to connect to a VPN".to_string(),
            ),
        };

        ksni::ToolTip {
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
            title,
            description,
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = self.cached_state.read().unwrap().clone();
        let mut items = Vec::new();

        // Status header with clear visual indicators
        let status_text = match &state.state {
            VpnState::Connected { server } => format!("🔒 Connected: {}", server),
            VpnState::Connecting { server } => format!("⏳ Connecting: {}...", server),
            VpnState::Reconnecting {
                server,
                attempt,
                max_attempts,
            } => {
                format!("🔄 Reconnecting: {} ({}/{})", server, attempt, max_attempts)
            }
            VpnState::Failed { server, reason } => format!("❌ Failed: {} - {}", server, reason),
            VpnState::Disconnected => "⭕ Disconnected".to_string(),
        };

        items.push(MenuItem::Standard(StandardItem {
            label: status_text,
            enabled: false,
            ..Default::default()
        }));

        items.push(MenuItem::Separator);

        // VPN connections with clear selection state
        if state.connections.is_empty() {
            items.push(MenuItem::Standard(StandardItem {
                label: "No VPN connections configured".to_string(),
                enabled: false,
                ..Default::default()
            }));
            items.push(MenuItem::Standard(StandardItem {
                label: "Use 'nmcli con import' to add VPNs".to_string(),
                enabled: false,
                ..Default::default()
            }));
        } else {
            let current_server = state.state.server_name();
            let is_busy = matches!(
                state.state,
                VpnState::Connecting { .. } | VpnState::Reconnecting { .. }
            );

            for connection in &state.connections {
                let conn_clone = connection.clone();
                let is_current = current_server == Some(connection.as_str());
                let is_connected =
                    matches!(&state.state, VpnState::Connected { server } if server == connection);

                items.push(MenuItem::Standard(StandardItem {
                    label: if is_connected {
                        format!("✓ {} (connected)", extract_short_name(connection))
                    } else if is_current {
                        format!("⋯ {} (in progress)", extract_short_name(connection))
                    } else {
                        format!("  {}", extract_short_name(connection))
                    },
                    enabled: !is_current && !is_busy, // Disable if this one or busy
                    activate: Box::new(move |tray: &mut Self| {
                        let tx = tray.tx.clone();
                        let conn = conn_clone.clone();
                        tokio::spawn(async move {
                            let _ = tx.send(VpnCommand::Connect(conn)).await;
                        });
                    }),
                    ..Default::default()
                }));
            }
        }

        items.push(MenuItem::Separator);

        // Disconnect button - only enabled when connected
        let can_disconnect = matches!(state.state, VpnState::Connected { .. });
        items.push(MenuItem::Standard(StandardItem {
            label: "Disconnect".to_string(),
            enabled: can_disconnect,
            activate: Box::new(|tray: &mut Self| {
                let tx = tray.tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(VpnCommand::Disconnect).await;
                });
            }),
            ..Default::default()
        }));

        items.push(MenuItem::Separator);

        // Auto-reconnect toggle with checkbox
        items.push(MenuItem::Checkmark(CheckmarkItem {
            label: "Auto-Reconnect".to_string(),
            enabled: true,
            checked: state.auto_reconnect,
            activate: Box::new(|tray: &mut Self| {
                let tx = tray.tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(VpnCommand::ToggleAutoReconnect).await;
                });
            }),
            ..Default::default()
        }));

        // Refresh connections
        items.push(MenuItem::Standard(StandardItem {
            label: "Refresh Connections".to_string(),
            enabled: true,
            activate: Box::new(|tray: &mut Self| {
                let tx = tray.tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(VpnCommand::RefreshConnections).await;
                });
            }),
            ..Default::default()
        }));

        items.push(MenuItem::Separator);

        // Quit
        items.push(MenuItem::Standard(StandardItem {
            label: "Quit".to_string(),
            icon_name: "application-exit".to_string(),
            enabled: true,
            activate: Box::new(|_| {
                std::process::exit(0);
            }),
            ..Default::default()
        }));

        items
    }
}

//
// Main
//

/// Path to the lock file for single-instance enforcement
fn get_lock_file_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("openvpn-tray.lock")
}

/// Check if another instance is already running
/// Returns Ok(File) with the lock file if we can proceed, Err if another instance exists
fn acquire_instance_lock() -> Result<File, String> {
    let lock_path = get_lock_file_path();
    
    // Check if lock file exists and contains a valid PID
    if lock_path.exists() {
        if let Ok(mut file) = File::open(&lock_path) {
            let mut contents = String::new();
            if file.read_to_string(&mut contents).is_ok() {
                if let Ok(pid) = contents.trim().parse::<u32>() {
                    // Check if process with this PID is still running
                    let proc_path = format!("/proc/{}", pid);
                    if std::path::Path::new(&proc_path).exists() {
                        // Check if it's actually our process
                        let cmdline_path = format!("/proc/{}/cmdline", pid);
                        if let Ok(cmdline) = fs::read_to_string(&cmdline_path) {
                            if cmdline.contains("openvpn-tray") {
                                return Err(format!(
                                    "Another instance is already running (PID {})",
                                    pid
                                ));
                            }
                        }
                    }
                }
            }
        }
        // Stale lock file, remove it
        let _ = fs::remove_file(&lock_path);
    }
    
    // Create new lock file with our PID
    let mut file = File::create(&lock_path)
        .map_err(|e| format!("Failed to create lock file: {}", e))?;
    
    let pid = std::process::id();
    file.write_all(pid.to_string().as_bytes())
        .map_err(|e| format!("Failed to write PID to lock file: {}", e))?;
    
    Ok(file)
}

/// Clean up lock file on exit
fn release_instance_lock() {
    let lock_path = get_lock_file_path();
    let _ = fs::remove_file(&lock_path);
}

#[tokio::main]
async fn main() {
    // Initialize logging
    env_logger::init();
    
    // Ensure single instance
    let _lock_file = match acquire_instance_lock() {
        Ok(file) => file,
        Err(msg) => {
            eprintln!("{}", msg);
            std::process::exit(1);
        }
    };
    
    // Register cleanup on exit
    ctrlc::set_handler(move || {
        release_instance_lock();
        std::process::exit(0);
    }).expect("Error setting Ctrl-C handler");
    
    info!("Starting NetworkManager VPN Supervisor");

    // Create shared state
    let state = Arc::new(RwLock::new(SharedState::default()));

    // Create command channel
    let (tx, rx) = mpsc::channel(32);

    // Create tray handle container (using std::sync::Mutex for blocking context compatibility)
    let tray_handle = Arc::new(std::sync::Mutex::new(None));

    // Create and spawn the supervisor
    let supervisor = VpnSupervisor::new(state.clone(), rx, tray_handle.clone());
    tokio::spawn(supervisor.run());

    // Create the tray
    let tray_service = VpnTray::new(state.clone(), tx);

    // Run the tray (this blocks in a separate thread)
    info!("Starting system tray");
    let tray_handle_clone = tray_handle.clone();
    std::thread::spawn(move || {
        use ksni::blocking::TrayMethods;
        match tray_service.spawn() {
            Ok(handle) => {
                // Store handle using std::sync::Mutex (no async needed)
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

    // Keep the main task alive
    loop {
        sleep(Duration::from_secs(60)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vpn_state_server_name() {
        let state = VpnState::Connected {
            server: "test-server".to_string(),
        };
        assert_eq!(state.server_name(), Some("test-server"));

        let state = VpnState::Disconnected;
        assert_eq!(state.server_name(), None);
    }

    #[test]
    fn test_shared_state_default() {
        let state = SharedState::default();
        assert_eq!(state.state, VpnState::Disconnected);
        assert!(state.auto_reconnect);
        assert!(state.connections.is_empty());
    }

    #[test]
    fn test_extract_short_name() {
        // Name with hyphen - should take the part before the first hyphen
        assert_eq!(extract_short_name("ie211-dublin"), "ie211");
        assert_eq!(extract_short_name("us8399-ashburn"), "us8399");
        
        // Name with multiple hyphens - should only split on the first
        assert_eq!(extract_short_name("de123-berlin-west"), "de123");
        
        // Name without hyphen - should return the whole name
        assert_eq!(extract_short_name("myvpn"), "myvpn");
        
        // Empty string
        assert_eq!(extract_short_name(""), "");
    }
}
