// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! NM output parsing — pure functions, easily testable.
//!
//! Extracts the nmcli output parsing logic from `nm::client` so it
//! can be unit-tested without needing a running NetworkManager.
//!
//! ## Performance
//!
//! All parsers operate line-by-line and avoid intermediate `Vec<&str>`
//! allocations by using `rsplit_once` / `split_once` directly. They are
//! called from the NM poll path that runs every `NM_POLL_INTERVAL_SECS`
//! seconds, so the hot path stays alloc-free except for the owned
//! `String`s in the returned data.

use std::net::IpAddr;

use crate::state::{ActiveVpnInfo, NmVpnState};

/// Classify a NM connection type string as VPN or not.
#[inline]
pub fn is_vpn_connection_type(conn_type: &str) -> bool {
    conn_type == "vpn" || conn_type == "wireguard"
}

/// Map an nmcli VPN state string to [`NmVpnState`].
///
/// Returns `None` for unknown / unmapped states (e.g. NM's "deactivated"
/// transient — we treat absence of an active VPN as the source of truth).
#[inline]
fn parse_vpn_state(state_str: &str) -> Option<NmVpnState> {
    match state_str {
        "activated" => Some(NmVpnState::Activated),
        "activating" => Some(NmVpnState::Activating),
        "deactivating" => Some(NmVpnState::Deactivating),
        _ => None,
    }
}

/// Parse VPN connections from nmcli `-t -f NAME,TYPE,STATE con show --active` output.
///
/// Format per line: `NAME:TYPE:STATE`. NAME may contain colons, so we split
/// from the right.
pub fn parse_active_vpns(stdout: &str) -> Vec<ActiveVpnInfo> {
    let mut vpns = Vec::new();

    for line in stdout.lines() {
        // Split from the right twice: peel off STATE, then TYPE; what remains is NAME.
        let Some((rest, state_str)) = line.rsplit_once(':') else {
            continue;
        };
        let Some((name, conn_type)) = rest.rsplit_once(':') else {
            continue;
        };
        if !is_vpn_connection_type(conn_type) {
            continue;
        }
        if let Some(state) = parse_vpn_state(state_str) {
            vpns.push(ActiveVpnInfo {
                name: name.to_string(),
                state,
            });
        }
    }

    vpns
}

/// Parse VPN connection names from nmcli `-t -f NAME,TYPE con show` output.
///
/// Format per line: `NAME:TYPE`. NAME may contain colons, so we split from the right.
pub fn parse_vpn_connections(stdout: &str) -> Vec<String> {
    let mut connections = Vec::new();
    for line in stdout.lines() {
        if let Some((name, conn_type)) = line.rsplit_once(':') {
            if is_vpn_connection_type(conn_type) {
                connections.push(name.to_string());
            }
        }
    }
    connections
}

/// Parse VPN UUID from nmcli `-t -f UUID,NAME,TYPE con show` output.
///
/// Format per line: `UUID:NAME:TYPE`. UUIDs are fixed-format (no colons),
/// so split first on `:` to isolate UUID, then split the rest from the right
/// to peel off TYPE and leave NAME (which may contain colons).
pub fn parse_vpn_uuid(stdout: &str, connection_name: &str) -> Option<String> {
    for line in stdout.lines() {
        if let Some((uuid, rest)) = line.split_once(':') {
            if let Some((name, conn_type)) = rest.rsplit_once(':') {
                if is_vpn_connection_type(conn_type) && name == connection_name {
                    return Some(uuid.to_string());
                }
            }
        }
    }
    None
}

/// Select the best active VPN by priority: activated > activating > deactivating.
pub fn select_best_vpn(vpns: &[ActiveVpnInfo]) -> Option<&ActiveVpnInfo> {
    vpns.iter()
        .find(|v| v.state == NmVpnState::Activated)
        .or_else(|| vpns.iter().find(|v| v.state == NmVpnState::Activating))
        .or_else(|| vpns.iter().find(|v| v.state == NmVpnState::Deactivating))
}

// =========================================================================
// VPN endpoint parsing (kill switch server-IP allowlist)
// =========================================================================

