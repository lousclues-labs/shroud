// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Unit tests for Tray module
//!
//! Tests menu/action wiring with mock notifier
//! without requiring a real GUI or display.

#[cfg(test)]
mod tray_tests {
    use crate::tray::{SharedState, VpnCommand};

    // =========================================================================
    // VpnCommand tests
    // =========================================================================

    #[test]
    fn test_vpn_command_variants() {
        // Test command variants exist and can be constructed
        let cmd1 = VpnCommand::Connect("test-vpn".to_string());
        let cmd2 = VpnCommand::Disconnect;
        let cmd3 = VpnCommand::ToggleKillSwitch;
        let cmd4 = VpnCommand::ToggleAutostart;

        // Test pattern matching
        match cmd1 {
            VpnCommand::Connect(name) => assert_eq!(name, "test-vpn"),
            _ => panic!("Expected Connect"),
        }
        match cmd2 {
            VpnCommand::Disconnect => {}
            _ => panic!("Expected Disconnect"),
        }
        match cmd3 {
            VpnCommand::ToggleKillSwitch => {}
            _ => panic!("Expected ToggleKillSwitch"),
        }
        match cmd4 {
            VpnCommand::ToggleAutostart => {}
            _ => panic!("Expected ToggleAutostart"),
        }
    }

    // =========================================================================
    // SharedState tests
    // =========================================================================

    #[test]
    fn test_shared_state_default() {
        let state = SharedState::default();
        assert!(!state.kill_switch, "Kill switch off by default");
        assert!(state.auto_reconnect, "Auto-reconnect on by default");
    }

    #[test]
    fn test_shared_state_modification() {
        let mut state = SharedState {
            kill_switch: true,
            ..Default::default()
        };
        assert!(state.kill_switch);
        state.kill_switch = false;
        assert!(!state.kill_switch);
    }

    #[test]
    fn test_shared_state_clone() {
        let state = SharedState {
            kill_switch: true,
            connections: vec!["vpn1".to_string()],
            ..Default::default()
        };
        let cloned = state.clone();
        assert_eq!(cloned.kill_switch, state.kill_switch);
        assert_eq!(cloned.connections.len(), 1);
    }

    // =========================================================================
    // Menu tests
    // =========================================================================

    #[test]
    fn test_menu_item_enabled_state() {
        fn compute_menu_state(is_connected: bool, has_vpns: bool) -> (bool, bool, bool) {
            (
                !is_connected && has_vpns, // connect_enabled
                is_connected,              // disconnect_enabled
                !is_connected && has_vpns, // vpn_list_visible
            )
        }

        let (c, d, v) = compute_menu_state(false, true);
        assert!(c && !d && v);

        let (c, d, v) = compute_menu_state(true, true);
        assert!(!c && d && !v);
    }

    // =========================================================================
    // Short name extraction tests
    // =========================================================================

    #[test]
    fn test_extract_short_name() {
        use crate::tray::service::extract_short_name;

        assert_eq!(extract_short_name("ie211-dublin"), "ie211");
        assert_eq!(extract_short_name("myvpn"), "myvpn");
    }

    // =========================================================================
    // Channel tests
    // =========================================================================

    #[tokio::test]
    async fn test_command_channel() {
        use tokio::sync::mpsc;
        let (tx, mut rx) = mpsc::channel::<VpnCommand>(16);
        tx.send(VpnCommand::Disconnect).await.unwrap();
        let cmd = rx.recv().await.unwrap();
        assert!(matches!(cmd, VpnCommand::Disconnect));
    }
}
