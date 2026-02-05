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
