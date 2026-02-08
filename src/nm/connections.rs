//! NetworkManager VPN connection helpers with type info.

use tokio::process::Command;

/// Get the nmcli command path (supports SHROUD_NMCLI env override for testing)
fn nmcli_command() -> String {
    std::env::var("SHROUD_NMCLI").unwrap_or_else(|_| "nmcli".to_string())
}

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

/// Get all VPN connections with their types
pub async fn list_vpn_connections_with_types() -> Vec<VpnConnection> {
    let output = Command::new(nmcli_command())
        .args(["-t", "-f", "NAME,TYPE,UUID", "connection", "show"])
        .output()
        .await;

    let mut connections = Vec::new();

    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 {
                let name = parts[0].to_string();
                let conn_type = parts[1];
                let uuid = parts[2].to_string();

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
    let output = Command::new(nmcli_command())
        .args(["-t", "-f", "connection.type", "connection", "show", name])
        .output()
        .await;

    if let Ok(output) = output {
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
    fn test_nmcli_command_default() {
        // When SHROUD_NMCLI is not set, should default to "nmcli"
        std::env::remove_var("SHROUD_NMCLI");
        assert_eq!(nmcli_command(), "nmcli");
    }

    #[test]
    fn test_nmcli_command_override() {
        std::env::set_var("SHROUD_NMCLI", "/custom/nmcli");
        assert_eq!(nmcli_command(), "/custom/nmcli");
        std::env::remove_var("SHROUD_NMCLI");
    }
}
