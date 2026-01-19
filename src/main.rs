//! OpenVPN System Tray Application with Auto-Reconnect
//!
//! A production-ready system tray application for managing OpenVPN connections
//! with auto-reconnect capabilities for Arch Linux / KDE Plasma (X11).
//!
//! # Arch Linux Setup
//!
//! Install required system packages:
//! ```bash
//! sudo pacman -S openvpn polkit rust networkmanager
//! ```
//!
//! Place your .ovpn files in `/etc/openvpn/` and create an auth file:
//! ```bash
//! sudo nano /etc/openvpn/auth.txt
//! # Add username on first line, password on second line
//! sudo chmod 600 /etc/openvpn/auth.txt
//! ```
//!
//! # Building
//!
//! ```bash
//! cargo build --release
//! ```

use ksni::{menu::StandardItem, Icon, MenuItem, Tray, TrayMethods};
use log::{debug, error, info, warn};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use notify_rust::Notification;
use regex::Regex;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{sleep, timeout, Duration};

/// Interval for polling NetworkManager state in seconds
const NM_POLL_INTERVAL_SECS: u64 = 5;

/// Timeout for nmcli commands in seconds
const NMCLI_TIMEOUT_SECS: u64 = 10;

/// Path to the OpenVPN configuration directory
const OVPN_CONFIG_DIR: &str = "/etc/openvpn";

/// Path to the authentication file for OpenVPN
const AUTH_FILE: &str = "/etc/openvpn/auth.txt";

/// Maximum number of reconnection attempts before giving up
const MAX_RETRY_ATTEMPTS: u32 = 5;

/// Delay between reconnection attempts in seconds
const RETRY_DELAY_SECS: u64 = 5;

/// Interval for polling OpenVPN process termination (milliseconds)
const KILL_POLL_INTERVAL_MS: u64 = 200;

/// Maximum time to wait for OpenVPN process to terminate (seconds)
const KILL_TIMEOUT_SECS: u64 = 5;

/// Maximum number of poll attempts before sending SIGKILL
/// Calculated as: (KILL_TIMEOUT_SECS * 1000) / KILL_POLL_INTERVAL_MS = 25 attempts
const KILL_POLL_MAX_ATTEMPTS: u64 = (KILL_TIMEOUT_SECS * 1000) / KILL_POLL_INTERVAL_MS;

// Compile-time assertion to ensure poll interval evenly divides timeout
const _: () = assert!(
    (KILL_TIMEOUT_SECS * 1000).is_multiple_of(KILL_POLL_INTERVAL_MS),
    "KILL_TIMEOUT_SECS must be evenly divisible by KILL_POLL_INTERVAL_MS"
);

/// Pre-compiled regex for extracting server identifiers from filenames
/// Matches patterns like "us8399", "uk1234", "de5678" (2 letters + digits)
static SERVER_NAME_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([a-zA-Z]{2}\d+)").expect("Invalid regex pattern"));

/// Source of the VPN connection
#[derive(Debug, Clone, PartialEq)]
pub enum VpnSource {
    /// Connection was initiated by this application
    App,
    /// Connection was initiated externally (e.g., NetworkManager)
    External,
}

/// VPN connection state
#[derive(Debug, Clone, PartialEq)]
pub enum VpnState {
    /// No active connection
    Disconnected,
    /// Currently establishing connection to a server
    Connecting(String),
    /// Successfully connected to a server
    Connected { server: String, source: VpnSource },
    /// Connection dropped, attempting to reconnect
    Retrying { server: String, attempt: u32 },
}

impl VpnState {
    /// Get the server name if in a connected or connecting state
    fn server_name(&self) -> Option<&str> {
        match self {
            VpnState::Connected { server, .. } | VpnState::Connecting(server) => Some(server),
            VpnState::Retrying { server, .. } => Some(server),
            VpnState::Disconnected => None,
        }
    }

    /// Check if connection is from an external source
    fn is_external(&self) -> bool {
        matches!(
            self,
            VpnState::Connected {
                source: VpnSource::External,
                ..
            }
        )
    }
}

/// Commands that can be sent to the VPN actor
#[derive(Debug)]
pub enum VpnCommand {
    /// Connect to a specific server
    Connect(String),
    /// Disconnect from the current server
    Disconnect,
    /// Toggle auto-reconnect feature
    ToggleAutoReconnect,
    /// Refresh the list of available servers
    RefreshServers,
    /// Sync with NetworkManager state (internal command)
    SyncNmState,
}