/// A VPN server endpoint extracted from a NetworkManager connection profile.
///
/// The kill switch can only pre-whitelist [`VpnEndpoint::Ip`] endpoints.
/// [`VpnEndpoint::Hostname`] endpoints cannot be whitelisted without DNS
/// resolution, which is intentionally NOT performed on the unprotected
/// enable path (SHROUD-VULN-041). Callers surface hostname endpoints to the
/// user instead of silently resolving them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VpnEndpoint {
    /// A literal IP address that can be added to the kill switch allowlist.
    Ip(IpAddr),
    /// A hostname endpoint that cannot be pre-whitelisted (no DNS on the
    /// unprotected network — SHROUD-VULN-041).
    Hostname(String),
}

/// Extract the host portion of a `host:port` endpoint string.
///
/// Handles, without any DNS resolution:
/// - IPv6 bracketed forms: `[2001:db8::1]:51820` → `2001:db8::1`
/// - IPv4 / hostname with port: `192.0.2.7:51820` → `192.0.2.7`,
///   `vpn.example.com:51820` → `vpn.example.com`
/// - Bare IPv6 (no port): `2001:db8::1` → `2001:db8::1`
/// - Bare IPv4 / hostname (no port): `192.0.2.7` → `192.0.2.7`
///
/// The returned slice borrows from `endpoint`; it is never resolved.
pub fn endpoint_host(endpoint: &str) -> &str {
    let e = endpoint.trim();

    // IPv6 bracketed form: [addr] or [addr]:port. Return the inner address.
    if let Some(rest) = e.strip_prefix('[') {
        return match rest.split_once(']') {
            Some((inner, _)) => inner,
            // Malformed (missing closing bracket) — return what we have.
            None => rest,
        };
    }

    // Exactly one colon → `host:port` (IPv4 or hostname). Strip the port only
    // if it is a valid u16, otherwise keep the whole token.
    //
    // More than one colon and no brackets → bare IPv6 literal (NM always
    // brackets IPv6 endpoints that carry a port), so leave it intact.
    if e.bytes().filter(|&b| b == b':').count() == 1 {
        if let Some((host, port)) = e.rsplit_once(':') {
            if port.parse::<u16>().is_ok() {
                return host;
            }
        }
    }

    e
}

/// Classify an endpoint host string as a whitelist-able IP or a hostname.
///
/// Performs no DNS resolution: a non-IP host is returned as
/// [`VpnEndpoint::Hostname`] (SHROUD-VULN-041).
pub fn classify_endpoint_host(host: &str) -> VpnEndpoint {
    match host.parse::<IpAddr>() {
        Ok(ip) => VpnEndpoint::Ip(ip),
        Err(_) => VpnEndpoint::Hostname(host.to_string()),
    }
}

/// Parse WireGuard peer endpoints from nmcli `-t -f wireguard.peers con show <name>`.
///
/// The terse output is a single line of the form:
///
/// ```text
/// wireguard.peers:<pubkey> allowed-ips=0.0.0.0/0 endpoint=192.0.2.7:51820, <pubkey2> ... endpoint=[2001:db8::1]:51820
/// ```
///
/// Peers are comma-separated; each peer's attributes are space-separated and
/// appear in arbitrary order, so we scan for the `endpoint=` token. The port
/// is stripped and IPv6 brackets are removed via [`endpoint_host`].
pub fn parse_wireguard_endpoints(stdout: &str) -> Vec<VpnEndpoint> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let Some(value) = line.strip_prefix("wireguard.peers:") else {
            continue;
        };
        for peer in value.split(',') {
            for token in peer.split_whitespace() {
                if let Some(ep) = token.strip_prefix("endpoint=") {
                    let host = endpoint_host(ep);
                    if !host.is_empty() {
                        out.push(classify_endpoint_host(host));
                    }
                }
            }
        }
    }
    out
}

