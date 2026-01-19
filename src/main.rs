//! OpenVPN System Tray Application with Auto-Reconnect
//!
//! A production-ready system tray application for managing OpenVPN connections
//! with auto-reconnect capabilities for Arch Linux / KDE Plasma (X11).
//!
//! # Arch Linux Setup
//!
//! Install required system packages:
//! ```bash
//! sudo pacman -S openvpn polkit rust
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

use ksni::{menu::StandardItem, Icon, MenuItem, Tray, TrayService};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use notify_rust::Notification;
use regex::Regex;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{sleep, Duration};

/// Path to the OpenVPN configuration directory
const OVPN_CONFIG_DIR: &str = "/etc/openvpn";

/// Path to the authentication file for OpenVPN
const AUTH_FILE: &str = "/etc/openvpn/auth.txt";

/// Maximum number of reconnection attempts before giving up
const MAX_RETRY_ATTEMPTS: u32 = 5;

/// Delay between reconnection attempts in seconds
const RETRY_DELAY_SECS: u64 = 5;

/// VPN connection state
#[derive(Debug, Clone, PartialEq)]
pub enum VpnState {
    /// No active connection
    Disconnected,
    /// Currently establishing connection to a server
    Connecting(String),
    /// Successfully connected to a server
    Connected(String),
    /// Connection dropped, attempting to reconnect
    Retrying { server: String, attempt: u32 },
}

impl VpnState {
    /// Get the server name if in a connected or connecting state
    fn server_name(&self) -> Option<&str> {
        match self {
            VpnState::Connected(s) | VpnState::Connecting(s) => Some(s),
            VpnState::Retrying { server, .. } => Some(server),
            VpnState::Disconnected => None,
        }
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
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            state: VpnState::Disconnected,
            auto_reconnect: true,
            servers: Vec::new(),
            intentional_disconnect: false,
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

    /// Run the actor's main loop
    pub async fn run(mut self) {
        // Initial server refresh
        self.refresh_servers().await;

        loop {
            tokio::select! {
                // Handle commands from the tray
                Some(cmd) = self.rx.recv() => {
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
                    }
                }

                // Monitor child process if one is running
                _ = async {
                    if let Some(ref mut child) = self.child {
                        child.wait().await
                    } else {
                        // Sleep forever if no child to monitor
                        std::future::pending::<std::io::Result<std::process::ExitStatus>>().await
                    }
                } => {
                    self.handle_process_exit().await;
                }
            }
        }
    }

    /// Handle connection to a server
    async fn handle_connect(&mut self, server: &str) {
        // If already connected to a different server, disconnect first
        {
            let state = self.state.read().await;
            if let Some(current) = state.state.server_name() {
                if current != server {
                    drop(state);
                    self.handle_disconnect(false).await;
                    // Wait for the process to fully exit
                    if let Some(ref mut child) = self.child {
                        let _ = child.wait().await;
                    }
                    self.child = None;
                }
            }
        }

        // Update state to connecting
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
                // Give OpenVPN some time to establish connection
                sleep(Duration::from_secs(3)).await;

                // Check if process is still running (connection likely successful)
                if self.child.as_mut().map(|c| c.try_wait().ok().flatten().is_none()).unwrap_or(false) {
                    let mut state = self.state.write().await;
                    state.state = VpnState::Connected(server.to_string());
                    drop(state);
                    self.update_tray().await;
                    self.send_notification("OpenVPN", &format!("Connected to {}", server));
                }
            }
            Err(e) => {
                eprintln!("Failed to start OpenVPN: {}", e);
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
        // Set intentional flag before killing
        {
            let mut state = self.state.write().await;
            state.intentional_disconnect = intentional;
        }

        // Send SIGTERM to the child process
        if let Some(ref child) = self.child {
            if let Some(pid) = child.id() {
                let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
            }
        }

        if intentional {
            // Wait for process to exit
            if let Some(ref mut child) = self.child {
                let _ = child.wait().await;
            }
            self.child = None;

            let mut state = self.state.write().await;
            state.state = VpnState::Disconnected;
            drop(state);
            self.update_tray().await;
            self.send_notification("OpenVPN", "Disconnected");
        }
    }