/// Shared state between the tray and the VPN actor
pub struct SharedState {
    /// Current VPN state
    pub state: VpnState,
    /// Whether auto-reconnect is enabled
    pub auto_reconnect: bool,
    /// List of available .ovpn configuration files
    pub servers: Vec<String>,
    /// Whether the user intentionally disconnected
    pub intentional_disconnect: bool,
    /// Name of the active NM VPN connection (for disconnect via nmcli)
    pub nm_connection_name: Option<String>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            state: VpnState::Disconnected,
            auto_reconnect: true,
            servers: Vec::new(),
            intentional_disconnect: false,
            nm_connection_name: None,
        }
    }
}

/// VPN Actor that handles process supervision separately from the GUI thread
pub struct VpnActor {
    /// Shared state accessible by the tray
    state: Arc<RwLock<SharedState>>,
    /// Channel receiver for commands from the tray
    rx: mpsc::Receiver<VpnCommand>,
    /// Currently running OpenVPN child process
    child: Option<Child>,
    /// Tray handle for updating the icon
    tray_handle: Arc<Mutex<Option<ksni::Handle<VpnTray>>>>,
}

impl VpnActor {
    /// Create a new VPN actor
    pub fn new(
        state: Arc<RwLock<SharedState>>,
        rx: mpsc::Receiver<VpnCommand>,
        tray_handle: Arc<Mutex<Option<ksni::Handle<VpnTray>>>>,
    ) -> Self {
        Self {
            state,
            rx,
            child: None,
            tray_handle,
        }
    }

    /// Check if the child process is still running
    /// Returns true if the process exists and has not exited
    fn is_child_running(&mut self) -> bool {
        self.child
            .as_mut()
            .map(|c| c.try_wait().ok().flatten().is_none())
            .unwrap_or(false)
    }

