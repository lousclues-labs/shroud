//! Tray state management — pure logic, no GTK/ksni dependencies.
//!
//! Extracts testable icon selection, tooltip generation, and menu
//! building logic from the ksni Tray implementation.

use crate::state::VpnState;

/// Tray icon state derived from VpnState
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIcon {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Degraded,
    Failed,
}

impl TrayIcon {
    /// Get the symbolic icon name for the current state.
    pub fn icon_name(&self) -> &'static str {
        match self {
            TrayIcon::Disconnected => "network-vpn-disconnected-symbolic",
            TrayIcon::Connecting => "network-vpn-acquiring-symbolic",
            TrayIcon::Connected => "network-vpn-symbolic",
            TrayIcon::Reconnecting => "network-vpn-acquiring-symbolic",
            TrayIcon::Degraded => "network-vpn-error-symbolic",
            TrayIcon::Failed => "network-error-symbolic",
        }
    }

    /// Get a fallback icon name when the symbolic icon is unavailable.
    pub fn fallback_icon_name(&self) -> &'static str {
        match self {
            TrayIcon::Connected => "network-vpn",
            _ => "network-offline",
        }
    }

    /// Generate tooltip text for the current state.
    pub fn tooltip(&self, server: Option<&str>) -> String {
        match (self, server) {
            (TrayIcon::Disconnected, _) => "Shroud: Disconnected".into(),
            (TrayIcon::Connecting, Some(s)) => format!("Shroud: Connecting to {}", s),
            (TrayIcon::Connecting, None) => "Shroud: Connecting...".into(),
            (TrayIcon::Connected, Some(s)) => format!("Shroud: Connected to {}", s),
            (TrayIcon::Connected, None) => "Shroud: Connected".into(),
            (TrayIcon::Reconnecting, Some(s)) => format!("Shroud: Reconnecting to {}", s),
            (TrayIcon::Reconnecting, None) => "Shroud: Reconnecting...".into(),
            (TrayIcon::Degraded, Some(s)) => format!("Shroud: Degraded connection to {}", s),
            (TrayIcon::Degraded, None) => "Shroud: Connection degraded".into(),
            (TrayIcon::Failed, Some(s)) => format!("Shroud: Failed ({})", s),
            (TrayIcon::Failed, None) => "Shroud: Error".into(),
        }
    }
}

impl From<&VpnState> for TrayIcon {
    fn from(state: &VpnState) -> Self {
        match state {
            VpnState::Disconnected => TrayIcon::Disconnected,
            VpnState::Connecting { .. } => TrayIcon::Connecting,
            VpnState::Connected { .. } => TrayIcon::Connected,
            VpnState::Reconnecting { .. } => TrayIcon::Reconnecting,
            VpnState::Degraded { .. } => TrayIcon::Degraded,
            VpnState::Failed { .. } => TrayIcon::Failed,
        }
    }
}

/// A menu item representation (pure data, no UI).
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub submenu: Option<Vec<MenuItem>>,
}

impl MenuItem {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled: true,
            checked: None,
            submenu: None,
        }
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    pub fn with_submenu(mut self, items: Vec<MenuItem>) -> Self {
        self.submenu = Some(items);
        self
    }

    pub fn separator() -> Self {
        Self {
            id: "separator".into(),
            label: "---".into(),
            enabled: false,
            checked: None,
            submenu: None,
        }
    }
}

