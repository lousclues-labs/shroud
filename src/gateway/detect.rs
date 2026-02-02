//! Interface auto-detection for gateway mode.

use log::{debug, warn};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Detect the LAN interface.
///
/// Looks for a non-loopback, non-VPN interface with an IP address.
pub fn detect_lan_interface() -> Option<String> {
    let interfaces = list_interfaces();

    for iface in interfaces {
        // Skip loopback
        if iface == "lo" {
            continue;
        }

        // Skip VPN interfaces
        if is_vpn_interface(&iface) {
            continue;
        }

        // Skip interfaces without IP
        if !has_ipv4_address(&iface) {
            continue;
        }

        debug!("Detected LAN interface: {}", iface);
        return Some(iface);
    }

    warn!("Could not auto-detect LAN interface");
    None
}

/// Detect the VPN interface.
///
/// Looks for tun*, tap*, or wg* interfaces.
pub fn detect_vpn_interface() -> Option<String> {
    let interfaces = list_interfaces();

    for iface in interfaces {
        if is_vpn_interface(&iface) && has_ipv4_address(&iface) {
            debug!("Detected VPN interface: {}", iface);
            return Some(iface);
        }
    }

    warn!("Could not auto-detect VPN interface");
    None
}

/// List all network interfaces.
fn list_interfaces() -> Vec<String> {
    let net_path = Path::new("/sys/class/net");

    match fs::read_dir(net_path) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(e) => {
            warn!("Failed to list interfaces: {}", e);
            vec![]
        }
    }
}

/// Check if an interface is a VPN interface.
fn is_vpn_interface(iface: &str) -> bool {
    iface.starts_with("tun")
        || iface.starts_with("tap")
        || iface.starts_with("wg")
        || iface.starts_with("proton")
        || iface.starts_with("mullvad")
        || iface.starts_with("nordlynx")
}

/// Check if an interface has an IPv4 address.
fn has_ipv4_address(iface: &str) -> bool {
    let output = Command::new("ip").args(["addr", "show", iface]).output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.contains("inet ")
        }
        _ => false,
    }
}

/// Get the IP address of an interface.
pub fn get_interface_ip(iface: &str) -> Option<String> {
    let output = Command::new("ip")
        .args(["-4", "addr", "show", iface])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with("inet ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let ip = parts[1].split('/').next()?;
                return Some(ip.to_string());
            }
        }
    }

    None
}

/// Get the subnet of an interface.
pub fn get_interface_subnet(iface: &str) -> Option<String> {
    let output = Command::new("ip")
        .args(["-4", "addr", "show", iface])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with("inet ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return Some(parts[1].to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_vpn_interface() {
        assert!(is_vpn_interface("tun0"));
        assert!(is_vpn_interface("wg0"));
        assert!(is_vpn_interface("tap0"));
        assert!(!is_vpn_interface("eth0"));
        assert!(!is_vpn_interface("enp3s0"));
        assert!(!is_vpn_interface("wlan0"));
    }

    #[test]
    fn test_list_interfaces_contains_lo() {
        let interfaces = list_interfaces();
        assert!(interfaces.contains(&"lo".to_string()));
    }
}