/// Parse OpenVPN remote endpoints from nmcli `-t -f vpn.data con show <name>`.
///
/// The terse output is a single line of the form:
///
/// ```text
/// vpn.data: remote = 192.0.2.7:1194, remote-cert-tls = server, remote-random = yes
/// ```
///
/// Items are comma-separated `key = value` pairs (note the spaces around `=`).
/// Only the exact key `remote` is matched — `remote-cert-tls`, `remote-random`
/// and similar keys are deliberately ignored.
pub fn parse_openvpn_endpoints(stdout: &str) -> Vec<VpnEndpoint> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let Some(data) = line.strip_prefix("vpn.data:") else {
            continue;
        };
        for item in data.split(',') {
            let Some((key, value)) = item.split_once('=') else {
                continue;
            };
            if key.trim() != "remote" {
                continue;
            }
            // The value may be `host:port`; take the first whitespace token to
            // tolerate trailing `proto`/flags some profiles append.
            let raw = value.split_whitespace().next().unwrap_or("");
            let host = endpoint_host(raw);
            if !host.is_empty() {
                out.push(classify_endpoint_host(host));
            }
        }
    }
    out
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

    mod endpoint_host_tests {
        use super::*;

        #[test]
        fn test_ipv4_with_port() {
            assert_eq!(endpoint_host("192.0.2.7:51820"), "192.0.2.7");
        }

        #[test]
        fn test_ipv4_without_port() {
            assert_eq!(endpoint_host("192.0.2.7"), "192.0.2.7");
        }

        #[test]
        fn test_ipv6_bracketed_with_port() {
            assert_eq!(endpoint_host("[2001:db8::1]:51820"), "2001:db8::1");
        }

        #[test]
        fn test_ipv6_bracketed_without_port() {
            assert_eq!(endpoint_host("[2001:db8::1]"), "2001:db8::1");
        }

        #[test]
        fn test_ipv6_bare_no_port_kept_intact() {
            // NM always brackets IPv6 endpoints that carry a port, so a
            // multi-colon unbracketed value is a bare IPv6 literal.
            assert_eq!(endpoint_host("2001:db8::1"), "2001:db8::1");
        }

        #[test]
        fn test_hostname_with_port() {
            assert_eq!(endpoint_host("vpn.example.com:51820"), "vpn.example.com");
        }

        #[test]
        fn test_hostname_without_port() {
            assert_eq!(endpoint_host("vpn.example.com"), "vpn.example.com");
        }

        #[test]
        fn test_non_numeric_port_kept() {
            // A trailing non-port token is not stripped.
            assert_eq!(endpoint_host("host:notaport"), "host:notaport");
        }

        #[test]
        fn test_whitespace_trimmed() {
            assert_eq!(endpoint_host("  192.0.2.7:1194  "), "192.0.2.7");
        }

        #[test]
        fn test_malformed_bracket() {
            assert_eq!(endpoint_host("[2001:db8::1"), "2001:db8::1");
        }
    }

    mod classify_endpoint_host_tests {
        use super::*;

        #[test]
        fn test_ipv4_classified_as_ip() {
            assert_eq!(
                classify_endpoint_host("192.0.2.7"),
                VpnEndpoint::Ip("192.0.2.7".parse().unwrap())
            );
        }

        #[test]
        fn test_ipv6_classified_as_ip() {
            assert_eq!(
                classify_endpoint_host("2001:db8::1"),
                VpnEndpoint::Ip("2001:db8::1".parse().unwrap())
            );
        }

        #[test]
        fn test_hostname_classified_as_hostname() {
            assert_eq!(
                classify_endpoint_host("vpn.example.com"),
                VpnEndpoint::Hostname("vpn.example.com".to_string())
            );
        }

        #[test]
        fn test_empty_classified_as_hostname() {
            assert_eq!(
                classify_endpoint_host(""),
                VpnEndpoint::Hostname(String::new())
            );
        }
    }

    mod parse_wireguard_endpoints_tests {
        use super::*;

        #[test]
        fn test_single_ipv4_peer() {
            let out = "wireguard.peers:aKey= allowed-ips=0.0.0.0/0 endpoint=192.0.2.7:51820\n";
            assert_eq!(
                parse_wireguard_endpoints(out),
                vec![VpnEndpoint::Ip("192.0.2.7".parse().unwrap())]
            );
        }

        #[test]
        fn test_ipv6_bracketed_peer() {
            let out = "wireguard.peers:aKey= allowed-ips=::/0 endpoint=[2001:db8::1]:51820\n";
            assert_eq!(
                parse_wireguard_endpoints(out),
                vec![VpnEndpoint::Ip("2001:db8::1".parse().unwrap())]
            );
        }

        #[test]
        fn test_multiple_peers_v4_and_v6() {
            // Exact format observed from nmcli 1.56.1 (comma-separated peers,
            // attribute order varies, endpoint appears last).
            let out = "wireguard.peers:K1= allowed-ips=0.0.0.0/0 endpoint=192.0.2.7:51820, \
                       K2= allowed-ips=::/0 endpoint=[2001:db8::1]:51820\n";
            assert_eq!(
                parse_wireguard_endpoints(out),
                vec![
                    VpnEndpoint::Ip("192.0.2.7".parse().unwrap()),
                    VpnEndpoint::Ip("2001:db8::1".parse().unwrap()),
                ]
            );
        }

        #[test]
        fn test_hostname_peer_rejected_as_non_ip() {
            let out =
                "wireguard.peers:aKey= allowed-ips=0.0.0.0/0 endpoint=vpn.example.com:51820\n";
            assert_eq!(
                parse_wireguard_endpoints(out),
                vec![VpnEndpoint::Hostname("vpn.example.com".to_string())]
            );
        }

        #[test]
        fn test_peer_without_endpoint_skipped() {
            // A peer with only allowed-ips (roaming peer) contributes nothing.
            let out = "wireguard.peers:aKey= allowed-ips=0.0.0.0/0\n";
            assert!(parse_wireguard_endpoints(out).is_empty());
        }

        #[test]
        fn test_attribute_order_independent() {
            let out = "wireguard.peers:aKey= endpoint=192.0.2.7:51820 allowed-ips=0.0.0.0/0\n";
            assert_eq!(
                parse_wireguard_endpoints(out),
                vec![VpnEndpoint::Ip("192.0.2.7".parse().unwrap())]
            );
        }

        #[test]
        fn test_empty_output() {
            assert!(parse_wireguard_endpoints("").is_empty());
        }

        #[test]
        fn test_non_wireguard_lines_ignored() {
            let out = "connection.id:my-wg\nwireguard.private-key:<hidden>\n";
            assert!(parse_wireguard_endpoints(out).is_empty());
        }
    }

    mod parse_openvpn_endpoints_tests {
        use super::*;

        #[test]
        fn test_basic_remote() {
            // Exact format observed from nmcli 1.56.1 (spaces around `=`).
            let out = "vpn.data: remote = 192.0.2.7:1194, remote-cert-tls = server\n";
            assert_eq!(
                parse_openvpn_endpoints(out),
                vec![VpnEndpoint::Ip("192.0.2.7".parse().unwrap())]
            );
        }

        #[test]
        fn test_realistic_vpn_data_format() {
            // Faithful to a real `nmcli -t -f vpn.data con show <ovpn>` line:
            // no space after `vpn.data:`, `remote` appears mid-list among other
            // keys, and `remote-cert-tls` is present as a decoy.
            let out = "vpn.data:auth = SHA512, ca = /home/u/.local/ca.crt, \
                       remote = 192.145.116.114:1194, remote-cert-tls = server, \
                       remote-random = yes\n";
            assert_eq!(
                parse_openvpn_endpoints(out),
                vec![VpnEndpoint::Ip("192.145.116.114".parse().unwrap())]
            );
        }

        #[test]
        fn test_ignores_remote_lookalike_keys() {
            // `remote-cert-tls`/`remote-random` must NOT be parsed as endpoints.
            let out = "vpn.data: remote-cert-tls = server, remote-random = yes\n";
            assert!(parse_openvpn_endpoints(out).is_empty());
        }

        #[test]
        fn test_hostname_remote_rejected_as_non_ip() {
            let out = "vpn.data: remote = vpn.example.com:1194\n";
            assert_eq!(
                parse_openvpn_endpoints(out),
                vec![VpnEndpoint::Hostname("vpn.example.com".to_string())]
            );
        }

        #[test]
        fn test_remote_without_port() {
            let out = "vpn.data: remote = 192.0.2.7\n";
            assert_eq!(
                parse_openvpn_endpoints(out),
                vec![VpnEndpoint::Ip("192.0.2.7".parse().unwrap())]
            );
        }

        #[test]
        fn test_empty_output() {
            assert!(parse_openvpn_endpoints("").is_empty());
        }

        #[test]
        fn test_non_vpn_data_lines_ignored() {
            let out =
                "connection.id:my-ovpn\nvpn.service-type:org.freedesktop.NetworkManager.openvpn\n";
            assert!(parse_openvpn_endpoints(out).is_empty());
        }
    }
}
