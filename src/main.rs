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

use ksni::{menu::StandardItem, MenuItem, Tray};
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

/// Base delay for exponential backoff in seconds
const RECONNECT_BASE_DELAY_SECS: u64 = 2;

/// Cap on reconnect delay in seconds
const RECONNECT_MAX_DELAY_SECS: u64 = 30;

/// Grace period after intentional disconnect to prevent false drop detection
const POST_DISCONNECT_GRACE_SECS: u64 = 5;

/// Wait after disconnect before initiating new connection
const POST_DISCONNECT_SETTLE_SECS: u64 = 2;

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
        info!("Connecting to: {}", connection_name);

        // Check current state
        let current_state = {
            let state = self.state.read().await;
            state.state.clone()
        };

        // If we're already connected to a different server, disconnect first
        if let Some(current_server) = current_state.server_name() {
            if current_server != connection_name {
                info!(
                    "Switching from {} to {}",
                    current_server, connection_name
                );
                // Disconnect the old connection
                let _ = nm_disconnect(current_server).await;
                // Wait for resources to settle
                sleep(Duration::from_secs(POST_DISCONNECT_SETTLE_SECS)).await;
            }
        }

        // Update state to Connecting
        {
            let mut state = self.state.write().await;
            state.state = VpnState::Connecting {
                server: connection_name.to_string(),
            };
        }
        self.update_tray().await;

        // Attempt connection via NetworkManager
        match nm_connect(connection_name).await {
            Ok(_) => {
                info!("Connection command sent successfully");
                // Wait for connection to establish
                sleep(Duration::from_secs(CONNECTION_VERIFY_DELAY_SECS)).await;

                // Verify connection
                if let Some(active_vpn) = nm_get_active_vpn().await {
                    if active_vpn == connection_name {
                        info!("Successfully connected to {}", connection_name);
                        {
                            let mut state = self.state.write().await;
                            state.state = VpnState::Connected {
                                server: connection_name.to_string(),
                            };
                        }
                        self.update_tray().await;
                        self.show_notification("VPN Connected", &format!("Connected to {}", connection_name));
                        return;
                    }
                }

                // Connection verification failed
                warn!("Connection verification failed for {}", connection_name);
                {
                    let mut state = self.state.write().await;
                    state.state = VpnState::Failed {
                        server: connection_name.to_string(),
                        reason: "Connection verification failed".to_string(),
                    };
                }
                self.update_tray().await;
            }
            Err(e) => {
                error!("Failed to connect to {}: {}", connection_name, e);
                {
                    let mut state = self.state.write().await;
                    state.state = VpnState::Failed {
                        server: connection_name.to_string(),
                        reason: e,
                    };
                }
                self.update_tray().await;
                self.show_notification("VPN Connection Failed", &format!("Failed to connect to {}", connection_name));
            }
        }
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
                // Grace period expired
                self.last_disconnect_time = None;
            }
        }

        // Get active VPN from NetworkManager
        let active_vpn = nm_get_active_vpn().await;

        let (current_state, auto_reconnect) = {
            let state = self.state.read().await;
            (state.state.clone(), state.auto_reconnect)
        };

        match (&current_state, active_vpn) {
            // We think we're connected, but NM shows nothing - connection dropped!
            (VpnState::Connected { server }, None) => {
                warn!("Connection to {} dropped unexpectedly", server);
                
                if auto_reconnect {
                    info!("Auto-reconnect enabled, attempting to reconnect");
                    let server_clone = server.clone();
                    self.attempt_reconnect(&server_clone, 1).await;
                } else {
                    info!("Auto-reconnect disabled, staying disconnected");
                    {
                        let mut state = self.state.write().await;
                        state.state = VpnState::Disconnected;
                    }
                    self.update_tray().await;
                    self.show_notification("VPN Disconnected", "Connection dropped");
                }
            }
            // We're disconnected but NM shows a VPN - external connection
            (VpnState::Disconnected, Some(vpn_name)) => {
                info!("Detected external VPN connection: {}", vpn_name);
                {
                    let mut state = self.state.write().await;
                    state.state = VpnState::Connected { server: vpn_name };
                }
                self.update_tray().await;
            }
            // We're in a stable state and NM agrees
            _ => {
                debug!("State synchronized with NetworkManager");
            }
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
                sleep(Duration::from_millis(100)).await;
            }
        });
        
        Self {
            cached_state,
            tx,
        }
    }

    /// Get the status text for the current state
    fn get_status_text(state: &VpnState) -> String {
        match state {
            VpnState::Disconnected => "Disconnected".to_string(),
            VpnState::Connecting { server } => format!("Connecting to {}...", server),
            VpnState::Connected { server } => format!("Connected: {}", server),
            VpnState::Reconnecting {
                server,
                attempt,
                max_attempts,
            } => format!("Reconnecting {}/{}: {}", attempt, max_attempts, server),
            VpnState::Failed { server, reason } => format!("Failed ({}): {}", server, reason),
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
        // Return empty to prefer icon_pixmap with colored status icons
        String::new()
    }

    fn title(&self) -> String {
        let state = self.cached_state.read().unwrap();
        Self::get_status_text(&state.state)
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let state = self.cached_state.read().unwrap();
        // Define colors as (Red, Green, Blue, Alpha)
        let (r, g, b, a) = match state.state {
            VpnState::Connected { .. } => (0, 200, 0, 255),        // Bright Green
            VpnState::Connecting { .. } => (255, 200, 0, 255),     // Yellow
            VpnState::Reconnecting { .. } => (255, 165, 0, 255),   // Orange
            VpnState::Failed { .. } => (200, 0, 0, 255),           // Red
            VpnState::Disconnected => (128, 128, 128, 255),        // Gray
        };

        // Use 24x24 for better visibility on HiDPI
        let size = 24i32;
        let mut data = Vec::with_capacity((size * size * 4) as usize);

        // StatusNotifierItem expects ARGB format (Alpha, Red, Green, Blue)
        for _ in 0..(size * size) {
            data.push(a);  // Alpha
            data.push(r);  // Red
            data.push(g);  // Green
            data.push(b);  // Blue
        }

        vec![ksni::Icon {
            width: size,
            height: size,
            data,
        }]
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = self.cached_state.read().unwrap().clone();
        let mut items = Vec::new();

        // Status label
        items.push(MenuItem::Standard(StandardItem {
            label: Self::get_status_text(&state.state),
            enabled: false,
            ..Default::default()
        }));

        items.push(MenuItem::Separator);

        // VPN connections
        if state.connections.is_empty() {
            items.push(MenuItem::Standard(StandardItem {
                label: "No VPN connections found".to_string(),
                enabled: false,
                ..Default::default()
            }));
        } else {
            for connection in &state.connections {
                let conn_clone: String = connection.clone();
                let is_current = state.state.server_name() == Some(connection);
                items.push(MenuItem::Standard(StandardItem {
                    label: if is_current {
                        format!("● {}", connection)
                    } else {
                        connection.clone()
                    },
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

        // Disconnect button
        let can_disconnect = state.state.server_name().is_some();
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

        // Auto-reconnect toggle
        items.push(MenuItem::Standard(StandardItem {
            label: if state.auto_reconnect {
                "✓ Auto-Reconnect".to_string()
            } else {
                "Auto-Reconnect".to_string()
            },
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
    std::thread::spawn(move || {
        use ksni::blocking::TrayMethods;
        match tray_service.spawn() {
            Ok(handle) => {
                // Store handle in the async context
                tokio::runtime::Handle::current().block_on(async {
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
}
