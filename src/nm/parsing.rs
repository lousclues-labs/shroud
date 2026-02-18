// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! NM output parsing — pure functions, easily testable.
//!
//! Extracts the nmcli output parsing logic from `nm::client` so it
//! can be unit-tested without needing a running NetworkManager.

use crate::state::{ActiveVpnInfo, NmVpnState};

/// Parse VPN connections from nmcli `-t -f NAME,TYPE,STATE con show --active` output.
#[allow(dead_code)]
pub fn parse_active_vpns(stdout: &str) -> Vec<ActiveVpnInfo> {
    let mut vpns = Vec::new();

    for line in stdout.lines() {
        // Split on colon from the right to handle names with colons
        let parts: Vec<&str> = line.rsplitn(3, ':').collect();
        if parts.len() >= 3 {
            let state_str = parts[0];
            let conn_type = parts[1];
            let name = parts[2];

            if conn_type == "vpn" || conn_type == "wireguard" {
                if let Some(state) = match state_str {
                    "activated" => Some(NmVpnState::Activated),
                    "activating" => Some(NmVpnState::Activating),
                    "deactivating" => Some(NmVpnState::Deactivating),
                    _ => None,
                } {
                    vpns.push(ActiveVpnInfo {
                        name: name.to_string(),
                        state,
                    });
                }
            }
        }
    }

    vpns
}

/// Parse VPN connection names from nmcli `-t -f NAME,TYPE con show` output.
#[allow(dead_code)]
pub fn parse_vpn_connections(stdout: &str) -> Vec<String> {
    let mut connections = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.rsplitn(2, ':').collect();
        if parts.len() >= 2 && (parts[0] == "vpn" || parts[0] == "wireguard") {
            connections.push(parts[1].to_string());
        }
    }
    connections
}

/// Parse VPN UUID from nmcli `-t -f UUID,NAME,TYPE con show` output.
///
/// Handles VPN names containing colons by splitting UUID on the first `:`
/// and type on the last `:` (UUIDs and types never contain colons).
#[allow(dead_code)]
pub fn parse_vpn_uuid(stdout: &str, connection_name: &str) -> Option<String> {
    for line in stdout.lines() {
        if let Some((uuid, rest)) = line.split_once(':') {
            if let Some((name, conn_type)) = rest.rsplit_once(':') {
                if (conn_type == "vpn" || conn_type == "wireguard") && name == connection_name {
                    return Some(uuid.to_string());
                }
            }
        }
    }
    None
}

/// Select the best active VPN by priority: activated > activating > deactivating.
#[allow(dead_code)]
pub fn select_best_vpn(vpns: &[ActiveVpnInfo]) -> Option<&ActiveVpnInfo> {
    vpns.iter()
        .find(|v| v.state == NmVpnState::Activated)
        .or_else(|| vpns.iter().find(|v| v.state == NmVpnState::Activating))
        .or_else(|| vpns.iter().find(|v| v.state == NmVpnState::Deactivating))
}

