//! System tray service implementation
//!
//! Provides the ksni-based system tray interface for the VPN manager.

use ksni::{menu::CheckmarkItem, menu::StandardItem, MenuItem, Tray};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::autostart::Autostart;
use crate::state::VpnState;
use crate::tray::icons::{get_status_icon, IconType};

/// Commands that can be sent to the VPN supervisor
#[derive(Debug)]
pub enum VpnCommand {
    /// Connect to a specific server
    Connect(String),
    /// Disconnect from the current server
    Disconnect,
    /// Toggle auto-reconnect feature
    ToggleAutoReconnect,
    /// Toggle kill switch (blocks non-VPN traffic)
    ToggleKillSwitch,
    /// Toggle autostart on login
    ToggleAutostart,
    /// Toggle debug logging to file
    ToggleDebugLogging,
    /// Open the log file in default viewer
    OpenLogFile,
    /// Refresh the list of available VPN connections
    RefreshConnections,
    /// Restart the application
    Restart,
    /// Request graceful shutdown
    Quit,
}

/// Shared state between the tray and the VPN supervisor
#[derive(Clone)]
pub struct SharedState {
    /// Current VPN state
    pub state: VpnState,
    /// Whether auto-reconnect is enabled
    pub auto_reconnect: bool,
    /// Whether kill switch is enabled
    pub kill_switch: bool,
    /// Whether debug logging is enabled
    pub debug_logging: bool,
    /// List of available VPN connections from NetworkManager
    pub connections: Vec<String>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            state: VpnState::Disconnected,
            auto_reconnect: true,
            kill_switch: false,
            debug_logging: false,
            connections: Vec::new(),
        }
    }
}

/// Extract a short display name from a VPN connection name
/// e.g., "ie211-dublin" -> "ie211" or "us8399-ashburn" -> "us8399"
#[inline]
pub fn extract_short_name(full_name: &str) -> &str {
    // Take everything before the first hyphen, or the whole name if no hyphen
    full_name.split('-').next().unwrap_or(full_name)
}

/// System tray interface
pub struct VpnTray {
    /// Cached state for synchronous tray methods
    pub cached_state: Arc<std::sync::RwLock<SharedState>>,
    /// Command sender to the supervisor
    tx: mpsc::Sender<VpnCommand>,
}

impl VpnTray {
    /// Create a new tray instance
    pub fn new(tx: mpsc::Sender<VpnCommand>) -> Self {
        // Create initial cached state
        let cached_state = Arc::new(std::sync::RwLock::new(SharedState::default()));

        Self { cached_state, tx }
    }
}