    /// Run the actor's main loop
    pub async fn run(mut self) {
        info!("VPN actor starting");
        
        // Initial server refresh and NM state sync
        self.refresh_servers().await;
        self.sync_nm_state().await;

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
                            self.handle_disconnect(true).await;
                        }
                        VpnCommand::ToggleAutoReconnect => {
                            self.toggle_auto_reconnect().await;
                        }
                        VpnCommand::RefreshServers => {
                            self.refresh_servers().await;
                        }
                        VpnCommand::SyncNmState => {
                            self.sync_nm_state().await;
                        }
                    }
                }

                // Poll NetworkManager state periodically
                _ = nm_poll_interval.tick() => {
                    debug!("Polling NetworkManager state");
                    self.sync_nm_state().await;
                }

                // Monitor child process if one is running
                _ = async {
                    if let Some(ref mut child) = self.child {
                        child.wait().await
                    } else {
                        // Sleep for a long time if no child to monitor
                        // Using a bounded sleep instead of pending() to avoid potential resource issues
                        sleep(Duration::from_secs(86400)).await; // 24 hours
                        Ok(std::process::ExitStatus::default())
                    }
                } => {
                    self.handle_process_exit().await;
                }
            }
        }
    }

    /// Handle connection to a server
    async fn handle_connect(&mut self, server: &str) {
        info!("Connecting to server: {}", server);
        
        // Check current state and disconnect if needed (switching servers or NM-managed)
        let (needs_disconnect, is_external, nm_conn_name) = {
            let state = self.state.read().await;
            let current_server = state.state.server_name();
            let switching = current_server.is_some() && current_server != Some(server);
            let is_external = state.state.is_external();
            let nm_conn = state.nm_connection_name.clone();
            (switching || is_external, is_external, nm_conn)
        };

        if needs_disconnect {
            // Set intentional_disconnect flag BEFORE disconnecting to prevent
            // auto-reconnect logic from firing for the old server during switch
            {
                let mut state = self.state.write().await;
                state.intentional_disconnect = true;
            }

            // Handle NM-managed (external) connections
            if is_external {
                if let Some(conn_name) = nm_conn_name {
                    debug!("Disconnecting NM connection {} before switching", conn_name);
                    let _ = Command::new("nmcli")
                        .arg("con")
                        .arg("down")
                        .arg(&conn_name)
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .await;
                    // Wait briefly to ensure NM cleanup completes
                    sleep(Duration::from_millis(500)).await;
                    // Clear NM connection name only after the NM connection is successfully down
                    {
                        let mut state = self.state.write().await;
                        state.nm_connection_name = None;
                    }
                }
            }

            // Handle app-managed connections: first try to terminate the pkexec wrapper
            if let Some(mut child) = self.child.take() {
                if let Some(pid) = child.id() {
                    debug!("Sending SIGTERM to pkexec process (PID: {}) for server switch", pid);
                    let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
                }
                debug!("Waiting for old pkexec process to exit");
                let _ = child.wait().await;
            }

            // CRITICAL: pkexec does not forward signals to its children.
            // The openvpn process running as root will continue even after pkexec dies.
            // We must explicitly kill the openvpn process using pkexec pkill.
            if !is_external {
                self.kill_openvpn_process().await;
            }

            debug!("Old VPN connection terminated, proceeding with new connection");
        }

        // Update state to connecting - intentional_disconnect is reset to false only
        // AFTER the old connection is confirmed dead and we are ready to start the new one.
        // nm_connection_name was already cleared after successful NM disconnect above.
        {
            let mut state = self.state.write().await;
            state.state = VpnState::Connecting(server.to_string());
            state.intentional_disconnect = false;
        }
        self.update_tray().await;
        self.send_notification("OpenVPN", &format!("Connecting to {}...", server));

        // Start the OpenVPN process
        match self.start_openvpn(server).await {
            Ok(child) => {
                self.child = Some(child);
                debug!("OpenVPN process started, waiting for connection");
                // Give OpenVPN some time to establish connection
                sleep(Duration::from_secs(3)).await;

                // Check if process is still running (connection likely successful)
                if self.is_child_running() {
                    info!("Connected to {}", server);
                    let mut state = self.state.write().await;
                    state.state = VpnState::Connected {
                        server: server.to_string(),
                        source: VpnSource::App,
                    };
                    drop(state);
                    self.update_tray().await;
                    self.send_notification("OpenVPN", &format!("Connected to {}", server));
                }
            }
            Err(e) => {
                error!("Failed to start OpenVPN: {}", e);
                let mut state = self.state.write().await;
                state.state = VpnState::Disconnected;
                drop(state);
                self.update_tray().await;
                self.send_notification("OpenVPN Error", &format!("Failed to connect: {}", e));
            }
        }
    }

    /// Handle disconnection
    async fn handle_disconnect(&mut self, intentional: bool) {
        info!("Disconnecting (intentional: {})", intentional);
        
        // Get current state to determine if this is an external connection
        let (is_external, nm_conn_name) = {
            let state = self.state.read().await;
            (
                state.state.is_external(),
                state.nm_connection_name.clone(),
            )
        };

        // Set intentional flag before killing
        {
            let mut state = self.state.write().await;
            state.intentional_disconnect = intentional;
        }

        // If external connection, disconnect via nmcli
        if is_external {
            if let Some(conn_name) = nm_conn_name {
                debug!("Disconnecting external NM connection: {}", conn_name);
                let _ = Command::new("nmcli")
                    .arg("con")
                    .arg("down")
                    .arg(&conn_name)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
            }
        } else {
            // First, try to terminate the child process (the pkexec wrapper)
            if let Some(ref child) = self.child {
                if let Some(pid) = child.id() {
                    debug!("Sending SIGTERM to pkexec process (PID: {})", pid);
                    let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
                }
            }

            // CRITICAL: pkexec does not forward signals to its children.
            // The openvpn process running as root will continue even after pkexec dies.
            // We must explicitly kill the openvpn process using pkexec pkill.
            self.kill_openvpn_process().await;
        }

        if intentional {
            // Wait for process to exit (if app-managed)
            if let Some(ref mut child) = self.child {
                let _ = child.wait().await;
            }
            self.child = None;

            let mut state = self.state.write().await;
            state.state = VpnState::Disconnected;
            state.nm_connection_name = None;
            drop(state);
            self.update_tray().await;
            self.send_notification("OpenVPN", "Disconnected");
        }
    }

    /// Handle unexpected process exit (for auto-reconnect)
    async fn handle_process_exit(&mut self) {
        debug!("OpenVPN process exited");
        
        // Ensure any orphaned openvpn process is killed before proceeding
        // This handles the case where pkexec dies but openvpn continues running
        self.kill_openvpn_process().await;
        
        let (should_reconnect, server) = {
            let state = self.state.read().await;

            // Only auto-reconnect if:
            // 1. It wasn't an intentional disconnect
            // 2. Auto-reconnect is enabled
            // 3. We were in a connected (app-initiated), connecting, or retrying state
            // 4. The connection was NOT external (NM-managed)
            let is_app_connection = matches!(
                &state.state,
                VpnState::Connected { source: VpnSource::App, .. }
                    | VpnState::Connecting(_)
                    | VpnState::Retrying { .. }
            );
            let should_reconnect = !state.intentional_disconnect
                && state.auto_reconnect
                && is_app_connection;

            debug!(
                "Process exit analysis: intentional={}, auto_reconnect={}, is_app={}, should_reconnect={}",
                state.intentional_disconnect, state.auto_reconnect, is_app_connection, should_reconnect
            );

            let server = state.state.server_name().map(|s| s.to_string());
            (should_reconnect, server)
        };

        self.child = None;

        if should_reconnect {
            if let Some(server) = server {
                info!("Initiating auto-reconnect to {}", server);
                self.attempt_reconnect(&server).await;
            }
        } else {
            let mut state = self.state.write().await;
            if !state.intentional_disconnect {
                debug!("Setting state to Disconnected (no auto-reconnect)");
                state.state = VpnState::Disconnected;
                drop(state);
                self.update_tray().await;
            }
        }
    }

    /// Attempt to reconnect with retry logic
    async fn attempt_reconnect(&mut self, server: &str) {
        let mut attempt = 1;

        while attempt <= MAX_RETRY_ATTEMPTS {
            // Check if user intentionally disconnected during retry
            {
                let state = self.state.read().await;
                if state.intentional_disconnect || !state.auto_reconnect {
                    info!("Reconnect cancelled (intentional disconnect or auto-reconnect disabled)");
                    return;
                }
            }

            info!("Reconnection attempt {}/{} to {}", attempt, MAX_RETRY_ATTEMPTS, server);
            
            // Update state to retrying
            {
                let mut state = self.state.write().await;
                state.state = VpnState::Retrying {
                    server: server.to_string(),
                    attempt,
                };
            }
            self.update_tray().await;
            self.send_notification(
                "OpenVPN",
                &format!("Connection lost. Reconnecting (attempt {}/{})", attempt, MAX_RETRY_ATTEMPTS),
            );

            // Wait before retrying
            debug!("Waiting {} seconds before retry", RETRY_DELAY_SECS);
            sleep(Duration::from_secs(RETRY_DELAY_SECS)).await;

            // Check again if user disconnected during wait
            {
                let state = self.state.read().await;
                if state.intentional_disconnect || !state.auto_reconnect {
                    info!("Reconnect cancelled during wait");
                    return;
                }
            }

            // Ensure any zombie openvpn process from previous attempt is killed
            // before trying to start a new connection
            self.kill_openvpn_process().await;

            // Try to connect
            match self.start_openvpn(server).await {
                Ok(child) => {
                    self.child = Some(child);
                    debug!("OpenVPN process started, waiting for connection");
                    // Give OpenVPN some time to establish connection
                    sleep(Duration::from_secs(3)).await;

                    // Check if process is still running
                    if self.is_child_running() {
                        info!("Reconnected successfully to {}", server);
                        let mut state = self.state.write().await;
                        state.state = VpnState::Connected {
                            server: server.to_string(),
                            source: VpnSource::App,
                        };
                        drop(state);
                        self.update_tray().await;
                        self.send_notification("OpenVPN", &format!("Reconnected to {}", server));
                        return;
                    } else {
                        // Process exited immediately, will retry
                        warn!("OpenVPN process exited immediately, will retry");
                        self.child = None;
                    }
                }
                Err(e) => {
                    error!("Reconnection attempt {} failed: {}", attempt, e);
                }
            }

            attempt += 1;
        }

        // Max retries exceeded
        {
            let mut state = self.state.write().await;
            state.state = VpnState::Disconnected;
        }
        self.update_tray().await;
        self.send_notification(
            "OpenVPN Error",
            "Max reconnection attempts reached. Please connect manually.",
        );
    }

    /// Start the OpenVPN process using pkexec
    async fn start_openvpn(&self, server: &str) -> Result<Child, std::io::Error> {
        let config_path = format!("{}/{}", OVPN_CONFIG_DIR, server);

        Command::new("pkexec")
            .arg("openvpn")
            .arg("--config")
            .arg(&config_path)
            .arg("--auth-user-pass")
            .arg(AUTH_FILE)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }

    /// Kill any running OpenVPN process using pkexec pkill
    ///
    /// This is necessary because pkexec does not forward signals to its children.
    /// When we kill the pkexec process, the openvpn process running as root becomes
    /// orphaned and continues running. This method uses pkexec pkill to directly
    /// terminate the openvpn process with root privileges.
    ///
    /// Note: This will kill ALL openvpn processes on the system, not just ones
    /// started by this application. This is intentional - this application is
    /// designed to be the sole manager of OpenVPN connections, and any other
    /// openvpn processes would conflict with new connections anyway.
    ///
    /// This method implements robust termination with:
    /// 1. Send SIGTERM via pkill
    /// 2. Poll for up to KILL_TIMEOUT_SECS to verify process termination
    /// 3. If still running, send SIGKILL as a last resort
    async fn kill_openvpn_process(&self) {
        debug!("Killing any running OpenVPN processes with pkexec pkill");
        
        // First, send SIGTERM to gracefully terminate
        let result = Command::new("pkexec")
            .arg("pkill")
            .arg("-x")  // Exact match on process name
            .arg("openvpn")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        match result {
            Ok(status) => {
                if status.success() {
                    debug!("Sent SIGTERM to OpenVPN process(es)");
                } else {
                    // pkill exit codes: 0 = matched, 1 = no match, 2 = syntax error, 3 = fatal
                    // Exit code 1 (no processes matched) means no process to kill, we're done
                    debug!("pkill returned non-zero (exit code {}), likely no openvpn processes running",
                           status.code().unwrap_or(-1));
                    return;
                }
            }
            Err(e) => {
                warn!("Failed to execute pkexec pkill: {}", e);
                return;
            }
        }

        // Poll to verify the process has terminated
        for attempt in 1..=KILL_POLL_MAX_ATTEMPTS {
            // Use pkill -0 to check if process exists (signal 0 checks existence without killing)
            let check_result = Command::new("pkexec")
                .arg("pkill")
                .arg("-0")  // Signal 0: check existence only
                .arg("-x")  // Exact match on process name
                .arg("openvpn")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;

            match check_result {
                Ok(status) => {
                    if !status.success() {
                        // Exit code 1 means no matching process found - success!
                        debug!("OpenVPN process terminated after {} poll attempts", attempt);
                        return;
                    }
                    // Process still exists, continue polling
                    debug!("OpenVPN process still running (poll attempt {}/{})", 
                           attempt, KILL_POLL_MAX_ATTEMPTS);
                }
                Err(e) => {
                    warn!("Failed to check OpenVPN process status: {}", e);
                    // Continue polling in case of transient error
                }
            }
            
            // Wait before next poll attempt (moved to end of loop to check immediately on first attempt)
            sleep(Duration::from_millis(KILL_POLL_INTERVAL_MS)).await;
        }

        // Process still running after timeout, send SIGKILL as last resort
        warn!("OpenVPN process did not terminate within {} seconds, sending SIGKILL", 
              KILL_TIMEOUT_SECS);
        
        let kill_result = Command::new("pkexec")
            .arg("pkill")
            .arg("-9")  // SIGKILL
            .arg("-x")  // Exact match on process name
            .arg("openvpn")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        match kill_result {
            Ok(status) => {
                if status.success() {
                    debug!("Sent SIGKILL to OpenVPN process(es)");
                } else {
                    debug!("SIGKILL pkill returned non-zero (exit code {})",
                           status.code().unwrap_or(-1));
                }
            }
            Err(e) => {
                warn!("Failed to execute SIGKILL pkill: {}", e);
            }
        }

        // Brief delay after SIGKILL to ensure cleanup
        sleep(Duration::from_millis(KILL_POLL_INTERVAL_MS)).await;
    }

    /// Toggle auto-reconnect feature
    async fn toggle_auto_reconnect(&mut self) {
        let mut state = self.state.write().await;
        state.auto_reconnect = !state.auto_reconnect;
        let enabled = state.auto_reconnect;
        drop(state);
        self.update_tray().await;
        self.send_notification(
            "OpenVPN",
            if enabled {
                "Auto-reconnect enabled"
            } else {
                "Auto-reconnect disabled"
            },
        );
    }

    /// Refresh the list of available servers
    async fn refresh_servers(&mut self) {
        debug!("Refreshing server list");
        let servers = scan_ovpn_files().await;
        info!("Found {} VPN configuration files", servers.len());
        let mut state = self.state.write().await;
        state.servers = servers;
        drop(state);
        self.update_tray().await;
    }

    /// Sync state with NetworkManager
    ///
    /// This method queries NetworkManager for active VPN connections and
    /// reconciles the internal state. NM state takes precedence when:
    /// - NM shows a VPN connected but internal state is disconnected
    /// - NM shows no VPN but internal state (external) shows connected
    async fn sync_nm_state(&mut self) {
        // Query NM for active VPN connections with timeout
        let nm_vpn = query_nm_active_vpn().await;

        let (needs_tray_update, state_change) = {
            let mut state = self.state.write().await;

            match (&state.state.clone(), &nm_vpn) {
                // Case 1: NM shows VPN connected, but we show disconnected
                // -> Update to Connected (External)
                (VpnState::Disconnected, Some((conn_name, server_name))) => {
                    // Only update if we didn't intentionally disconnect
                    if !state.intentional_disconnect {
                        state.state = VpnState::Connected {
                            server: server_name.clone(),
                            source: VpnSource::External,
                        };
                        state.nm_connection_name = Some(conn_name.clone());
                        (true, Some(format!("Detected external VPN: {}", server_name)))
                    } else {
                        (false, None)
                    }
                }

                // Case 2: NM shows no VPN, but we show connected (external)
                // -> Update to Disconnected
                (VpnState::Connected { source: VpnSource::External, .. }, None) => {
                    state.state = VpnState::Disconnected;
                    state.nm_connection_name = None;
                    (true, Some("External VPN disconnected".to_string()))
                }

                // Case 3: NM shows VPN connected, we show connected (app-initiated)
                // -> App process may have been picked up by NM, keep as App
                (VpnState::Connected { source: VpnSource::App, .. }, Some(_)) => {
                    // No action needed, app-initiated connection still valid
                    (false, None)
                }

                // Case 4: NM shows different VPN than external state
                (VpnState::Connected { source: VpnSource::External, server: current }, Some((conn_name, new_server))) => {
                    if current != new_server {
                        state.state = VpnState::Connected {
                            server: new_server.clone(),
                            source: VpnSource::External,
                        };
                        state.nm_connection_name = Some(conn_name.clone());
                        (true, Some(format!("External VPN changed to: {}", new_server)))
                    } else {
                        (false, None)
                    }
                }

                // Case 5: NM shows VPN during connecting/retrying state
                // -> Trust the internal state as we're actively managing it
                (VpnState::Connecting(_) | VpnState::Retrying { .. }, _) => {
                    // No action during transitional states
                    (false, None)
                }

                // Case 6: NM shows no VPN, app shows connected (app-initiated)
                // -> The process exit handler will deal with this
                (VpnState::Connected { source: VpnSource::App, .. }, None) => {
                    // Let the process monitor handle this
                    (false, None)
                }

                // Default: No state change needed
                _ => (false, None),
            }
        }; // state lock is released here

        if let Some(change) = state_change {
            info!("NM sync: {}", change);
        }

        if needs_tray_update {
            self.update_tray().await;
        }
    }

    /// Update the tray icon
    async fn update_tray(&self) {
        let handle = self.tray_handle.lock().await;
        if let Some(ref h) = *handle {
            if h.update(|_| {}).await.is_none() {
                debug!("Tray update returned None (service may have shut down)");
            }
        }
    }

    /// Send a desktop notification
    fn send_notification(&self, summary: &str, body: &str) {
        let _ = Notification::new()
            .summary(summary)
            .body(body)
            .icon("network-vpn")
            .timeout(3000)
            .show();
    }
}

