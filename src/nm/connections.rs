// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! NetworkManager VPN connection helpers with type info.

use std::net::IpAddr;

use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::parsing::{parse_openvpn_endpoints, parse_wireguard_endpoints, VpnEndpoint};

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

/// Result of scanning all NetworkManager VPN profiles for server endpoints.
///
/// Produced by [`detect_vpn_endpoints`] and consumed by the kill switch when
/// building the server-IP allowlist and when warning about configurations the
/// kill switch cannot pre-whitelist.
#[derive(Debug, Default, Clone)]
pub struct VpnEndpointScan {
    /// Deduplicated literal server IPs that can be added to the allowlist.
    pub ips: Vec<IpAddr>,
    /// Names of VPN connections whose only endpoint is a hostname and which
    /// therefore cannot be pre-whitelisted without DNS resolution on the
    /// unprotected network (SHROUD-VULN-041).
    pub hostname_vpns: Vec<String>,
}

/// Run nmcli with a captured stdout, honoring the centralized command path
/// ([`super::nmcli_command`]) and the standard timeout. Returns `None` on
/// timeout, spawn failure, or non-zero exit.
async fn run_nmcli_capture(args: &[&str]) -> Option<String> {
    let output = timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new(super::nmcli_command()).args(args).output(),
    )
    .await;
    match output {
        Ok(Ok(o)) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).to_string()),
        _ => None,
    }
}

/// Scan every configured NM VPN connection for server endpoints.
///
/// Reads the correct nmcli field per VPN type — `vpn.data remote=` for
/// OpenVPN, `wireguard.peers` `endpoint=` for WireGuard — and classifies each
/// endpoint as a whitelist-able IP or an unresolvable hostname via the pure
/// parsers in [`super::parsing`]. The classification, IPv4 re-validation, and
/// deduplication are delegated to the pure [`aggregate_endpoint_scan`] so they
/// can be unit-tested without spawning processes.
///
/// # Security
///
/// Performs NO DNS resolution (SHROUD-VULN-041): hostname endpoints are
/// reported via [`VpnEndpointScan::hostname_vpns`] rather than resolved on the
/// unprotected network. Routes exclusively through [`super::nmcli_command`],
/// preserving the `SHROUD_NMCLI` test gating (SHROUD-VULN-005).
pub async fn detect_vpn_endpoints() -> VpnEndpointScan {
    let mut per_connection: Vec<(String, Vec<VpnEndpoint>)> = Vec::new();

    for conn in list_vpn_connections_with_types().await {
        let endpoints = match conn.vpn_type {
            VpnType::WireGuard => {
                match run_nmcli_capture(&[
                    "-t",
                    "-f",
                    "wireguard.peers",
                    "connection",
                    "show",
                    &conn.name,
                ])
                .await
                {
                    Some(stdout) => parse_wireguard_endpoints(&stdout),
                    None => continue,
                }
            }
            VpnType::OpenVpn => {
                match run_nmcli_capture(&["-t", "-f", "vpn.data", "connection", "show", &conn.name])
                    .await
                {
                    Some(stdout) => parse_openvpn_endpoints(&stdout),
                    None => continue,
                }
            }
            VpnType::Unknown => continue,
        };
        per_connection.push((conn.name, endpoints));
    }

    aggregate_endpoint_scan(per_connection)
}