/// Classify a NM connection type string as VPN or not.
#[allow(dead_code)]
pub fn is_vpn_connection_type(conn_type: &str) -> bool {
    conn_type == "vpn" || conn_type == "wireguard"
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod parse_active_vpns_tests {
        use super::*;

        #[test]
        fn test_basic() {
            let output = "my-vpn:vpn:activated\nwg-us:wireguard:activated\n";
            let vpns = parse_active_vpns(output);
            assert_eq!(vpns.len(), 2);
            assert_eq!(vpns[0].name, "my-vpn");
            assert_eq!(vpns[0].state, NmVpnState::Activated);
            assert_eq!(vpns[1].name, "wg-us");
        }

        #[test]
        fn test_colons_in_name() {
            let output = "vpn:server:123:vpn:activating\n";
            let vpns = parse_active_vpns(output);
            assert_eq!(vpns.len(), 1);
            assert_eq!(vpns[0].name, "vpn:server:123");
            assert_eq!(vpns[0].state, NmVpnState::Activating);
        }

        #[test]
        fn test_filters_non_vpn() {
            let output =
                "my-vpn:vpn:activated\nwifi:802-11-wireless:activated\neth0:ethernet:activated\n";
            let vpns = parse_active_vpns(output);
            assert_eq!(vpns.len(), 1);
            assert_eq!(vpns[0].name, "my-vpn");
        }

        #[test]
        fn test_deactivating() {
            let output = "vpn1:vpn:deactivating\n";
            let vpns = parse_active_vpns(output);
            assert_eq!(vpns.len(), 1);
            assert_eq!(vpns[0].state, NmVpnState::Deactivating);
        }

        #[test]
        fn test_unknown_state_ignored() {
            let output = "vpn1:vpn:unknown\n";
            let vpns = parse_active_vpns(output);
            assert!(vpns.is_empty());
        }

        #[test]
        fn test_empty_output() {
            assert!(parse_active_vpns("").is_empty());
        }

        #[test]
        fn test_multiple_states() {
            let output = "vpn1:vpn:activated\nvpn2:vpn:activating\nvpn3:vpn:deactivating\n";
            let vpns = parse_active_vpns(output);
            assert_eq!(vpns.len(), 3);
        }
    }

    mod parse_vpn_connections_tests {
        use super::*;

        #[test]
        fn test_basic() {
            let output = "my-vpn:vpn\nwg-us:wireguard\nwifi:802-11-wireless\n";
            let conns = parse_vpn_connections(output);
            assert_eq!(conns, vec!["my-vpn", "wg-us"]);
        }

        #[test]
        fn test_colons_in_name() {
            let output = "vpn:server:east:vpn\n";
            let conns = parse_vpn_connections(output);
            assert_eq!(conns, vec!["vpn:server:east"]);
        }

        #[test]
        fn test_empty() {
            assert!(parse_vpn_connections("").is_empty());
        }

        #[test]
        fn test_no_vpns() {
            let output = "wifi:802-11-wireless\nethernet:802-3-ethernet\n";
            assert!(parse_vpn_connections(output).is_empty());
        }

        #[test]
        fn test_spaces_in_name() {
            let output = "My VPN Connection:vpn\n";
            let conns = parse_vpn_connections(output);
            assert_eq!(conns, vec!["My VPN Connection"]);
        }
    }

    mod parse_vpn_uuid_tests {
        use super::*;

        #[test]
        fn test_basic() {
            let output = "abc-123:my-vpn:vpn\ndef-456:other:vpn\n";
            assert_eq!(
                parse_vpn_uuid(output, "my-vpn"),
                Some("abc-123".to_string())
            );
        }

        #[test]
        fn test_real_uuid() {
            let output = "550e8400-e29b-41d4-a716-446655440000:my-vpn:vpn\n";
            assert_eq!(
                parse_vpn_uuid(output, "my-vpn"),
                Some("550e8400-e29b-41d4-a716-446655440000".to_string())
            );
        }

        #[test]
        fn test_not_found() {
            let output = "abc-123:my-vpn:vpn\n";
            assert!(parse_vpn_uuid(output, "nonexistent").is_none());
        }

        #[test]
        fn test_ignores_non_vpn() {
            let output = "abc-123:my-wifi:802-11-wireless\n";
            assert!(parse_vpn_uuid(output, "my-wifi").is_none());
        }

        #[test]
        fn test_empty() {
            assert!(parse_vpn_uuid("", "anything").is_none());
        }

        #[test]
        fn test_wireguard() {
            let output = "abc-123:wg0:wireguard\n";
            assert_eq!(parse_vpn_uuid(output, "wg0"), Some("abc-123".to_string()));
        }
    }

    mod select_best_vpn_tests {
        use super::*;

        #[test]
        fn test_prefers_activated() {
            let vpns = vec![
                ActiveVpnInfo {
                    name: "activating-vpn".into(),
                    state: NmVpnState::Activating,
                },
                ActiveVpnInfo {
                    name: "active-vpn".into(),
                    state: NmVpnState::Activated,
                },
            ];
            let best = select_best_vpn(&vpns).unwrap();
            assert_eq!(best.name, "active-vpn");
        }

        #[test]
        fn test_prefers_activating_over_deactivating() {
            let vpns = vec![
                ActiveVpnInfo {
                    name: "leaving".into(),
                    state: NmVpnState::Deactivating,
                },
                ActiveVpnInfo {
                    name: "joining".into(),
                    state: NmVpnState::Activating,
                },
            ];
            let best = select_best_vpn(&vpns).unwrap();
            assert_eq!(best.name, "joining");
        }

        #[test]
        fn test_deactivating_only() {
            let vpns = vec![ActiveVpnInfo {
                name: "leaving".into(),
                state: NmVpnState::Deactivating,
            }];
            let best = select_best_vpn(&vpns).unwrap();
            assert_eq!(best.name, "leaving");
        }

        #[test]
        fn test_empty() {
            assert!(select_best_vpn(&[]).is_none());
        }
    }

    mod is_vpn_type_tests {
        use super::*;

        #[test]
        fn test_vpn_types() {
            assert!(is_vpn_connection_type("vpn"));
            assert!(is_vpn_connection_type("wireguard"));
        }

        #[test]
        fn test_non_vpn_types() {
            assert!(!is_vpn_connection_type("802-11-wireless"));
            assert!(!is_vpn_connection_type("802-3-ethernet"));
            assert!(!is_vpn_connection_type("bridge"));
            assert!(!is_vpn_connection_type(""));
        }
    }
}
