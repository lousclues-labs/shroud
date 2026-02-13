//! Kill switch rule generation — pure functions, easily testable.
//!
//! Contains pure-logic helpers for building firewall rule strings,
//! validating DNS modes, and classifying network traffic.
//! No system calls, no I/O.

#![allow(dead_code)]

use std::net::{IpAddr, Ipv4Addr};

/// Validate that a string is a safe IPv4 address (no injection characters).
/// Returns true only for valid IPv4 addresses.
pub fn is_valid_ipv4(s: &str) -> bool {
    s.parse::<Ipv4Addr>().is_ok()
}

/// Validate that a string is a safe CIDR notation (addr/prefix).
/// Returns true only for valid private/link-local CIDR ranges.
pub fn is_valid_private_cidr(s: &str) -> bool {
    if let Some((addr_str, prefix_str)) = s.split_once('/') {
        if let (Ok(addr), Ok(prefix)) = (addr_str.parse::<Ipv4Addr>(), prefix_str.parse::<u32>()) {
            if !(8..=32).contains(&prefix) {
                return false; // Reject /0 through /7 — too broad
            }
            // Must be RFC1918 or link-local
            return addr.is_private() || addr.is_link_local();
        }
    }
    false
}

/// LAN subnets that should always be allowed (RFC 1918 + link-local).
pub const LAN_SUBNETS: &[&str] = &[
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16", // Link-local
];