/// Aggregate per-connection classified endpoints into a deduplicated scan.
///
/// Pure (no I/O) so it is unit-testable without a live NetworkManager. For
/// each connection it:
/// - keeps literal IPs, re-validating IPv4 through
///   [`crate::killswitch::rules::is_valid_ipv4`] (the same validator applied
///   at every rule-interpolation site — SHROUD-VULN-022 family). IPv6 is
///   already type-checked by the `IpAddr` parse.
/// - deduplicates IPs across all tunnels.
/// - flags a connection as hostname-only when it has at least one hostname
///   endpoint and NO usable IP endpoint — that is the configuration whose
///   handshake the kill switch would block (SHROUD-VULN-041 forbids resolving
///   the hostname to fix it on the unprotected path).
fn aggregate_endpoint_scan(per_connection: Vec<(String, Vec<VpnEndpoint>)>) -> VpnEndpointScan {
    let mut scan = VpnEndpointScan::default();

    for (name, endpoints) in per_connection {
        let mut has_ip = false;
        let mut has_hostname = false;
        for endpoint in endpoints {
            match endpoint {
                VpnEndpoint::Ip(ip) => {
                    if let IpAddr::V4(v4) = ip {
                        if !crate::killswitch::rules::is_valid_ipv4(&v4.to_string()) {
                            continue;
                        }
                    }
                    has_ip = true;
                    if !scan.ips.contains(&ip) {
                        scan.ips.push(ip);
                    }
                }
                VpnEndpoint::Hostname(_) => has_hostname = true,
            }
        }

        if has_hostname && !has_ip && !scan.hostname_vpns.contains(&name) {
            scan.hostname_vpns.push(name);
        }
    }

    scan
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

    mod aggregate_endpoint_scan_tests {
        use super::*;
        use std::net::IpAddr;

        fn ip(s: &str) -> IpAddr {
            s.parse().unwrap()
        }

        #[test]
        fn test_single_wireguard_ipv4() {
            let scan = aggregate_endpoint_scan(vec![(
                "wg-us".into(),
                vec![VpnEndpoint::Ip(ip("192.0.2.7"))],
            )]);
            assert_eq!(scan.ips, vec![ip("192.0.2.7")]);
            assert!(scan.hostname_vpns.is_empty());
        }

        #[test]
        fn test_multi_tunnel_dedup() {
            // Two tunnels share a server IP; a third adds a distinct one.
            let scan = aggregate_endpoint_scan(vec![
                ("wg-a".into(), vec![VpnEndpoint::Ip(ip("192.0.2.7"))]),
                ("wg-b".into(), vec![VpnEndpoint::Ip(ip("192.0.2.7"))]),
                ("ovpn-c".into(), vec![VpnEndpoint::Ip(ip("198.51.100.9"))]),
            ]);
            assert_eq!(scan.ips, vec![ip("192.0.2.7"), ip("198.51.100.9")]);
            assert!(scan.hostname_vpns.is_empty());
        }

        #[test]
        fn test_hostname_only_flagged() {
            let scan = aggregate_endpoint_scan(vec![(
                "wg-host".into(),
                vec![VpnEndpoint::Hostname("vpn.example.com".into())],
            )]);
            assert!(scan.ips.is_empty());
            assert_eq!(scan.hostname_vpns, vec!["wg-host".to_string()]);
        }

        #[test]
        fn test_mixed_ip_and_hostname_not_flagged() {
            // A VPN with at least one usable IP can still establish, so it is
            // NOT flagged as hostname-only even if another peer is a hostname.
            let scan = aggregate_endpoint_scan(vec![(
                "wg-mixed".into(),
                vec![
                    VpnEndpoint::Ip(ip("192.0.2.7")),
                    VpnEndpoint::Hostname("roaming.example.com".into()),
                ],
            )]);
            assert_eq!(scan.ips, vec![ip("192.0.2.7")]);
            assert!(scan.hostname_vpns.is_empty());
        }

        #[test]
        fn test_ipv6_kept() {
            let scan = aggregate_endpoint_scan(vec![(
                "wg-v6".into(),
                vec![VpnEndpoint::Ip(ip("2001:db8::1"))],
            )]);
            assert_eq!(scan.ips, vec![ip("2001:db8::1")]);
            assert!(scan.hostname_vpns.is_empty());
        }

        #[test]
        fn test_multiple_hostname_only_vpns() {
            let scan = aggregate_endpoint_scan(vec![
                (
                    "wg-h1".into(),
                    vec![VpnEndpoint::Hostname("a.example.com".into())],
                ),
                (
                    "wg-h2".into(),
                    vec![VpnEndpoint::Hostname("b.example.com".into())],
                ),
            ]);
            assert!(scan.ips.is_empty());
            assert_eq!(
                scan.hostname_vpns,
                vec!["wg-h1".to_string(), "wg-h2".to_string()]
            );
        }

        #[test]
        fn test_empty_input() {
            let scan = aggregate_endpoint_scan(vec![]);
            assert!(scan.ips.is_empty());
            assert!(scan.hostname_vpns.is_empty());
        }

        #[test]
        fn test_connection_without_endpoints_ignored() {
            // A connection that yielded no endpoints (e.g. roaming-only WG peer)
            // contributes nothing and is not flagged.
            let scan = aggregate_endpoint_scan(vec![("wg-empty".into(), vec![])]);
            assert!(scan.ips.is_empty());
            assert!(scan.hostname_vpns.is_empty());
        }
    }
}