/// Scan for .ovpn files in the config directory
async fn scan_ovpn_files() -> Vec<String> {
    let mut servers = Vec::new();
    let path = PathBuf::from(OVPN_CONFIG_DIR);

    if let Ok(mut entries) = tokio::fs::read_dir(&path).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.ends_with(".ovpn") {
                servers.push(file_name);
            }
        }
    }

    servers.sort();
    servers
}

/// Query NetworkManager for active VPN connections
///
/// Returns Some((connection_name, server_display_name)) if a VPN is active,
/// None otherwise.
///
/// Uses `nmcli -t -f NAME,TYPE,STATE con show --active` to find active VPNs.
/// Note: nmcli output format is "NAME:TYPE:STATE" but connection names can
/// contain colons, so we parse from the end to get TYPE and STATE reliably.
///
/// This function has a timeout to prevent blocking on slow nmcli responses.
async fn query_nm_active_vpn() -> Option<(String, String)> {
    // Execute nmcli with timeout to prevent blocking
    let nmcli_future = Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE,STATE", "con", "show", "--active"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    let output = match timeout(Duration::from_secs(NMCLI_TIMEOUT_SECS), nmcli_future).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            warn!("nmcli execution failed: {}", e);
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

    // Parse output: each line is "NAME:TYPE:STATE"
    // Connection names can contain colons, so we parse from the right
    // to reliably get TYPE and STATE, then the rest is the NAME
    for line in stdout.lines() {
        // Split into at most 3 parts from the right
        // Format: NAME:TYPE:STATE where NAME may contain colons
        if let Some(last_colon) = line.rfind(':') {
            let state = &line[last_colon + 1..];
            let rest = &line[..last_colon];
            
            if let Some(second_last_colon) = rest.rfind(':') {
                let conn_type = &rest[second_last_colon + 1..];
                let name = &rest[..second_last_colon];

                if conn_type == "vpn" && state == "activated" {
                    debug!("NM reports active VPN: {}", name);
                    // Use connection name as both the connection identifier and display name
                    return Some((name.to_string(), name.to_string()));
                }
            }
        }
    }

    None
}