/// Detect actual local network subnets from system interfaces.
///
/// Returns CIDR strings for non-loopback, non-tunnel interfaces.
/// Falls back to full RFC1918 ranges if detection fails.
pub fn detect_local_subnets() -> Vec<String> {
    let output = match std::process::Command::new("ip")
        .args(["-o", "-4", "addr", "show", "scope", "global"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => {
            tracing::warn!("Failed to detect local subnets, using RFC1918 fallback");
            return LAN_SUBNETS.iter().map(|s| s.to_string()).collect();
        }
    };

    let mut subnets = Vec::new();
    for line in output.lines() {
        // Format: "N: iface    inet addr/prefix ..."
        let parts: Vec<&str> = line.split_whitespace().collect();
        // Skip tunnel/VPN interfaces
        if let Some(iface) = parts.get(1) {
            if iface.starts_with("tun")
                || iface.starts_with("tap")
                || iface.starts_with("wg")
                || *iface == "lo"
                // SECURITY: Skip virtual/container interfaces that widen the
                // kill switch LAN exception unnecessarily (SHROUD-VULN-042).
                || iface.starts_with("docker")
                || iface.starts_with("veth")
                || iface.starts_with("virbr")
                || iface.starts_with("br-")
                || iface.starts_with("cni")
                || iface.starts_with("flannel")
                || iface.starts_with("podman")
            {
                continue;
            }
        }
        // Extract addr/prefix (4th field after "inet")
        if let Some(pos) = parts.iter().position(|&p| p == "inet") {
            if let Some(addr_prefix) = parts.get(pos + 1) {
                if let Some((addr_str, prefix_str)) = addr_prefix.split_once('/') {
                    if let (Ok(addr), Ok(prefix)) = (
                        addr_str.parse::<std::net::Ipv4Addr>(),
                        prefix_str.parse::<u32>(),
                    ) {
                        // Mask host bits to get network CIDR
                        let mask = if prefix == 0 {
                            0u32
                        } else {
                            !0u32 << (32 - prefix)
                        };
                        let net = u32::from(addr) & mask;
                        let net_addr = std::net::Ipv4Addr::from(net);
                        let cidr = format!("{}/{}", net_addr, prefix);

                        // SECURITY: Only allow private (RFC1918) and link-local subnets.
                        // Reject 0.0.0.0/0 or any public range that would open the
                        // kill switch to all traffic (SHROUD-VULN-021).
                        if !is_valid_private_cidr(&cidr) {
                            tracing::warn!(
                                "Rejected non-private subnet from interface detection: {}",
                                cidr
                            );
                            continue;
                        }

                        subnets.push(cidr);
                    }
                }
            }
        }
    }

    // Always include link-local for mDNS/device discovery
    subnets.push("169.254.0.0/16".to_string());

    if subnets.len() <= 1 {
        // Only link-local detected — fall back to RFC1918
        tracing::debug!("No LAN subnets detected, using RFC1918 fallback");
        return LAN_SUBNETS.iter().map(|s| s.to_string()).collect();
    }

    tracing::debug!("Detected local subnets: {:?}", subnets);
    subnets
}

/// Build LAN allow rules for specific subnets.
pub fn build_lan_rules_for_subnets(chain: &str, subnets: &[String]) -> Vec<String> {
    subnets
        .iter()
        .map(|subnet| format!("iptables -A {} -d {} -j ACCEPT", chain, subnet))
        .collect()
}

/// Build LAN rules with port restrictions (only common services).
pub fn build_lan_restricted_rules(chain: &str, subnets: &[String]) -> Vec<String> {
    let mut rules = Vec::new();
    for subnet in subnets {
        // ICMP (ping)
        rules.push(format!(
            "iptables -A {} -d {} -p icmp -j ACCEPT",
            chain, subnet
        ));
        // DNS to LAN resolvers
        rules.push(format!(
            "iptables -A {} -d {} -p udp --dport 53 -j ACCEPT",
            chain, subnet
        ));
        // mDNS/Bonjour
        rules.push(format!(
            "iptables -A {} -d {} -p udp --dport 5353 -j ACCEPT",
            chain, subnet
        ));
        // SSDP/UPnP
        rules.push(format!(
            "iptables -A {} -d {} -p udp --dport 1900 -j ACCEPT",
            chain, subnet
        ));
        // Printing (IPP)
        rules.push(format!(
            "iptables -A {} -d {} -p tcp --dport 631 -j ACCEPT",
            chain, subnet
        ));
        // SMB
        rules.push(format!(
            "iptables -A {} -d {} -p tcp --dport 445 -j ACCEPT",
            chain, subnet
        ));
        // NetBIOS
        rules.push(format!(
            "iptables -A {} -d {} -p udp --dport 137:138 -j ACCEPT",
            chain, subnet
        ));
        rules.push(format!(
            "iptables -A {} -d {} -p tcp --dport 139 -j ACCEPT",
            chain, subnet
        ));
    }
    rules
}

/// Well-known DoH provider IPs to block.
pub const DOH_PROVIDERS: &[&str] = &[
    "1.1.1.1",         // Cloudflare
    "1.0.0.1",         // Cloudflare
    "8.8.8.8",         // Google
    "8.8.4.4",         // Google
    "9.9.9.9",         // Quad9
    "149.112.112.112", // Quad9
    "208.67.222.222",  // OpenDNS (Cisco)
    "208.67.220.220",  // OpenDNS (Cisco)
    "94.140.14.14",    // AdGuard
    "94.140.15.15",    // AdGuard
    "185.228.168.168", // CleanBrowsing
    "185.228.169.168", // CleanBrowsing
    "8.26.56.26",      // Comodo
    "8.20.247.20",     // Comodo
];

/// VPN tunnel interface prefixes.
pub const VPN_INTERFACE_PREFIXES: &[&str] = &["tun", "tap", "wg"];

/// Classify an IP address.
#[derive(Debug, PartialEq)]
pub enum IpClass {
    Loopback,
    LinkLocal,
    PrivateLan,
    Public,
}

/// Classify an IP address into a category.
pub fn classify_ip(ip: &IpAddr) -> IpClass {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                IpClass::Loopback
            } else if v4.is_link_local() {
                IpClass::LinkLocal
            } else if v4.is_private() {
                IpClass::PrivateLan
            } else {
                IpClass::Public
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                IpClass::Loopback
            } else if (v6.segments()[0] & 0xffc0) == 0xfe80 {
                // fe80::/10 — IPv6 link-local
                IpClass::LinkLocal
            } else {
                IpClass::Public
            }
        }
    }
}

/// Check if an IP address is a well-known DoH provider.
pub fn is_doh_provider(ip: &str) -> bool {
    DOH_PROVIDERS.contains(&ip)
}

/// Build an iptables rule string for allowing a specific VPN server.
pub fn build_server_allow_rule(server_ip: &IpAddr, chain: &str) -> String {
    format!("iptables -A {} -d {} -j ACCEPT", chain, server_ip)
}

/// Build loopback allow rule.
pub fn build_loopback_rule(chain: &str) -> String {
    format!("iptables -A {} -o lo -j ACCEPT", chain)
}

/// Build LAN allow rules (using full RFC1918 fallback ranges).
pub fn build_lan_rules(chain: &str) -> Vec<String> {
    let fallback: Vec<String> = LAN_SUBNETS.iter().map(|s| s.to_string()).collect();
    build_lan_rules_for_subnets(chain, &fallback)
}

