//! Gateway configuration validation — pure functions, easily testable.
//!
//! Validates interface names, subnets, and parses route output
//! without invoking any system commands.

use std::net::IpAddr;
use std::str::FromStr;

/// Validate a network interface name.
///
/// Linux interface names must be ≤ 15 chars, start with a letter,
/// and contain only alphanumeric, dash, or underscore characters.
pub fn validate_interface_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Interface name cannot be empty".into());
    }

    if name.len() > 15 {
        return Err("Interface name too long (max 15 chars)".into());
    }

    if !name
        .chars()
        .next()
        .map(|c| c.is_alphabetic())
        .unwrap_or(false)
    {
        return Err("Interface name must start with a letter".into());
    }

    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Interface name contains invalid characters".into());
    }

    Ok(())
}

/// Validate a subnet in CIDR notation (e.g. `192.168.1.0/24`).
pub fn validate_subnet(subnet: &str) -> Result<(IpAddr, u8), String> {
    let parts: Vec<&str> = subnet.split('/').collect();

    if parts.len() != 2 {
        return Err("Invalid subnet format (expected IP/prefix)".into());
    }

    let ip = IpAddr::from_str(parts[0]).map_err(|e| format!("Invalid IP address: {}", e))?;

    let prefix: u8 = parts[1]
        .parse()
        .map_err(|_| "Invalid prefix length".to_string())?;

    let max_prefix = if ip.is_ipv4() { 32 } else { 128 };
    if prefix > max_prefix {
        return Err(format!(
            "Prefix length {} too large for {}",
            prefix,
            if ip.is_ipv4() { "IPv4" } else { "IPv6" }
        ));
    }

    Ok((ip, prefix))
}

/// Check if an interface name looks like a VPN tunnel.
pub fn is_vpn_interface(name: &str) -> bool {
    let vpn_prefixes = [
        "tun", "tap", "wg", "ppp", "vpn", "proton", "mullvad", "nordlynx",
    ];
    vpn_prefixes.iter().any(|p| name.starts_with(p))
}

/// Check if an interface name looks like a physical NIC.
pub fn is_physical_interface(name: &str) -> bool {
    let physical_prefixes = ["eth", "enp", "ens", "eno", "wlan", "wlp"];
    physical_prefixes.iter().any(|p| name.starts_with(p))
}

/// Parse `ip route` output to find the default-route interface.
pub fn parse_default_interface(route_output: &str) -> Option<String> {
    for line in route_output.lines() {
        if !line.starts_with("default") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        for (i, part) in parts.iter().enumerate() {
            if *part == "dev" && i + 1 < parts.len() {
                return Some(parts[i + 1].to_string());
            }
        }
    }
    None
}

/// Parse `ip route` output to find the default-route gateway IP.
pub fn parse_default_gateway(route_output: &str) -> Option<String> {
    for line in route_output.lines() {
        if !line.starts_with("default") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        for (i, part) in parts.iter().enumerate() {
            if *part == "via" && i + 1 < parts.len() {
                return Some(parts[i + 1].to_string());
            }
        }
    }
    None
}

/// Build an iptables MASQUERADE rule string.
pub fn build_masquerade_rule(out_interface: &str) -> String {
    format!("-t nat -A POSTROUTING -o {} -j MASQUERADE", out_interface)
}

/// Build an iptables FORWARD rule string.
pub fn build_forward_rule(in_iface: &str, out_iface: &str, action: &str) -> String {
    format!("-A FORWARD -i {} -o {} -j {}", in_iface, out_iface, action)
}