    /// Handle unexpected process exit (for auto-reconnect)
    async fn handle_process_exit(&mut self) {
        let (should_reconnect, server) = {
            let state = self.state.read().await;

            // Only auto-reconnect if:
            // 1. It wasn't an intentional disconnect
            // 2. Auto-reconnect is enabled
            // 3. We were in a connected or retrying state
            let should_reconnect = !state.intentional_disconnect
                && state.auto_reconnect
                && matches!(
                    state.state,
                    VpnState::Connected(_) | VpnState::Retrying { .. }
                );

            let server = state.state.server_name().map(|s| s.to_string());
            (should_reconnect, server)
        };

        self.child = None;

        if should_reconnect {
            if let Some(server) = server {
                self.attempt_reconnect(&server).await;
            }
        } else {
            let mut state = self.state.write().await;
            if !state.intentional_disconnect {
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
                    return;
                }
            }

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
            sleep(Duration::from_secs(RETRY_DELAY_SECS)).await;

            // Check again if user disconnected during wait
            {
                let state = self.state.read().await;
                if state.intentional_disconnect || !state.auto_reconnect {
                    return;
                }
            }

            // Try to connect
            match self.start_openvpn(server).await {
                Ok(child) => {
                    self.child = Some(child);
                    // Give OpenVPN some time to establish connection
                    sleep(Duration::from_secs(3)).await;

                    // Check if process is still running
                    if self.child.as_mut().map(|c| c.try_wait().ok().flatten().is_none()).unwrap_or(false) {
                        let mut state = self.state.write().await;
                        state.state = VpnState::Connected(server.to_string());
                        drop(state);
                        self.update_tray().await;
                        self.send_notification("OpenVPN", &format!("Reconnected to {}", server));
                        return;
                    } else {
                        // Process exited immediately, will retry
                        self.child = None;
                    }
                }
                Err(e) => {
                    eprintln!("Reconnection attempt {} failed: {}", attempt, e);
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
        let servers = scan_ovpn_files().await;
        let mut state = self.state.write().await;
        state.servers = servers;
        drop(state);
        self.update_tray().await;
    }

    /// Update the tray icon
    async fn update_tray(&self) {
        let handle = self.tray_handle.lock().await;
        if let Some(ref h) = *handle {
            h.update(|_| {});
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

/// Clean filename for display (e.g., us8399.nordvpn.com.udp.ovpn -> us8399)
fn clean_server_name(filename: &str) -> String {
    // Remove .ovpn extension first
    let name = filename.trim_end_matches(".ovpn");

    // Try to extract just the server identifier (before first dot)
    let re = Regex::new(r"^([a-zA-Z]{2}\d+)").unwrap();
    if let Some(caps) = re.captures(name) {
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
            })
            .unwrap_or_default()
    }
}

impl Tray for VpnTray {
    fn id(&self) -> String {
        "openvpn-tray".to_string()
    }

    fn title(&self) -> String {
        "OpenVPN Manager".to_string()
    }

    fn icon_name(&self) -> String {
        let state = self.get_state_blocking();
        match state.state {
            VpnState::Connected(_) => "network-vpn".to_string(),
            VpnState::Connecting(_) | VpnState::Retrying { .. } => "network-vpn-acquiring".to_string(),
            VpnState::Disconnected => "network-vpn-disconnected".to_string(),
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let state = self.get_state_blocking();
        let color = match state.state {
            VpnState::Connected(_) => (0, 255, 0, 255),      // Green
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
            VpnState::Connected(server) => format!("Connected to {}", clean_server_name(server)),
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
            VpnState::Connected(server) => format!("● Connected: {}", clean_server_name(server)),
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
    // Shared state between tray and actor
    let state = Arc::new(RwLock::new(SharedState::default()));

    // Command channel from tray to actor
    let (tx, rx) = mpsc::channel::<VpnCommand>(32);

    // Tray handle holder
    let tray_handle: Arc<Mutex<Option<ksni::Handle<VpnTray>>>> = Arc::new(Mutex::new(None));

    // Create the tray
    let tray = VpnTray::new(Arc::clone(&state), tx);

    // Create and spawn the tray service
    let service = TrayService::new(tray);
    let handle = service.handle();
    
    // Store the handle
    {
        let mut h = tray_handle.lock().await;
        *h = Some(handle);
    }

    // Spawn the tray service
    tokio::spawn(async move {
        let _ = service.run();
    });

    // Create and run the VPN actor
    let actor = VpnActor::new(Arc::clone(&state), rx, Arc::clone(&tray_handle));

    // Handle SIGTERM/SIGINT for graceful shutdown
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    tokio::select! {
        _ = actor.run() => {}
        _ = ctrl_c => {
            println!("\nReceived SIGINT, shutting down...");
        }
        _ = sigterm.recv() => {
            println!("\nReceived SIGTERM, shutting down...");
        }
    }

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
            VpnState::Connected("test.ovpn".to_string()).server_name(),
            Some("test.ovpn")
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
    fn test_shared_state_default() {
        let state = SharedState::default();
        assert_eq!(state.state, VpnState::Disconnected);
        assert!(state.auto_reconnect);
        assert!(state.servers.is_empty());
        assert!(!state.intentional_disconnect);
    }
}