/// Build VPN interface allow rules.
pub fn build_vpn_interface_rules(chain: &str) -> Vec<String> {
    VPN_INTERFACE_PREFIXES
        .iter()
        .map(|prefix| format!("iptables -A {} -o {}+ -j ACCEPT", chain, prefix))
        .collect()
}

/// Build DNS tunnel-mode rules (allow DNS only through VPN tunnel).
pub fn build_dns_tunnel_rules(chain: &str) -> Vec<String> {
    let mut rules = Vec::new();

    // Allow DNS on VPN interfaces
    for prefix in VPN_INTERFACE_PREFIXES {
        rules.push(format!(
            "iptables -A {} -o {}+ -p udp --dport 53 -j ACCEPT",
            chain, prefix
        ));
        rules.push(format!(
            "iptables -A {} -o {}+ -p tcp --dport 53 -j ACCEPT",
            chain, prefix
        ));
    }

    // Block all other DNS
    rules.push(format!("iptables -A {} -p udp --dport 53 -j DROP", chain));
    rules.push(format!("iptables -A {} -p tcp --dport 53 -j DROP", chain));
    // Block DNS-over-TLS
    rules.push(format!("iptables -A {} -p tcp --dport 853 -j DROP", chain));

    rules
}

/// Build DNS localhost-mode rules (allow DNS only to 127.0.0.1 and 127.0.0.53).
pub fn build_dns_localhost_rules(chain: &str) -> Vec<String> {
    let mut rules = Vec::new();

    // SECURITY: Only allow DNS to specific loopback addresses,
    // not the entire 127.0.0.0/8 range.
    rules.push(format!(
        "iptables -A {} -d 127.0.0.1 -p udp --dport 53 -j ACCEPT",
        chain
    ));
    rules.push(format!(
        "iptables -A {} -d 127.0.0.1 -p tcp --dport 53 -j ACCEPT",
        chain
    ));
    rules.push(format!(
        "iptables -A {} -d 127.0.0.53 -p udp --dport 53 -j ACCEPT",
        chain
    ));
    rules.push(format!(
        "iptables -A {} -d 127.0.0.53 -p tcp --dport 53 -j ACCEPT",
        chain
    ));

    // Block all other DNS
    rules.push(format!("iptables -A {} -p udp --dport 53 -j DROP", chain));
    rules.push(format!("iptables -A {} -p tcp --dport 53 -j DROP", chain));
    rules.push(format!("iptables -A {} -p tcp --dport 853 -j DROP", chain));

    rules
}

/// Build DNS any-mode rules (allow all DNS - not recommended).
pub fn build_dns_any_rules(chain: &str) -> Vec<String> {
    vec![
        format!("iptables -A {} -p udp --dport 53 -j ACCEPT", chain),
        format!("iptables -A {} -p tcp --dport 53 -j ACCEPT", chain),
    ]
}

/// Build DoH blocking rules.
pub fn build_doh_blocking_rules(chain: &str) -> Vec<String> {
    DOH_PROVIDERS
        .iter()
        .map(|ip| format!("iptables -A {} -d {} -p tcp --dport 443 -j DROP", chain, ip))
        .collect()
}

/// Build DoH blocking rules from a custom blocklist.
pub fn build_custom_doh_blocking_rules(chain: &str, custom_ips: &[String]) -> Vec<String> {
    custom_ips
        .iter()
        .map(|ip| format!("iptables -A {} -d {} -p tcp --dport 443 -j DROP", chain, ip))
        .collect()
}

/// Build the final DROP rule (must be last in chain).
pub fn build_default_drop_rule(chain: &str) -> String {
    format!("iptables -A {} -j DROP", chain)
}

/// Build IPv6 blocking rules (drop all IPv6 output except loopback).
pub fn build_ipv6_block_rules(chain: &str) -> Vec<String> {
    vec![
        format!("ip6tables -A {} -o lo -j ACCEPT", chain),
        format!("ip6tables -A {} -j DROP", chain),
    ]
}

/// Build IPv6 tunnel-mode rules (allow IPv6 only on VPN interfaces).
pub fn build_ipv6_tunnel_rules(chain: &str) -> Vec<String> {
    let mut rules = Vec::new();

    rules.push(format!("ip6tables -A {} -o lo -j ACCEPT", chain));
    for prefix in VPN_INTERFACE_PREFIXES {
        rules.push(format!("ip6tables -A {} -o {}+ -j ACCEPT", chain, prefix));
    }
    rules.push(format!("ip6tables -A {} -j DROP", chain));

    rules
}