/// Build iptables FORWARD rules for a list of allowed source IPs.
pub fn build_client_rules(in_iface: &str, out_iface: &str, allowed_ips: &[String]) -> Vec<String> {
    allowed_ips
        .iter()
        .map(|ip| {
            format!(
                "-A FORWARD -i {} -o {} -s {} -j ACCEPT",
                in_iface, out_iface, ip
            )
        })
        .collect()
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ----- validate_interface_name -----

    mod interface_name_tests {
        use super::*;

        #[test]
        fn test_valid_names() {
            assert!(validate_interface_name("eth0").is_ok());
            assert!(validate_interface_name("wlan0").is_ok());
            assert!(validate_interface_name("enp3s0").is_ok());
            assert!(validate_interface_name("tun0").is_ok());
            assert!(validate_interface_name("wg-server").is_ok());
            assert!(validate_interface_name("vpn_tunnel").is_ok());
            assert!(validate_interface_name("a").is_ok());
        }

        #[test]
        fn test_empty_name() {
            assert!(validate_interface_name("").is_err());
        }

        #[test]
        fn test_too_long() {
            let name = "a".repeat(16);
            assert!(validate_interface_name(&name).is_err());

            let max = "a".repeat(15);
            assert!(validate_interface_name(&max).is_ok());
        }

        #[test]
        fn test_starts_with_number() {
            assert!(validate_interface_name("0eth").is_err());
            assert!(validate_interface_name("9tun").is_err());
        }

        #[test]
        fn test_shell_injection() {
            assert!(validate_interface_name("eth0;rm").is_err());
            assert!(validate_interface_name("eth0`id`").is_err());
            assert!(validate_interface_name("eth0$(id)").is_err());
            assert!(validate_interface_name("eth0|cat").is_err());
            assert!(validate_interface_name("eth0 x").is_err());
        }

        #[test]
        fn test_special_chars() {
            assert!(validate_interface_name("eth.0").is_err());
            assert!(validate_interface_name("eth:0").is_err());
            assert!(validate_interface_name("eth/0").is_err());
        }
    }

    // ----- validate_subnet -----

    mod subnet_tests {
        use super::*;

        #[test]
        fn test_valid_ipv4_subnets() {
            assert!(validate_subnet("192.168.1.0/24").is_ok());
            assert!(validate_subnet("10.0.0.0/8").is_ok());
            assert!(validate_subnet("172.16.0.0/16").is_ok());
            assert!(validate_subnet("0.0.0.0/0").is_ok());
            assert!(validate_subnet("255.255.255.255/32").is_ok());
        }

        #[test]
        fn test_valid_ipv6_subnets() {
            assert!(validate_subnet("::1/128").is_ok());
            assert!(validate_subnet("fe80::/10").is_ok());
            assert!(validate_subnet("2001:db8::/32").is_ok());
        }

        #[test]
        fn test_returns_correct_values() {
            let (ip, prefix) = validate_subnet("192.168.1.0/24").unwrap();
            assert!(ip.is_ipv4());
            assert_eq!(prefix, 24);
        }

        #[test]
        fn test_invalid_format() {
            assert!(validate_subnet("192.168.1.0").is_err());
            assert!(validate_subnet("/24").is_err());
            assert!(validate_subnet("192.168.1.0/24/extra").is_err());
        }

        #[test]
        fn test_invalid_ip() {
            assert!(validate_subnet("999.999.999.999/24").is_err());
            assert!(validate_subnet("not-an-ip/24").is_err());
        }

        #[test]
        fn test_invalid_prefix() {
            assert!(validate_subnet("192.168.1.0/33").is_err());
            assert!(validate_subnet("::1/129").is_err());
            assert!(validate_subnet("192.168.1.0/abc").is_err());
        }
    }

    // ----- is_vpn_interface / is_physical_interface -----

    mod interface_type_tests {
        use super::*;

        #[test]
        fn test_vpn_interfaces() {
            assert!(is_vpn_interface("tun0"));
            assert!(is_vpn_interface("tun99"));
            assert!(is_vpn_interface("tap0"));
            assert!(is_vpn_interface("wg0"));
            assert!(is_vpn_interface("wg-server"));
            assert!(is_vpn_interface("ppp0"));
            assert!(is_vpn_interface("proton0"));
            assert!(is_vpn_interface("mullvad-wg"));
            assert!(is_vpn_interface("nordlynx"));
        }

        #[test]
        fn test_non_vpn_interfaces() {
            assert!(!is_vpn_interface("eth0"));
            assert!(!is_vpn_interface("enp3s0"));
            assert!(!is_vpn_interface("wlan0"));
            assert!(!is_vpn_interface("lo"));
            assert!(!is_vpn_interface("docker0"));
            assert!(!is_vpn_interface("br0"));
        }

        #[test]
        fn test_physical_interfaces() {
            assert!(is_physical_interface("eth0"));
            assert!(is_physical_interface("enp3s0"));
            assert!(is_physical_interface("ens33"));
            assert!(is_physical_interface("eno1"));
            assert!(is_physical_interface("wlan0"));
            assert!(is_physical_interface("wlp2s0"));
        }

        #[test]
        fn test_non_physical_interfaces() {
            assert!(!is_physical_interface("tun0"));
            assert!(!is_physical_interface("lo"));
            assert!(!is_physical_interface("docker0"));
            assert!(!is_physical_interface("br0"));
            assert!(!is_physical_interface("virbr0"));
        }
    }

    // ----- parse_default_interface / parse_default_gateway -----

    mod route_parsing_tests {
        use super::*;

        #[test]
        fn test_parse_simple_route() {
            let output = "default via 192.168.1.1 dev eth0 proto static metric 100";
            assert_eq!(parse_default_interface(output), Some("eth0".into()));
            assert_eq!(parse_default_gateway(output), Some("192.168.1.1".into()));
        }

        #[test]
        fn test_parse_multiple_routes() {
            let output = "\
default via 192.168.1.1 dev eth0 proto static metric 100
10.0.0.0/8 via 10.0.0.1 dev wg0
192.168.0.0/16 dev eth0 proto kernel scope link src 192.168.1.100";
            assert_eq!(parse_default_interface(output), Some("eth0".into()));
        }

        #[test]
        fn test_no_default_route() {
            let output = "10.0.0.0/8 via 10.0.0.1 dev wg0";
            assert_eq!(parse_default_interface(output), None);
            assert_eq!(parse_default_gateway(output), None);
        }

        #[test]
        fn test_empty_output() {
            assert_eq!(parse_default_interface(""), None);
            assert_eq!(parse_default_gateway(""), None);
        }

        #[test]
        fn test_default_without_dev() {
            let output = "default via 10.0.0.1";
            assert_eq!(parse_default_interface(output), None);
            assert_eq!(parse_default_gateway(output), Some("10.0.0.1".into()));
        }
    }

    // ----- build rules -----

    mod build_rules_tests {
        use super::*;

        #[test]
        fn test_masquerade_rule() {
            let rule = build_masquerade_rule("eth0");
            assert!(rule.contains("-t nat"));
            assert!(rule.contains("-o eth0"));
            assert!(rule.contains("MASQUERADE"));
        }

        #[test]
        fn test_forward_rule() {
            let rule = build_forward_rule("eth0", "tun0", "ACCEPT");
            assert!(rule.contains("-i eth0"));
            assert!(rule.contains("-o tun0"));
            assert!(rule.contains("-j ACCEPT"));
        }

        #[test]
        fn test_forward_rule_drop() {
            let rule = build_forward_rule("eth0", "tun0", "DROP");
            assert!(rule.contains("-j DROP"));
        }

        #[test]
        fn test_client_rules_empty() {
            let rules = build_client_rules("eth0", "tun0", &[]);
            assert!(rules.is_empty());
        }

        #[test]
        fn test_client_rules_multiple() {
            let ips = vec!["192.168.1.10".into(), "192.168.1.20".into()];
            let rules = build_client_rules("eth0", "tun0", &ips);
            assert_eq!(rules.len(), 2);
            assert!(rules[0].contains("-s 192.168.1.10"));
            assert!(rules[1].contains("-s 192.168.1.20"));
            assert!(rules[0].contains("-i eth0"));
            assert!(rules[0].contains("-o tun0"));
        }
    }
}