/// Build the tray menu based on current state (pure function).
pub fn build_menu(
    state: &VpnState,
    vpn_list: &[String],
    kill_switch_enabled: bool,
    auto_reconnect: bool,
) -> Vec<MenuItem> {
    let mut items = Vec::new();

    // Status item (disabled, just for display)
    let status_text = match state {
        VpnState::Disconnected => "Status: Disconnected".into(),
        VpnState::Connecting { server } => format!("Status: Connecting to {}", server),
        VpnState::Connected { server } => format!("Status: Connected to {}", server),
        VpnState::Reconnecting {
            server, attempt, ..
        } => format!("Status: Reconnecting to {} (attempt {})", server, attempt),
        VpnState::Degraded { server } => format!("Status: Degraded ({})", server),
        VpnState::Failed { server, reason } => {
            format!("Status: Failed ({} - {})", server, reason)
        }
    };
    items.push(MenuItem::new("status", &status_text).disabled());
    items.push(MenuItem::separator());

    // VPN submenu
    let vpn_items: Vec<MenuItem> = vpn_list
        .iter()
        .map(|vpn| {
            let is_active = state.server_name() == Some(vpn.as_str());
            MenuItem::new(&format!("vpn:{}", vpn), vpn).checked(is_active)
        })
        .collect();

    if vpn_items.is_empty() {
        items.push(MenuItem::new("no-vpns", "No VPN connections").disabled());
    } else {
        items.push(MenuItem::new("vpns", "Connect").with_submenu(vpn_items));
    }

    // Disconnect (only if not disconnected)
    let can_disconnect = !matches!(state, VpnState::Disconnected);
    items.push(if can_disconnect {
        MenuItem::new("disconnect", "Disconnect")
    } else {
        MenuItem::new("disconnect", "Disconnect").disabled()
    });

    items.push(MenuItem::separator());

    // Kill switch toggle
    items.push(MenuItem::new("killswitch", "Kill Switch").checked(kill_switch_enabled));

    // Auto-reconnect toggle
    items.push(MenuItem::new("auto-reconnect", "Auto-reconnect").checked(auto_reconnect));

    items.push(MenuItem::separator());
    items.push(MenuItem::new("quit", "Quit Shroud"));

    items
}

/// Actions that can result from menu item activation.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    Connect(String),
    Disconnect,
    ToggleKillSwitch,
    ToggleAutoReconnect,
    Quit,
    None,
}