impl Tray for VpnTray {
    // Enable left-click to open menu (in addition to right-click)
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "shroud".to_string()
    }

    fn icon_name(&self) -> String {
        // Return empty string to force use of icon_pixmap() colored icons
        String::new()
    }

    fn title(&self) -> String {
        let state = self.cached_state.read().unwrap_or_else(|poisoned| {
            log::warn!("Tray cached_state lock poisoned, recovering");
            poisoned.into_inner()
        });
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
            VpnState::Degraded { server } => format!("⚠️ {}", extract_short_name(server)),
            VpnState::Failed { server, .. } => format!("❌ {}", extract_short_name(server)),
            VpnState::Disconnected => "⭕ VPN".to_string(),
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let state = self.cached_state.read().unwrap_or_else(|poisoned| {
            log::warn!("Tray cached_state lock poisoned, recovering");
            poisoned.into_inner()
        });
        match state.state {
            VpnState::Connected { .. } => get_status_icon(IconType::Connected),
            VpnState::Connecting { .. } | VpnState::Reconnecting { .. } => {
                get_status_icon(IconType::Connecting)
            }
            VpnState::Degraded { .. } => get_status_icon(IconType::Degraded),
            VpnState::Failed { .. } => get_status_icon(IconType::Failed),
            VpnState::Disconnected => get_status_icon(IconType::Disconnected),
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let state = self.cached_state.read().unwrap_or_else(|poisoned| {
            log::warn!("Tray cached_state lock poisoned, recovering");
            poisoned.into_inner()
        });
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
            VpnState::Degraded { server } => (
                format!("⚠️ Degraded: {}", server),
                "Connection may be unstable".to_string(),
            ),
            VpnState::Failed { server, reason } => (format!("Failed: {}", server), reason.clone()),
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
        let state = self
            .cached_state
            .read()
            .unwrap_or_else(|poisoned| {
                log::warn!("Tray cached_state lock poisoned, recovering");
                poisoned.into_inner()
            })
            .clone();
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
            VpnState::Degraded { server } => format!("⚠️ Degraded: {}", server),
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
            let is_busy = state.state.is_busy();

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
                    enabled: !is_current && !is_busy,
                    activate: Box::new(move |tray: &mut Self| {
                        let conn = conn_clone.clone();
                        // Use try_send (non-blocking) - ksni uses an async runtime internally
                        let _ = tray.tx.try_send(VpnCommand::Connect(conn));
                    }),
                    ..Default::default()
                }));
            }
        }

        items.push(MenuItem::Separator);

        // Disconnect button - only enabled when connected
        let can_disconnect = matches!(
            state.state,
            VpnState::Connected { .. } | VpnState::Degraded { .. }
        );
        items.push(MenuItem::Standard(StandardItem {
            label: "Disconnect".to_string(),
            enabled: can_disconnect,
            activate: Box::new(|tray: &mut Self| {
                // Use try_send (non-blocking) - ksni uses an async runtime internally
                let _ = tray.tx.try_send(VpnCommand::Disconnect);
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
                // Use try_send (non-blocking) - ksni uses an async runtime internally
                let _ = tray.tx.try_send(VpnCommand::ToggleAutoReconnect);
            }),
            ..Default::default()
        }));

        // Kill switch toggle with checkbox
        items.push(MenuItem::Checkmark(CheckmarkItem {
            label: "Kill Switch".to_string(),
            enabled: true,
            checked: state.kill_switch,
            activate: Box::new(|tray: &mut Self| {
                // Use try_send (non-blocking) - ksni uses an async runtime internally
                let _ = tray.tx.try_send(VpnCommand::ToggleKillSwitch);
            }),
            ..Default::default()
        }));

        // Autostart toggle with checkbox
        items.push(MenuItem::Checkmark(CheckmarkItem {
            label: "Start on Login".to_string(),
            enabled: true,
            checked: Autostart::is_enabled(),
            activate: Box::new(|tray: &mut Self| {
                // Use try_send (non-blocking) - ksni uses an async runtime internally
                let _ = tray.tx.try_send(VpnCommand::ToggleAutostart);
            }),
            ..Default::default()
        }));

        // Refresh connections
        items.push(MenuItem::Standard(StandardItem {
            label: "Refresh Connections".to_string(),
            enabled: true,
            activate: Box::new(|tray: &mut Self| {
                // Use try_send (non-blocking) - ksni uses an async runtime internally
                let _ = tray.tx.try_send(VpnCommand::RefreshConnections);
            }),
            ..Default::default()
        }));

        items.push(MenuItem::Separator);

        // Debug logging toggle
        items.push(MenuItem::Checkmark(CheckmarkItem {
            label: "Debug Logging".to_string(),
            enabled: true,
            checked: state.debug_logging,
            activate: Box::new(|tray: &mut Self| {
                // Use try_send (non-blocking) - ksni uses an async runtime internally
                let _ = tray.tx.try_send(VpnCommand::ToggleDebugLogging);
            }),
            ..Default::default()
        }));

        // Open log file
        items.push(MenuItem::Standard(StandardItem {
            label: "Open Log File".to_string(),
            enabled: state.debug_logging,
            activate: Box::new(|tray: &mut Self| {
                // Use try_send (non-blocking) - ksni uses an async runtime internally
                let _ = tray.tx.try_send(VpnCommand::OpenLogFile);
            }),
            ..Default::default()
        }));

        items.push(MenuItem::Separator);

        // Restart application
        items.push(MenuItem::Standard(StandardItem {
            label: "Restart Daemon".to_string(),
            icon_name: "view-refresh".to_string(),
            enabled: true,
            activate: Box::new(|tray: &mut Self| {
                // Use try_send (non-blocking) - ksni uses an async runtime internally
                let _ = tray.tx.try_send(VpnCommand::Restart);
            }),
            ..Default::default()
        }));

        // Quit
        items.push(MenuItem::Standard(StandardItem {
            label: "Quit".to_string(),
            icon_name: "application-exit".to_string(),
            enabled: true,
            activate: Box::new(|tray: &mut Self| {
                let _ = tray.tx.try_send(VpnCommand::Quit);
            }),
            ..Default::default()
        }));

        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- extract_short_name -----

    #[test]
    fn test_extract_short_name() {
        assert_eq!(extract_short_name("ie211-dublin"), "ie211");
        assert_eq!(extract_short_name("us8399-ashburn"), "us8399");
        assert_eq!(extract_short_name("de123-berlin-west"), "de123");
        assert_eq!(extract_short_name("myvpn"), "myvpn");
        assert_eq!(extract_short_name(""), "");
    }

    #[test]
    fn test_extract_short_name_no_hyphen() {
        assert_eq!(extract_short_name("singlevpn"), "singlevpn");
    }

    #[test]
    fn test_extract_short_name_leading_hyphen() {
        assert_eq!(extract_short_name("-starts-with-hyphen"), "");
    }

    #[test]
    fn test_extract_short_name_trailing_hyphen() {
        assert_eq!(extract_short_name("server-"), "server");
    }

    #[test]
    fn test_extract_short_name_multiple_hyphens() {
        assert_eq!(extract_short_name("uk-lon-001-fast"), "uk");
    }

    #[test]
    fn test_extract_short_name_numbers_only() {
        assert_eq!(extract_short_name("12345-server"), "12345");
    }

    #[test]
    fn test_extract_short_name_unicode() {
        assert_eq!(extract_short_name("münchen-vpn"), "münchen");
    }

    // ----- SharedState -----

    #[test]
    fn test_shared_state_default() {
        let state = SharedState::default();
        assert_eq!(state.state, VpnState::Disconnected);
        assert!(state.auto_reconnect);
        assert!(!state.kill_switch);
        assert!(state.connections.is_empty());
    }

    #[test]
    fn test_shared_state_clone() {
        let state = SharedState {
            state: VpnState::Connected {
                server: "vpn1".into(),
            },
            kill_switch: true,
            connections: vec!["vpn1".into(), "vpn2".into()],
            ..Default::default()
        };

        let cloned = state.clone();
        assert_eq!(cloned.state, state.state);
        assert!(cloned.kill_switch);
        assert_eq!(cloned.connections.len(), 2);
    }

    #[test]
    fn test_shared_state_modify_connections() {
        let mut state = SharedState::default();
        state.connections.push("vpn-a".into());
        state.connections.push("vpn-b".into());
        state.connections.push("vpn-c".into());

        assert_eq!(state.connections.len(), 3);
        assert_eq!(state.connections[0], "vpn-a");
    }

    #[test]
    fn test_shared_state_modify_vpn_state() {
        let mut state = SharedState {
            state: VpnState::Connecting {
                server: "vpn1".into(),
            },
            ..Default::default()
        };
        assert!(matches!(state.state, VpnState::Connecting { .. }));

        state.state = VpnState::Connected {
            server: "vpn1".into(),
        };
        assert!(matches!(state.state, VpnState::Connected { .. }));

        state.state = VpnState::Degraded {
            server: "vpn1".into(),
        };
        assert!(matches!(state.state, VpnState::Degraded { .. }));

        state.state = VpnState::Reconnecting {
            server: "vpn1".into(),
            attempt: 2,
            max_attempts: 10,
        };
        assert!(matches!(state.state, VpnState::Reconnecting { .. }));

        state.state = VpnState::Failed {
            server: "vpn1".into(),
            reason: "timeout".into(),
        };
        assert!(matches!(state.state, VpnState::Failed { .. }));
    }

    #[test]
    fn test_shared_state_toggle_flags() {
        let mut state = SharedState::default();

        // Toggle auto_reconnect
        state.auto_reconnect = !state.auto_reconnect;
        assert!(!state.auto_reconnect);
        state.auto_reconnect = !state.auto_reconnect;
        assert!(state.auto_reconnect);

        // Toggle kill_switch
        state.kill_switch = !state.kill_switch;
        assert!(state.kill_switch);
        state.kill_switch = !state.kill_switch;
        assert!(!state.kill_switch);

        // Toggle debug_logging
        state.debug_logging = !state.debug_logging;
        assert!(state.debug_logging);
    }

    // ----- VpnCommand -----

    #[test]
    fn test_vpn_command_connect() {
        let cmd = VpnCommand::Connect("test-vpn".to_string());
        match cmd {
            VpnCommand::Connect(name) => assert_eq!(name, "test-vpn"),
            _ => panic!("Expected Connect"),
        }
    }

    #[test]
    fn test_vpn_command_disconnect() {
        let cmd = VpnCommand::Disconnect;
        assert!(matches!(cmd, VpnCommand::Disconnect));
    }

    #[test]
    fn test_vpn_command_toggle_auto_reconnect() {
        let cmd = VpnCommand::ToggleAutoReconnect;
        assert!(matches!(cmd, VpnCommand::ToggleAutoReconnect));
    }

    #[test]
    fn test_vpn_command_toggle_killswitch() {
        let cmd = VpnCommand::ToggleKillSwitch;
        assert!(matches!(cmd, VpnCommand::ToggleKillSwitch));
    }

    #[test]
    fn test_vpn_command_toggle_autostart() {
        let cmd = VpnCommand::ToggleAutostart;
        assert!(matches!(cmd, VpnCommand::ToggleAutostart));
    }

    #[test]
    fn test_vpn_command_debug() {
        let cmd = VpnCommand::Connect("vpn".into());
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("Connect"));
        assert!(debug.contains("vpn"));
    }

    #[test]
    fn test_vpn_command_all_variants() {
        // Ensure all variants can be constructed
        let commands: Vec<VpnCommand> = vec![
            VpnCommand::Connect("vpn".into()),
            VpnCommand::Disconnect,
            VpnCommand::ToggleAutoReconnect,
            VpnCommand::ToggleKillSwitch,
            VpnCommand::ToggleAutostart,
            VpnCommand::ToggleDebugLogging,
            VpnCommand::OpenLogFile,
            VpnCommand::RefreshConnections,
            VpnCommand::Restart,
        ];
        assert_eq!(commands.len(), 9);
    }
}
