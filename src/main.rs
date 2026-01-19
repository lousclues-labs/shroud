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
const DISCONNECT_VERIFY_MAX_ATTEMPTS: u32 = 10;

/// Interval between disconnect verification attempts in milliseconds
const DISCONNECT_VERIFY_INTERVAL_MS: u64 = 500;

/// Interval for syncing cached state to tray in milliseconds
const CACHE_SYNC_INTERVAL_MS: u64 = 50;

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
    /// Tray handle for updating the icon
    tray_handle: Arc<tokio::sync::Mutex<Option<ksni::blocking::Handle<VpnTray>>>>,
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
        tray_handle: Arc<tokio::sync::Mutex<Option<ksni::blocking::Handle<VpnTray>>>>,
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
            self.update_tray().await;

            // Disconnect and wait for it to complete
            let disconnect_result = nm_disconnect(current).await;
            if let Err(e) = disconnect_result {
                warn!("Disconnect failed (continuing anyway): {}", e);
            }

            // Wait and verify disconnect completed
            for _ in 0..DISCONNECT_VERIFY_MAX_ATTEMPTS {
                sleep(Duration::from_millis(DISCONNECT_VERIFY_INTERVAL_MS)).await;
                if nm_get_active_vpn().await.is_none() {
                    debug!("Previous VPN disconnected successfully");
                    break;
                }
            }

            // Extra settle time for NetworkManager
            sleep(Duration::from_secs(1)).await;
        }

        // Step 4: Update state to Connecting
        {
            let mut state = self.state.write().await;
            state.state = VpnState::Connecting {
                server: connection_name.to_string(),
            };
        }
        self.update_tray().await;
        self.show_notification("VPN", &format!("Connecting to {}...", connection_name));

        // Step 5: Attempt connection with retries
        let mut attempts = 0;

        while attempts < MAX_CONNECT_ATTEMPTS {
            attempts += 1;

            match nm_connect(connection_name).await {
                Ok(_) => {
                    // Wait for connection to establish
                    sleep(Duration::from_secs(CONNECTION_VERIFY_DELAY_SECS)).await;

                    // Verify connection
                    if let Some(active) = nm_get_active_vpn().await {
                        if active == connection_name {
                            info!("Successfully connected to {}", connection_name);
                            {
                                let mut state = self.state.write().await;
                                state.state = VpnState::Connected {
                                    server: connection_name.to_string(),
                                };
                            }
                            self.update_tray().await;
                            self.show_notification(
                                "VPN Connected",
                                &format!("Connected to {}", connection_name),
                            );
                            return;
                        }
                    }

                    warn!(
                        "Connection verification failed (attempt {}/{})",
                        attempts, MAX_CONNECT_ATTEMPTS
                    );
                }
                Err(e) => {
                    warn!("Connection attempt {} failed: {}", attempts, e);
                }
            }

            if attempts < MAX_CONNECT_ATTEMPTS {
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
        self.update_tray().await;
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
                    self.update_tray().await;
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

        // Get active VPN from NetworkManager
        let active_vpn = nm_get_active_vpn().await;

        let (current_state, auto_reconnect) = {
            let state = self.state.read().await;
            (state.state.clone(), state.auto_reconnect)
        };

        let mut needs_tray_update = false;

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
            self.update_tray().await;
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

        // Get actual current state from NM
        let active_vpn = nm_get_active_vpn().await;

        {
            let mut state = self.state.write().await;
            match active_vpn {
                Some(conn_name) => {
                    info!("Resync: VPN {} is active", conn_name);
                    state.state = VpnState::Connected {
                        server: conn_name.clone(),
                    };
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

        self.update_tray().await;
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
                self.update_tray().await;
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
            self.update_tray().await;

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
                            self.update_tray().await;
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
        self.update_tray().await;
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
        self.update_tray().await;
    }

    /// Update the tray icon
    async fn update_tray(&self) {
        if let Some(handle) = self.tray_handle.lock().await.as_ref() {
            handle.update(|_tray: &mut VpnTray| {});
        }
    }

    /// Show a desktop notification
    fn show_notification(&self, title: &str, body: &str) {
        let _ = Notification::new()
            .summary(title)
            .body(body)
            .timeout(5000)
            .show();
    }
}

//
// NetworkManager Interface Functions
//

/// Get the active VPN connection name from NetworkManager
async fn nm_get_active_vpn() -> Option<String> {
    debug!("Querying active VPN from NetworkManager");

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

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        // Split on colon, but only take the last 2 fields (TYPE and STATE)
        // This handles connection names that contain colons
        let parts: Vec<&str> = line.rsplitn(3, ':').collect();
        if parts.len() >= 3 {
            let state = parts[0];
            let conn_type = parts[1];
            let name = parts[2];
            
            if conn_type == "vpn" && state == "activated" {
                debug!("Found active VPN: {}", name);
                return Some(name.to_string());
            }
        }
    }

    debug!("No active VPN found");
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
        info!("VPN deactivation successful");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = format!("nmcli failed: {}", stderr.trim());
        error!("{}", msg);
        Err(msg)
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

/// Create a solid color icon in ARGB32 format
///
/// Returns icons in common sizes (16x16, 24x24, 32x32) for different DPI scales.
/// The data is in ARGB32 format with network byte order (big endian).
fn create_colored_icon(r: u8, g: u8, b: u8, a: u8) -> Vec<ksni::Icon> {
    let sizes = [16, 24, 32];
    let pixel = [a, r, g, b]; // ARGB32 in network byte order
    
    sizes
        .iter()
        .map(|&size| {
            let pixel_count = (size * size) as usize;
            let mut data = Vec::with_capacity(pixel_count * 4);
            
            // Efficiently fill with repeated ARGB pixel data
            for _ in 0..pixel_count {
                data.extend_from_slice(&pixel);
            }
            
            ksni::Icon {
                width: size,
                height: size,
                data,
            }
        })
        .collect()
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
    pub fn new(state: Arc<RwLock<SharedState>>, tx: mpsc::Sender<VpnCommand>) -> Self {
        // Create initial cached state
        let cached_state = Arc::new(std::sync::RwLock::new(SharedState::default()));
        
        // Spawn a task to keep cached state synchronized
        let state_clone = state.clone();
        let cached_clone = cached_state.clone();
        tokio::spawn(async move {
            loop {
                {
                    let current = state_clone.read().await;
                    let mut cached = cached_clone.write().unwrap();
                    *cached = current.clone();
                }
                sleep(Duration::from_millis(CACHE_SYNC_INTERVAL_MS)).await;
            }
        });
        
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
        // Use themed icons based on state for a native look
        let state = self.cached_state.read().unwrap();
        match state.state {
            VpnState::Connected { .. } => "network-vpn".to_string(),
            VpnState::Connecting { .. } | VpnState::Reconnecting { .. } => {
                "network-vpn-acquiring".to_string()
            }
            VpnState::Failed { .. } => "network-vpn-disconnected".to_string(),
            VpnState::Disconnected => "network-vpn-disconnected".to_string(),
        }
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
            VpnState::Disconnected => "VPN".to_string(),
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // Return colored icons for glanceable status
        let state = self.cached_state.read().unwrap();
        match state.state {
            // Green for connected
            VpnState::Connected { .. } => create_colored_icon(0, 200, 0, 255),
            // Amber/yellow for connecting/reconnecting
            VpnState::Connecting { .. } | VpnState::Reconnecting { .. } => {
                create_colored_icon(255, 191, 0, 255)
            }
            // Red for disconnected/failed
            VpnState::Failed { .. } | VpnState::Disconnected => {
                create_colored_icon(220, 0, 0, 255)
            }
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

#[tokio::main]
async fn main() {
    // Initialize logging
    env_logger::init();
    info!("Starting NetworkManager VPN Supervisor");

    // Create shared state
    let state = Arc::new(RwLock::new(SharedState::default()));

    // Create command channel
    let (tx, rx) = mpsc::channel(32);

    // Create tray handle container
    let tray_handle = Arc::new(tokio::sync::Mutex::new(None));

    // Create and spawn the supervisor
    let supervisor = VpnSupervisor::new(state.clone(), rx, tray_handle.clone());
    tokio::spawn(supervisor.run());

    // Create the tray
    let tray_service = VpnTray::new(state.clone(), tx);

    // Run the tray (this blocks in a separate thread)
    info!("Starting system tray");
    let tray_handle_clone = tray_handle.clone();
    // Capture the runtime handle before spawning the thread
    let runtime_handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        use ksni::blocking::TrayMethods;
        match tray_service.spawn() {
            Ok(handle) => {
                // Store handle in the async context using the captured runtime handle
                runtime_handle.block_on(async {
                    *tray_handle_clone.lock().await = Some(handle);
                });
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