/// Map a menu item id to a MenuAction.
pub fn handle_menu_action(action_id: &str) -> MenuAction {
    if let Some(vpn_name) = action_id.strip_prefix("vpn:") {
        return MenuAction::Connect(vpn_name.to_string());
    }

    match action_id {
        "disconnect" => MenuAction::Disconnect,
        "killswitch" => MenuAction::ToggleKillSwitch,
        "auto-reconnect" => MenuAction::ToggleAutoReconnect,
        "quit" => MenuAction::Quit,
        _ => MenuAction::None,
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod tray_icon {
        use super::*;

        #[test]
        fn test_icon_names_unique() {
            let icons = [
                TrayIcon::Disconnected,
                TrayIcon::Connected,
                TrayIcon::Degraded,
                TrayIcon::Failed,
            ];
            let names: Vec<_> = icons.iter().map(|i| i.icon_name()).collect();
            // Connected and Connecting share an icon; others must differ
            assert_ne!(names[0], names[1]);
            assert_ne!(names[1], names[2]);
        }

        #[test]
        fn test_icon_names_return_symbolic() {
            assert!(TrayIcon::Disconnected.icon_name().ends_with("-symbolic"));
            assert!(TrayIcon::Connected.icon_name().ends_with("-symbolic"));
            assert!(TrayIcon::Connecting.icon_name().ends_with("-symbolic"));
            assert!(TrayIcon::Degraded.icon_name().ends_with("-symbolic"));
            assert!(TrayIcon::Failed.icon_name().ends_with("-symbolic"));
            assert!(TrayIcon::Reconnecting.icon_name().ends_with("-symbolic"));
        }

        #[test]
        fn test_fallback_connected() {
            assert_eq!(TrayIcon::Connected.fallback_icon_name(), "network-vpn");
        }

        #[test]
        fn test_fallback_others() {
            assert_eq!(
                TrayIcon::Disconnected.fallback_icon_name(),
                "network-offline"
            );
            assert_eq!(TrayIcon::Failed.fallback_icon_name(), "network-offline");
            assert_eq!(TrayIcon::Degraded.fallback_icon_name(), "network-offline");
        }

        #[test]
        fn test_tooltip_disconnected() {
            let t = TrayIcon::Disconnected.tooltip(None);
            assert!(t.contains("Disconnected"));
        }

        #[test]
        fn test_tooltip_connected_with_server() {
            let t = TrayIcon::Connected.tooltip(Some("my-vpn"));
            assert!(t.contains("Connected"));
            assert!(t.contains("my-vpn"));
        }

        #[test]
        fn test_tooltip_connected_without_server() {
            let t = TrayIcon::Connected.tooltip(None);
            assert!(t.contains("Connected"));
        }

        #[test]
        fn test_tooltip_connecting() {
            let t = TrayIcon::Connecting.tooltip(Some("vpn1"));
            assert!(t.contains("Connecting"));
            assert!(t.contains("vpn1"));
        }

        #[test]
        fn test_tooltip_reconnecting() {
            let t = TrayIcon::Reconnecting.tooltip(Some("vpn1"));
            assert!(t.contains("Reconnecting"));
        }

        #[test]
        fn test_tooltip_degraded() {
            let t = TrayIcon::Degraded.tooltip(Some("vpn1"));
            assert!(t.contains("Degraded"));
        }

        #[test]
        fn test_tooltip_failed_with_server() {
            let t = TrayIcon::Failed.tooltip(Some("vpn1"));
            assert!(t.contains("Failed"));
        }

        #[test]
        fn test_tooltip_failed_without_server() {
            let t = TrayIcon::Failed.tooltip(None);
            assert!(t.contains("Error"));
        }

        #[test]
        fn test_from_vpn_state_disconnected() {
            assert_eq!(
                TrayIcon::from(&VpnState::Disconnected),
                TrayIcon::Disconnected
            );
        }

        #[test]
        fn test_from_vpn_state_connected() {
            let s = VpnState::Connected { server: "v".into() };
            assert_eq!(TrayIcon::from(&s), TrayIcon::Connected);
        }

        #[test]
        fn test_from_vpn_state_connecting() {
            let s = VpnState::Connecting { server: "v".into() };
            assert_eq!(TrayIcon::from(&s), TrayIcon::Connecting);
        }

        #[test]
        fn test_from_vpn_state_reconnecting() {
            let s = VpnState::Reconnecting {
                server: "v".into(),
                attempt: 1,
                max_attempts: 10,
            };
            assert_eq!(TrayIcon::from(&s), TrayIcon::Reconnecting);
        }

        #[test]
        fn test_from_vpn_state_degraded() {
            let s = VpnState::Degraded { server: "v".into() };
            assert_eq!(TrayIcon::from(&s), TrayIcon::Degraded);
        }

        #[test]
        fn test_from_vpn_state_failed() {
            let s = VpnState::Failed {
                server: "v".into(),
                reason: "err".into(),
            };
            assert_eq!(TrayIcon::from(&s), TrayIcon::Failed);
        }
    }

    mod menu_item {
        use super::*;

        #[test]
        fn test_new() {
            let item = MenuItem::new("test", "Test Label");
            assert_eq!(item.id, "test");
            assert_eq!(item.label, "Test Label");
            assert!(item.enabled);
            assert!(item.checked.is_none());
            assert!(item.submenu.is_none());
        }

        #[test]
        fn test_disabled() {
            let item = MenuItem::new("x", "X").disabled();
            assert!(!item.enabled);
        }

        #[test]
        fn test_checked() {
            let item = MenuItem::new("x", "X").checked(true);
            assert_eq!(item.checked, Some(true));
        }

        #[test]
        fn test_unchecked() {
            let item = MenuItem::new("x", "X").checked(false);
            assert_eq!(item.checked, Some(false));
        }

        #[test]
        fn test_with_submenu() {
            let sub = vec![MenuItem::new("s1", "Sub 1"), MenuItem::new("s2", "Sub 2")];
            let item = MenuItem::new("p", "Parent").with_submenu(sub);
            assert!(item.submenu.is_some());
            assert_eq!(item.submenu.unwrap().len(), 2);
        }

        #[test]
        fn test_separator() {
            let sep = MenuItem::separator();
            assert_eq!(sep.id, "separator");
            assert!(!sep.enabled);
            assert!(sep.checked.is_none());
        }
    }

    mod build_menu_tests {
        use super::*;

        #[test]
        fn test_menu_disconnected_has_status() {
            let menu = build_menu(&VpnState::Disconnected, &["vpn1".into()], false, true);
            assert!(menu.iter().any(|i| i.id == "status"));
        }

        #[test]
        fn test_menu_disconnected_disconnect_disabled() {
            let menu = build_menu(&VpnState::Disconnected, &["vpn1".into()], false, true);
            let dc = menu.iter().find(|i| i.id == "disconnect").unwrap();
            assert!(!dc.enabled);
        }

        #[test]
        fn test_menu_connected_disconnect_enabled() {
            let state = VpnState::Connected {
                server: "vpn1".into(),
            };
            let menu = build_menu(&state, &["vpn1".into()], false, false);
            let dc = menu.iter().find(|i| i.id == "disconnect").unwrap();
            assert!(dc.enabled);
        }

        #[test]
        fn test_menu_no_vpns() {
            let menu = build_menu(&VpnState::Disconnected, &[], false, false);
            assert!(menu.iter().any(|i| i.id == "no-vpns" && !i.enabled));
        }

        #[test]
        fn test_menu_has_quit() {
            let menu = build_menu(&VpnState::Disconnected, &[], false, false);
            assert!(menu.iter().any(|i| i.id == "quit"));
        }

        #[test]
        fn test_menu_ks_checked() {
            let menu = build_menu(&VpnState::Disconnected, &[], true, false);
            let ks = menu.iter().find(|i| i.id == "killswitch").unwrap();
            assert_eq!(ks.checked, Some(true));
        }

        #[test]
        fn test_menu_auto_reconnect_checked() {
            let menu = build_menu(&VpnState::Disconnected, &[], false, true);
            let ar = menu.iter().find(|i| i.id == "auto-reconnect").unwrap();
            assert_eq!(ar.checked, Some(true));
        }

        #[test]
        fn test_vpn_submenu_marks_active() {
            let state = VpnState::Connected {
                server: "vpn2".into(),
            };
            let menu = build_menu(
                &state,
                &["vpn1".into(), "vpn2".into(), "vpn3".into()],
                false,
                false,
            );
            let vpns_item = menu.iter().find(|i| i.id == "vpns").unwrap();
            let submenu = vpns_item.submenu.as_ref().unwrap();

            let vpn2 = submenu.iter().find(|i| i.id == "vpn:vpn2").unwrap();
            assert_eq!(vpn2.checked, Some(true));

            let vpn1 = submenu.iter().find(|i| i.id == "vpn:vpn1").unwrap();
            assert_eq!(vpn1.checked, Some(false));
        }

        #[test]
        fn test_menu_reconnecting_disconnect_enabled() {
            let state = VpnState::Reconnecting {
                server: "vpn1".into(),
                attempt: 2,
                max_attempts: 10,
            };
            let menu = build_menu(&state, &["vpn1".into()], false, false);
            let dc = menu.iter().find(|i| i.id == "disconnect").unwrap();
            assert!(dc.enabled);
        }

        #[test]
        fn test_menu_failed_disconnect_enabled() {
            let state = VpnState::Failed {
                server: "vpn1".into(),
                reason: "err".into(),
            };
            let menu = build_menu(&state, &["vpn1".into()], false, false);
            let dc = menu.iter().find(|i| i.id == "disconnect").unwrap();
            assert!(dc.enabled);
        }

        #[test]
        fn test_status_text_connected() {
            let state = VpnState::Connected {
                server: "my-vpn".into(),
            };
            let menu = build_menu(&state, &[], false, false);
            let status = menu.iter().find(|i| i.id == "status").unwrap();
            assert!(status.label.contains("Connected"));
            assert!(status.label.contains("my-vpn"));
        }
    }

    mod handle_menu_action_tests {
        use super::*;

        #[test]
        fn test_vpn_connect() {
            assert_eq!(
                handle_menu_action("vpn:my-vpn"),
                MenuAction::Connect("my-vpn".into())
            );
        }

        #[test]
        fn test_vpn_connect_with_hyphen() {
            assert_eq!(
                handle_menu_action("vpn:us-east-1"),
                MenuAction::Connect("us-east-1".into())
            );
        }

        #[test]
        fn test_disconnect() {
            assert_eq!(handle_menu_action("disconnect"), MenuAction::Disconnect);
        }

        #[test]
        fn test_killswitch() {
            assert_eq!(
                handle_menu_action("killswitch"),
                MenuAction::ToggleKillSwitch
            );
        }

        #[test]
        fn test_auto_reconnect() {
            assert_eq!(
                handle_menu_action("auto-reconnect"),
                MenuAction::ToggleAutoReconnect
            );
        }

        #[test]
        fn test_quit() {
            assert_eq!(handle_menu_action("quit"), MenuAction::Quit);
        }

        #[test]
        fn test_unknown() {
            assert_eq!(handle_menu_action("foobar"), MenuAction::None);
        }

        #[test]
        fn test_separator() {
            assert_eq!(handle_menu_action("separator"), MenuAction::None);
        }
    }
}
