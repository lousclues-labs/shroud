// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! NetworkManager VPN connection helpers with type info.

use tokio::process::Command;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct VpnConnection {
    pub name: String,
    pub vpn_type: VpnType,
    pub uuid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VpnType {
    WireGuard,
    OpenVpn,
    Unknown,
}

impl std::fmt::Display for VpnType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VpnType::WireGuard => write!(f, "wireguard"),
            VpnType::OpenVpn => write!(f, "openvpn"),
            VpnType::Unknown => write!(f, "unknown"),
        }
    }
}

/// Timeout for nmcli commands in connections module
const NMCLI_TIMEOUT_SECS: u64 = 30;

/// Get all VPN connections with their types
pub async fn list_vpn_connections_with_types() -> Vec<VpnConnection> {
    let output = timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new(super::nmcli_command())
            .args(["-t", "-f", "NAME,TYPE,UUID", "connection", "show"])
            .output(),
    )
    .await;

    let mut connections = Vec::new();

    if let Ok(Ok(output)) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);

        for line in stdout.lines() {
            // SECURITY: Use rsplitn to split from the right. nmcli -t uses ':'
            // as a delimiter, but connection names can contain ':'. The type and
            // UUID fields (rightmost) never contain colons (SHROUD-VULN-027).
            let parts: Vec<&str> = line.rsplitn(3, ':').collect();
            if parts.len() >= 3 {
                // rsplitn reverses order: [uuid, type, name]
                let name = parts[2].to_string();
                let conn_type = parts[1];
                let uuid = parts[0].to_string();

                let vpn_type = match conn_type {
                    "wireguard" => VpnType::WireGuard,
                    "vpn" => VpnType::OpenVpn,
                    _ => continue,
                };

                connections.push(VpnConnection {
                    name,
                    vpn_type,
                    uuid,
                });
            }
        }
    }

    connections
}

/// Get VPN type for a specific connection
pub async fn get_vpn_type(name: &str) -> VpnType {
    let output = timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new(super::nmcli_command())
            .args(["-t", "-f", "connection.type", "connection", "show", name])
            .output(),
    )
    .await;

    if let Ok(Ok(output)) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let type_str = stdout.trim().trim_start_matches("connection.type:");

        match type_str {
            "wireguard" => VpnType::WireGuard,
            "vpn" => VpnType::OpenVpn,
            _ => VpnType::Unknown,
        }
    } else {
        VpnType::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vpn_type_display_wireguard() {
        assert_eq!(VpnType::WireGuard.to_string(), "wireguard");
    }

    #[test]
    fn test_vpn_type_display_openvpn() {
        assert_eq!(VpnType::OpenVpn.to_string(), "openvpn");
    }

    #[test]
    fn test_vpn_type_display_unknown() {
        assert_eq!(VpnType::Unknown.to_string(), "unknown");
    }

    #[test]
    fn test_vpn_type_equality() {
        assert_eq!(VpnType::WireGuard, VpnType::WireGuard);
        assert_ne!(VpnType::WireGuard, VpnType::OpenVpn);
        assert_ne!(VpnType::OpenVpn, VpnType::Unknown);
    }

    #[test]
    fn test_vpn_type_clone() {
        let t = VpnType::WireGuard;
        let cloned = t;
        assert_eq!(t, cloned);
    }

    #[test]
    fn test_vpn_type_debug() {
        let debug = format!("{:?}", VpnType::OpenVpn);
        assert!(debug.contains("OpenVpn"));
    }

    #[test]
    fn test_vpn_connection_struct() {
        let conn = VpnConnection {
            name: "my-vpn".into(),
            vpn_type: VpnType::WireGuard,
            uuid: "abc-123".into(),
        };
        assert_eq!(conn.name, "my-vpn");
        assert_eq!(conn.vpn_type, VpnType::WireGuard);
        assert_eq!(conn.uuid, "abc-123");
    }

    #[test]
    fn test_vpn_connection_clone() {
        let conn = VpnConnection {
            name: "vpn1".into(),
            vpn_type: VpnType::OpenVpn,
            uuid: "uuid-1".into(),
        };
        let cloned = conn.clone();
        assert_eq!(cloned.name, "vpn1");
        assert_eq!(cloned.vpn_type, VpnType::OpenVpn);
    }

    #[test]
    fn test_nmcli_command_returns_string() {
        // nmcli_command() should return a non-empty string regardless of env
        let cmd = crate::nm::nmcli_command();
        assert!(!cmd.is_empty());
        // Should be either "nmcli" or a custom path
        assert!(cmd == "nmcli" || cmd.starts_with('/'));
    }
}