/// Clean filename for display (e.g., us8399.nordvpn.com.udp.ovpn -> us8399)
fn clean_server_name(filename: &str) -> String {
    // Remove .ovpn extension first
    let name = filename.trim_end_matches(".ovpn");

    // Try to extract just the server identifier (before first dot)
    if let Some(caps) = SERVER_NAME_REGEX.captures(name) {
        return caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_else(|| name.to_string());
    }

    // Fallback: return name without extension
    name.to_string()
}

/// System tray implementation
pub struct VpnTray {
    /// Shared state with the VPN actor
    state: Arc<RwLock<SharedState>>,
    /// Channel to send commands to the VPN actor
    tx: mpsc::Sender<VpnCommand>,
}

impl VpnTray {
    /// Create a new tray instance
    pub fn new(state: Arc<RwLock<SharedState>>, tx: mpsc::Sender<VpnCommand>) -> Self {
        Self { state, tx }
    }

    /// Get the current state synchronously (for ksni callbacks)
    fn get_state_blocking(&self) -> SharedState {
        // Use try_read to avoid blocking; return default if lock unavailable
        self.state
            .try_read()
            .map(|s| SharedState {
                state: s.state.clone(),
                auto_reconnect: s.auto_reconnect,
                servers: s.servers.clone(),
                intentional_disconnect: s.intentional_disconnect,
                nm_connection_name: s.nm_connection_name.clone(),
            })
            .unwrap_or_default()
    }
}