/// Validate that a chain name is safe (no injection).
pub fn validate_chain_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Chain name cannot be empty".into());
    }
    if name.len() > 28 {
        return Err("Chain name too long (max 28 chars)".into());
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Chain name contains invalid characters".into());
    }
    Ok(())
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ----- IP and CIDR validation (SHROUD-VULN-021, VULN-022) -----

    #[test]
    fn test_is_valid_ipv4_accepts_valid() {
        assert!(is_valid_ipv4("1.1.1.1"));
        assert!(is_valid_ipv4("192.168.1.1"));
        assert!(is_valid_ipv4("255.255.255.255"));
    }

    #[test]
    fn test_is_valid_ipv4_rejects_injection() {
        assert!(!is_valid_ipv4("1.1.1.1 tcp dport 443 drop"));
        assert!(!is_valid_ipv4("1.1.1.1\n}\n}\n"));
        assert!(!is_valid_ipv4("not-an-ip"));
        assert!(!is_valid_ipv4(""));
    }

    #[test]
    fn test_is_valid_private_cidr_accepts_rfc1918() {
        assert!(is_valid_private_cidr("192.168.1.0/24"));
        assert!(is_valid_private_cidr("10.0.0.0/8"));
        assert!(is_valid_private_cidr("172.16.0.0/12"));
        assert!(is_valid_private_cidr("169.254.0.0/16"));
    }

    #[test]
    fn test_is_valid_private_cidr_rejects_public() {
        assert!(!is_valid_private_cidr("8.8.8.0/24")); // public
        assert!(!is_valid_private_cidr("0.0.0.0/0")); // too broad
        assert!(!is_valid_private_cidr("10.0.0.0/4")); // prefix < 8
        assert!(!is_valid_private_cidr("192.168.1.0/33")); // invalid prefix
        assert!(!is_valid_private_cidr("not-a-cidr"));
    }

    // ----- classify_ip -----

    mod classify_tests {
        use super::*;

        #[test]
        fn test_loopback() {
            let ip: IpAddr = "127.0.0.1".parse().unwrap();
            assert_eq!(classify_ip(&ip), IpClass::Loopback);
        }

        #[test]
        fn test_ipv6_loopback() {
            let ip: IpAddr = "::1".parse().unwrap();
            assert_eq!(classify_ip(&ip), IpClass::Loopback);
        }

        #[test]
        fn test_link_local() {
            let ip: IpAddr = "169.254.1.1".parse().unwrap();
            assert_eq!(classify_ip(&ip), IpClass::LinkLocal);
        }

        #[test]
        fn test_private_lan() {
            for addr in &["10.0.0.1", "172.16.0.1", "192.168.1.1"] {
                let ip: IpAddr = addr.parse().unwrap();
                assert_eq!(
                    classify_ip(&ip),
                    IpClass::PrivateLan,
                    "Expected {} to be PrivateLan",
                    addr
                );
            }
        }

        #[test]
        fn test_public() {
            for addr in &["8.8.8.8", "1.1.1.1", "203.0.113.50"] {
                let ip: IpAddr = addr.parse().unwrap();
                assert_eq!(
                    classify_ip(&ip),
                    IpClass::Public,
                    "Expected {} to be Public",
                    addr
                );
            }
        }

        #[test]
        fn test_ipv6_public() {
            let ip: IpAddr = "2001:db8::1".parse().unwrap();
            assert_eq!(classify_ip(&ip), IpClass::Public);
        }
    }

    // ----- is_doh_provider -----

    mod doh_tests {
        use super::*;

        #[test]
        fn test_known_providers() {
            assert!(is_doh_provider("1.1.1.1"));
            assert!(is_doh_provider("8.8.8.8"));
            assert!(is_doh_provider("9.9.9.9"));
            assert!(is_doh_provider("208.67.222.222"));
        }

        #[test]
        fn test_unknown_provider() {
            assert!(!is_doh_provider("192.168.1.1"));
            assert!(!is_doh_provider("10.0.0.1"));
            assert!(!is_doh_provider("203.0.113.50"));
        }
    }

    // ----- Rule building -----

    mod rule_building_tests {
        use super::*;

        const CHAIN: &str = "SHROUD_KILLSWITCH";

        #[test]
        fn test_server_allow_rule() {
            let ip: IpAddr = "203.0.113.50".parse().unwrap();
            let rule = build_server_allow_rule(&ip, CHAIN);
            assert!(rule.contains(CHAIN));
            assert!(rule.contains("203.0.113.50"));
            assert!(rule.contains("-j ACCEPT"));
        }

        #[test]
        fn test_loopback_rule() {
            let rule = build_loopback_rule(CHAIN);
            assert!(rule.contains("-o lo"));
            assert!(rule.contains("-j ACCEPT"));
        }

        #[test]
        fn test_lan_rules() {
            let rules = build_lan_rules(CHAIN);
            assert_eq!(rules.len(), LAN_SUBNETS.len());
            for rule in &rules {
                assert!(rule.contains("-j ACCEPT"));
                assert!(rule.contains(CHAIN));
            }
            // Specific subnets
            assert!(rules.iter().any(|r| r.contains("10.0.0.0/8")));
            assert!(rules.iter().any(|r| r.contains("192.168.0.0/16")));
            assert!(rules.iter().any(|r| r.contains("172.16.0.0/12")));
        }

        #[test]
        fn test_vpn_interface_rules() {
            let rules = build_vpn_interface_rules(CHAIN);
            assert!(rules.iter().any(|r| r.contains("tun+")));
            assert!(rules.iter().any(|r| r.contains("wg+")));
            assert!(rules.iter().any(|r| r.contains("tap+")));
        }

        #[test]
        fn test_default_drop_rule() {
            let rule = build_default_drop_rule(CHAIN);
            assert!(rule.contains("-j DROP"));
            assert!(rule.contains(CHAIN));
        }
    }

    // ----- DNS rules -----

    mod dns_rule_tests {
        use super::*;

        const CHAIN: &str = "SHROUD_KILLSWITCH";

        #[test]
        fn test_tunnel_mode_allows_vpn_dns() {
            let rules = build_dns_tunnel_rules(CHAIN);
            // Should allow DNS on each VPN interface
            assert!(rules
                .iter()
                .any(|r| r.contains("tun+") && r.contains("udp") && r.contains("ACCEPT")));
            assert!(rules
                .iter()
                .any(|r| r.contains("wg+") && r.contains("udp") && r.contains("ACCEPT")));
        }

        #[test]
        fn test_tunnel_mode_blocks_other_dns() {
            let rules = build_dns_tunnel_rules(CHAIN);
            // Should have DROP rules for port 53
            assert!(rules
                .iter()
                .any(|r| r.contains("--dport 53") && r.contains("DROP")));
            // Should block DNS-over-TLS
            assert!(rules
                .iter()
                .any(|r| r.contains("--dport 853") && r.contains("DROP")));
        }

        #[test]
        fn test_tunnel_mode_accept_before_drop() {
            let rules = build_dns_tunnel_rules(CHAIN);
            // ACCEPT rules should come before DROP rules
            let first_accept = rules.iter().position(|r| r.contains("ACCEPT")).unwrap();
            let first_drop = rules.iter().position(|r| r.contains("DROP")).unwrap();
            assert!(
                first_accept < first_drop,
                "ACCEPT rules must come before DROP rules"
            );
        }

        #[test]
        fn test_localhost_mode_allows_loopback_dns() {
            let rules = build_dns_localhost_rules(CHAIN);
            assert!(rules
                .iter()
                .any(|r| r.contains("127.0.0.1") && r.contains("ACCEPT")));
        }

        #[test]
        fn test_localhost_mode_blocks_other_dns() {
            let rules = build_dns_localhost_rules(CHAIN);
            assert!(rules
                .iter()
                .any(|r| r.contains("--dport 53") && r.contains("DROP")));
            assert!(rules
                .iter()
                .any(|r| r.contains("--dport 853") && r.contains("DROP")));
        }

        #[test]
        fn test_any_mode_accepts_all_dns() {
            let rules = build_dns_any_rules(CHAIN);
            assert!(rules.iter().all(|r| r.contains("ACCEPT")));
            assert!(!rules.iter().any(|r| r.contains("DROP")));
        }

        #[test]
        fn test_doh_blocking() {
            let rules = build_doh_blocking_rules(CHAIN);
            assert_eq!(rules.len(), DOH_PROVIDERS.len());
            for rule in &rules {
                assert!(rule.contains("--dport 443"));
                assert!(rule.contains("-j DROP"));
            }
            assert!(rules.iter().any(|r| r.contains("1.1.1.1")));
            assert!(rules.iter().any(|r| r.contains("8.8.8.8")));
        }

        #[test]
        fn test_custom_doh_blocking() {
            let custom = vec!["100.100.100.100".into(), "200.200.200.200".into()];
            let rules = build_custom_doh_blocking_rules(CHAIN, &custom);
            assert_eq!(rules.len(), 2);
            assert!(rules[0].contains("100.100.100.100"));
            assert!(rules[1].contains("200.200.200.200"));
        }

        #[test]
        fn test_custom_doh_blocking_empty() {
            let rules = build_custom_doh_blocking_rules(CHAIN, &[]);
            assert!(rules.is_empty());
        }
    }

    // ----- IPv6 rules -----

    mod ipv6_rule_tests {
        use super::*;

        const CHAIN: &str = "SHROUD_KILLSWITCH";

        #[test]
        fn test_ipv6_block_allows_loopback() {
            let rules = build_ipv6_block_rules(CHAIN);
            assert!(rules[0].contains("-o lo") && rules[0].contains("ACCEPT"));
        }

        #[test]
        fn test_ipv6_block_drops_rest() {
            let rules = build_ipv6_block_rules(CHAIN);
            assert!(rules.last().unwrap().contains("-j DROP"));
        }

        #[test]
        fn test_ipv6_tunnel_allows_vpn() {
            let rules = build_ipv6_tunnel_rules(CHAIN);
            assert!(rules.iter().any(|r| r.contains("tun+")));
            assert!(rules.iter().any(|r| r.contains("wg+")));
        }

        #[test]
        fn test_ipv6_tunnel_drops_rest() {
            let rules = build_ipv6_tunnel_rules(CHAIN);
            assert!(rules.last().unwrap().contains("-j DROP"));
        }

        #[test]
        fn test_ipv6_rules_use_ip6tables() {
            let rules = build_ipv6_block_rules(CHAIN);
            for rule in &rules {
                assert!(
                    rule.starts_with("ip6tables"),
                    "IPv6 rules must use ip6tables"
                );
            }
        }
    }

    // ----- Chain name validation -----

    mod chain_validation_tests {
        use super::*;

        #[test]
        fn test_valid_chain_names() {
            assert!(validate_chain_name("SHROUD_KILLSWITCH").is_ok());
            assert!(validate_chain_name("MY-CHAIN").is_ok());
            assert!(validate_chain_name("chain1").is_ok());
        }

        #[test]
        fn test_empty_chain() {
            assert!(validate_chain_name("").is_err());
        }

        #[test]
        fn test_too_long_chain() {
            let long = "a".repeat(29);
            assert!(validate_chain_name(&long).is_err());
        }

        #[test]
        fn test_chain_injection() {
            assert!(validate_chain_name("chain; rm -rf /").is_err());
            assert!(validate_chain_name("chain$(id)").is_err());
            assert!(validate_chain_name("chain`id`").is_err());
            assert!(validate_chain_name("chain name").is_err());
        }

        #[test]
        fn test_max_length_chain() {
            let max = "a".repeat(28);
            assert!(validate_chain_name(&max).is_ok());
        }
    }

    // ----- Constants tests -----

    mod constants_tests {
        use super::*;

        #[test]
        fn test_lan_subnets_are_private() {
            for subnet in LAN_SUBNETS {
                let parts: Vec<&str> = subnet.split('/').collect();
                let ip: IpAddr = parts[0].parse().unwrap();
                let class = classify_ip(&ip);
                assert!(
                    matches!(class, IpClass::PrivateLan | IpClass::LinkLocal),
                    "LAN subnet {} should classify as private or link-local",
                    subnet
                );
            }
        }

        #[test]
        fn test_doh_providers_are_public() {
            for ip_str in DOH_PROVIDERS {
                let ip: IpAddr = ip_str.parse().unwrap();
                assert_eq!(
                    classify_ip(&ip),
                    IpClass::Public,
                    "DoH provider {} should be public",
                    ip_str
                );
            }
        }

        #[test]
        fn test_vpn_prefixes_non_empty() {
            assert!(!VPN_INTERFACE_PREFIXES.is_empty());
            for prefix in VPN_INTERFACE_PREFIXES {
                assert!(!prefix.is_empty());
            }
        }
    }
}
