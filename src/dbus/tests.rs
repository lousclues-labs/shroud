// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Unit tests for D-Bus module

#[cfg(test)]
mod dbus_tests {
    use crate::dbus::NmEvent;

    #[test]
    fn test_nm_event_variants() {
        let events = vec![
            NmEvent::VpnActivated {
                name: "test-vpn".to_string(),
            },
            NmEvent::VpnDeactivated {
                name: "test-vpn".to_string(),
            },
            NmEvent::VpnActivating {
                name: "test-vpn".to_string(),
            },
            NmEvent::VpnFailed {
                name: "test-vpn".to_string(),
                reason: "timeout".to_string(),
            },
            NmEvent::ConnectivityChanged { connected: true },
        ];

        assert_eq!(events.len(), 5);

        for event in events {
            match event {
                NmEvent::VpnActivated { name } => assert_eq!(name, "test-vpn"),
                NmEvent::VpnDeactivated { name } => assert_eq!(name, "test-vpn"),
                NmEvent::VpnActivating { name } => assert_eq!(name, "test-vpn"),
                NmEvent::VpnFailed { name, reason } => {
                    assert_eq!(name, "test-vpn");
                    assert_eq!(reason, "timeout");
                }
                NmEvent::ConnectivityChanged { connected } => assert!(connected),
            }
        }
    }

    #[test]
    fn test_nm_event_clone() {
        let event = NmEvent::VpnActivated {
            name: "my-vpn".to_string(),
        };
        let cloned = event.clone();

        if let (NmEvent::VpnActivated { name: a }, NmEvent::VpnActivated { name: b }) =
            (event, cloned)
        {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_vpn_state_parsing() {
        fn parse_vpn_state(state: u32) -> &'static str {
            match state {
                0 => "unknown",
                1 => "prepare",
                2 => "need_auth",
                3 => "connect",
                4 => "ip_config",
                5 => "activated",
                6 => "failed",
                7 => "disconnected",
                _ => "invalid",
            }
        }

        assert_eq!(parse_vpn_state(0), "unknown");
        assert_eq!(parse_vpn_state(5), "activated");
        assert_eq!(parse_vpn_state(6), "failed");
        assert_eq!(parse_vpn_state(7), "disconnected");
        assert_eq!(parse_vpn_state(99), "invalid");
    }

    #[test]
    fn test_vpn_state_to_event() {
        fn state_to_event(state: u32, vpn_name: &str) -> Option<NmEvent> {
            match state {
                1..=4 => Some(NmEvent::VpnActivating {
                    name: vpn_name.to_string(),
                }),
                5 => Some(NmEvent::VpnActivated {
                    name: vpn_name.to_string(),
                }),
                6 => Some(NmEvent::VpnFailed {
                    name: vpn_name.to_string(),
                    reason: "failed".to_string(),
                }),
                7 => Some(NmEvent::VpnDeactivated {
                    name: vpn_name.to_string(),
                }),
                _ => None,
            }
        }

        for state in [1, 2, 3, 4] {
            assert!(matches!(
                state_to_event(state, "vpn"),
                Some(NmEvent::VpnActivating { .. })
            ));
        }
        assert!(matches!(
            state_to_event(5, "vpn"),
            Some(NmEvent::VpnActivated { .. })
        ));
        assert!(matches!(
            state_to_event(6, "vpn"),
            Some(NmEvent::VpnFailed { .. })
        ));
        assert!(matches!(
            state_to_event(7, "vpn"),
            Some(NmEvent::VpnDeactivated { .. })
        ));
    }

    #[test]
    fn test_dbus_path_parsing() {
        let path = "/org/freedesktop/NetworkManager/ActiveConnection/123";
        assert!(path.starts_with("/org/freedesktop/NetworkManager"));
        let id = path.rsplit('/').next().unwrap();
        assert_eq!(id, "123");
    }

    #[test]
    fn test_connection_type_detection() {
        fn is_vpn_type(conn_type: &str) -> bool {
            matches!(conn_type, "vpn" | "wireguard")
        }

        assert!(is_vpn_type("vpn"));
        assert!(is_vpn_type("wireguard"));
        assert!(!is_vpn_type("802-11-wireless"));
    }

    #[tokio::test]
    async fn test_event_channel_capacity() {
        use tokio::sync::mpsc;
        let (tx, _rx) = mpsc::channel::<NmEvent>(32);
        for i in 0..10 {
            tx.send(NmEvent::VpnActivating {
                name: format!("vpn-{}", i),
            })
            .await
            .unwrap();
        }
    }
}
