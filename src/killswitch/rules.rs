//! Kill switch rule generation — pure functions, easily testable.
//!
//! Contains pure-logic helpers for building firewall rule strings,
//! validating DNS modes, and classifying network traffic.
//! No system calls, no I/O.

use std::net::IpAddr;

/// LAN subnets that should always be allowed (RFC 1918 + link-local).
pub const LAN_SUBNETS: &[&str] = &[
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16", // Link-local
];

/// Well-known DoH provider IPs to block.
pub const DOH_PROVIDERS: &[&str] = &[
    "1.1.1.1",         // Cloudflare
    "1.0.0.1",         // Cloudflare
    "8.8.8.8",         // Google
    "8.8.4.4",         // Google
    "9.9.9.9",         // Quad9
    "149.112.112.112", // Quad9
    "208.67.222.222",  // OpenDNS
    "208.67.220.220",  // OpenDNS
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

/// Build LAN allow rules.
pub fn build_lan_rules(chain: &str) -> Vec<String> {
    LAN_SUBNETS
        .iter()
        .map(|subnet| format!("iptables -A {} -d {} -j ACCEPT", chain, subnet))
        .collect()
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

/// Build DNS localhost-mode rules (allow DNS only to 127.0.0.0/8).
pub fn build_dns_localhost_rules(chain: &str) -> Vec<String> {
    let mut rules = Vec::new();

    rules.push(format!(
        "iptables -A {} -d 127.0.0.0/8 -p udp --dport 53 -j ACCEPT",
        chain
    ));
    rules.push(format!(
        "iptables -A {} -d 127.0.0.0/8 -p tcp --dport 53 -j ACCEPT",
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
                .any(|r| r.contains("127.0.0.0/8") && r.contains("ACCEPT")));
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