impl Tray for VpnTray {
    /// Enable menu popup on left-click (in addition to right-click)
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "openvpn-tray".to_string()
    }

    fn title(&self) -> String {
        "OpenVPN Manager".to_string()
    }

    fn icon_name(&self) -> String {
        let state = self.get_state_blocking();
        match state.state {
            VpnState::Connected { .. } => "network-vpn".to_string(),
            VpnState::Connecting(_) | VpnState::Retrying { .. } => "network-vpn-acquiring".to_string(),
            VpnState::Disconnected => "network-vpn-disconnected".to_string(),
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let state = self.get_state_blocking();
        let color = match state.state {
            VpnState::Connected { .. } => (0, 255, 0, 255),      // Green
            VpnState::Connecting(_) | VpnState::Retrying { .. } => (255, 200, 0, 255), // Yellow/Orange
            VpnState::Disconnected => (255, 0, 0, 255),      // Red
        };

        // Create a simple 22x22 colored icon
        let size = 22;
        let mut argb = Vec::with_capacity((size * size * 4) as usize);
        for _ in 0..(size * size) {
            argb.push(color.3); // Alpha
            argb.push(color.0); // Red
            argb.push(color.1); // Green
            argb.push(color.2); // Blue
        }

        vec![Icon {
            width: size,
            height: size,
            data: argb,
        }]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let state = self.get_state_blocking();
        let description = match &state.state {
            VpnState::Connected { server, source } => {
                let source_indicator = if *source == VpnSource::External { " (NM)" } else { "" };
                format!("Connected to {}{}", clean_server_name(server), source_indicator)
            }
            VpnState::Connecting(server) => format!("Connecting to {}...", clean_server_name(server)),
            VpnState::Retrying { server, attempt } => {
                format!("Reconnecting to {} (attempt {})", clean_server_name(server), attempt)
            }
            VpnState::Disconnected => "Disconnected".to_string(),
        };

        ksni::ToolTip {
            icon_name: self.icon_name(),
            icon_pixmap: Vec::new(),
            title: "OpenVPN Manager".to_string(),
            description,
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = self.get_state_blocking();
        let mut items: Vec<MenuItem<Self>> = Vec::new();

        // Status header
        let status_text = match &state.state {
            VpnState::Connected { server, source } => {
                let source_indicator = if *source == VpnSource::External { " (NM)" } else { "" };
                format!("● Connected: {}{}", clean_server_name(server), source_indicator)
            }
            VpnState::Connecting(server) => format!("◐ Connecting: {}", clean_server_name(server)),
            VpnState::Retrying { server, attempt } => {
                format!("◐ Retrying: {} ({}/{})", clean_server_name(server), attempt, MAX_RETRY_ATTEMPTS)
            }
            VpnState::Disconnected => "○ Disconnected".to_string(),
        };

        items.push(MenuItem::Standard(StandardItem {
            label: status_text,
            enabled: false,
            ..Default::default()
        }));

        items.push(MenuItem::Separator);

        // Server list
        let current_server = state.state.server_name().map(|s| s.to_string());
        let servers = state.servers.clone();

        for server in servers {
            let display_name = clean_server_name(&server);
            let is_current = current_server.as_deref() == Some(&server);
            let server_clone = server.clone();
            let tx = self.tx.clone();

            items.push(MenuItem::Standard(StandardItem {
                label: if is_current {
                    format!("✓ {}", display_name)
                } else {
                    format!("  {}", display_name)
                },
                enabled: !is_current || matches!(state.state, VpnState::Disconnected),
                activate: Box::new(move |_| {
                    let tx = tx.clone();
                    let server = server_clone.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(VpnCommand::Connect(server)).await;
                    });
                }),
                ..Default::default()
            }));
        }

        items.push(MenuItem::Separator);

        // Disconnect button
        let tx_disconnect = self.tx.clone();
        let is_connected = !matches!(state.state, VpnState::Disconnected);
        items.push(MenuItem::Standard(StandardItem {
            label: "Disconnect".to_string(),
            enabled: is_connected,
            activate: Box::new(move |_| {
                let tx = tx_disconnect.clone();
                tokio::spawn(async move {
                    let _ = tx.send(VpnCommand::Disconnect).await;
                });
            }),
            ..Default::default()
        }));

        items.push(MenuItem::Separator);

        // Auto-reconnect toggle
        let tx_auto = self.tx.clone();
        items.push(MenuItem::Checkmark(ksni::menu::CheckmarkItem {
            label: "Auto-Reconnect".to_string(),
            enabled: true,
            checked: state.auto_reconnect,
            activate: Box::new(move |_| {
                let tx = tx_auto.clone();
                tokio::spawn(async move {
                    let _ = tx.send(VpnCommand::ToggleAutoReconnect).await;
                });
            }),
            ..Default::default()
        }));

        // Refresh servers
        let tx_refresh = self.tx.clone();
        items.push(MenuItem::Standard(StandardItem {
            label: "Refresh Servers".to_string(),
            enabled: true,
            activate: Box::new(move |_| {
                let tx = tx_refresh.clone();
                tokio::spawn(async move {
                    let _ = tx.send(VpnCommand::RefreshServers).await;
                });
            }),
            ..Default::default()
        }));

        items.push(MenuItem::Separator);

        // Quit button
        items.push(MenuItem::Standard(StandardItem {
            label: "Quit".to_string(),
            enabled: true,
            activate: Box::new(|_| {
                std::process::exit(0);
            }),
            ..Default::default()
        }));

        items
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging from RUST_LOG environment variable
    // Default to "info" level if not set
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("OpenVPN Tray starting");
    
    // Shared state between tray and actor
    let state = Arc::new(RwLock::new(SharedState::default()));

    // Command channel from tray to actor
    let (tx, rx) = mpsc::channel::<VpnCommand>(32);

    // Tray handle holder
    let tray_handle: Arc<Mutex<Option<ksni::Handle<VpnTray>>>> = Arc::new(Mutex::new(None));

    // Create the tray
    let tray = VpnTray::new(Arc::clone(&state), tx);

    // Spawn the tray service (ksni 0.3 API)
    let handle = tray.spawn().await?;
    
    // Store the handle
    {
        let mut h = tray_handle.lock().await;
        *h = Some(handle);
    }

    // Create and run the VPN actor
    let actor = VpnActor::new(Arc::clone(&state), rx, Arc::clone(&tray_handle));

    // Handle SIGTERM/SIGINT for graceful shutdown
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    tokio::select! {
        _ = actor.run() => {}
        _ = ctrl_c => {
            info!("Received SIGINT, shutting down...");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM, shutting down...");
        }
    }

    info!("OpenVPN Tray shutting down");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_server_name() {
        assert_eq!(clean_server_name("us8399.nordvpn.com.udp.ovpn"), "us8399");
        assert_eq!(clean_server_name("uk1234.nordvpn.com.tcp.ovpn"), "uk1234");
        assert_eq!(clean_server_name("de5678.ovpn"), "de5678");
        assert_eq!(clean_server_name("custom-server.ovpn"), "custom-server");
        assert_eq!(clean_server_name("simple.ovpn"), "simple");
    }

    #[test]
    fn test_vpn_state_server_name() {
        assert_eq!(VpnState::Disconnected.server_name(), None);
        assert_eq!(
            VpnState::Connecting("test.ovpn".to_string()).server_name(),
            Some("test.ovpn")
        );
        assert_eq!(
            VpnState::Connected {
                server: "test.ovpn".to_string(),
                source: VpnSource::App
            }
            .server_name(),
            Some("test.ovpn")
        );
        assert_eq!(
            VpnState::Connected {
                server: "external.ovpn".to_string(),
                source: VpnSource::External
            }
            .server_name(),
            Some("external.ovpn")
        );
        assert_eq!(
            VpnState::Retrying {
                server: "test.ovpn".to_string(),
                attempt: 1
            }
            .server_name(),
            Some("test.ovpn")
        );
    }

    #[test]
    fn test_vpn_state_is_external() {
        assert!(!VpnState::Disconnected.is_external());
        assert!(!VpnState::Connecting("test.ovpn".to_string()).is_external());
        assert!(
            !VpnState::Connected {
                server: "test.ovpn".to_string(),
                source: VpnSource::App
            }
            .is_external()
        );
        assert!(
            VpnState::Connected {
                server: "external.ovpn".to_string(),
                source: VpnSource::External
            }
            .is_external()
        );
        assert!(
            !VpnState::Retrying {
                server: "test.ovpn".to_string(),
                attempt: 1
            }
            .is_external()
        );
    }

    #[test]
    fn test_shared_state_default() {
        let state = SharedState::default();
        assert_eq!(state.state, VpnState::Disconnected);
        assert!(state.auto_reconnect);
        assert!(state.servers.is_empty());
        assert!(!state.intentional_disconnect);
        assert!(state.nm_connection_name.is_none());
    }

    #[test]
    fn test_vpn_source_equality() {
        assert_eq!(VpnSource::App, VpnSource::App);
        assert_eq!(VpnSource::External, VpnSource::External);
        assert_ne!(VpnSource::App, VpnSource::External);
    }
}
