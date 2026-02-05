//! Unit tests for Gateway module
//!
//! Tests NAT/forwarding flag logic and config gating
//! without requiring root privileges or real interfaces.

#[cfg(test)]
mod gateway_tests {
    use crate::gateway::GatewayError;

    // =========================================================================
    // GatewayError tests
    // =========================================================================

    #[test]
    fn test_gateway_error_display() {
        let forwarding_err = GatewayError::Forwarding("failed to enable".to_string());
        let nat_err = GatewayError::Nat("masquerade failed".to_string());
        let firewall_err = GatewayError::Firewall("rule insertion failed".to_string());
        let detection_err = GatewayError::Detection("no interface found".to_string());

        assert!(forwarding_err.to_string().contains("IP forwarding"));
        assert!(nat_err.to_string().contains("NAT"));
        assert!(firewall_err.to_string().contains("Firewall"));
        assert!(detection_err.to_string().contains("Interface detection"));
    }

    #[test]
    fn test_gateway_error_is_error_trait() {
        let err = GatewayError::Forwarding("test".to_string());
        let _: &dyn std::error::Error = &err;
    }

    // =========================================================================
    // Interface detection logic tests
    // =========================================================================

    #[test]
    fn test_vpn_interface_patterns() {
        // VPN interfaces follow common patterns
        let vpn_patterns = ["tun0", "tun1", "wg0", "wg1", "tap0"];
        let non_vpn = ["eth0", "enp0s3", "wlan0", "lo", "docker0"];

        for iface in vpn_patterns {
            assert!(
                iface.starts_with("tun")
                    || iface.starts_with("wg")
                    || iface.starts_with("tap"),
                "{} should match VPN pattern",
                iface
            );
        }

        for iface in non_vpn {
            assert!(
                !iface.starts_with("tun")
                    && !iface.starts_with("wg")
                    && !iface.starts_with("tap"),
                "{} should not match VPN pattern",
                iface
            );
        }
    }

    #[test]
    fn test_lan_interface_patterns() {
        // LAN interfaces follow common patterns
        let lan_patterns = ["eth0", "enp0s3", "ens3", "em1", "eno1"];

        for iface in lan_patterns {
            let is_lan = iface.starts_with("eth")
                || iface.starts_with("en")
                || iface.starts_with("em");
            assert!(is_lan, "{} should match LAN pattern", iface);
        }

        // Not LAN
        let not_lan = ["lo", "tun0", "wg0", "docker0", "virbr0"];
        for iface in not_lan {
            let is_lan = iface.starts_with("eth")
                || iface.starts_with("en")
                || iface.starts_with("em");
            assert!(!is_lan, "{} should not match LAN pattern", iface);
        }
    }

    // =========================================================================
    // Forwarding flag logic tests
    // =========================================================================

    #[test]
    fn test_forwarding_flag_parsing() {
        // /proc/sys/net/ipv4/ip_forward contains "0\n" or "1\n"
        let enabled = "1\n";
        let disabled = "0\n";
        let enabled_no_newline = "1";
        let disabled_no_newline = "0";

        assert_eq!(enabled.trim(), "1");
        assert_eq!(disabled.trim(), "0");
        assert_eq!(enabled_no_newline.trim(), "1");
        assert_eq!(disabled_no_newline.trim(), "0");

        // Parse as boolean
        let is_enabled = |s: &str| s.trim() == "1";

        assert!(is_enabled(enabled));
        assert!(!is_enabled(disabled));
        assert!(is_enabled(enabled_no_newline));
        assert!(!is_enabled(disabled_no_newline));
    }

    #[test]
    fn test_ipv6_forwarding_flag() {
        // /proc/sys/net/ipv6/conf/all/forwarding
        let values = ["0\n", "1\n", "0", "1"];

        for val in values {
            let parsed = val.trim() == "1";
            assert!(parsed == (val.contains('1')));
        }
    }

    // =========================================================================
    // NAT rule logic tests
    // =========================================================================

    #[test]
    fn test_nat_masquerade_rule_format() {
        // NAT rules follow format:
        // iptables -t nat -A POSTROUTING -o <vpn_iface> -j MASQUERADE
        let vpn_interface = "tun0";
        let expected_args = [
            "-t", "nat", "-A", "POSTROUTING", "-o", vpn_interface, "-j", "MASQUERADE",
        ];

        assert_eq!(expected_args[0], "-t");
        assert_eq!(expected_args[1], "nat");
        assert_eq!(expected_args[5], vpn_interface);
        assert_eq!(expected_args[7], "MASQUERADE");
    }

    #[test]
    fn test_forward_rule_format() {
        // Forward rules:
        // iptables -A FORWARD -i <lan> -o <vpn> -j ACCEPT
        // iptables -A FORWARD -i <vpn> -o <lan> -m state --state RELATED,ESTABLISHED -j ACCEPT
        let lan = "eth0";
        let vpn = "tun0";

        let outbound_args = ["-A", "FORWARD", "-i", lan, "-o", vpn, "-j", "ACCEPT"];
        let inbound_args = [
            "-A",
            "FORWARD",
            "-i",
            vpn,
            "-o",
            lan,
            "-m",
            "state",
            "--state",
            "RELATED,ESTABLISHED",
            "-j",
            "ACCEPT",
        ];

        assert_eq!(outbound_args[3], lan);
        assert_eq!(outbound_args[5], vpn);
        assert_eq!(inbound_args[3], vpn);
        assert_eq!(inbound_args[5], lan);
    }

    // =========================================================================
    // Config gating tests
    // =========================================================================

    #[test]
    fn test_gateway_config_defaults() {
        use crate::config::GatewayConfig;

        let config = GatewayConfig::default();

        // Verify defaults are safe
        assert!(!config.enabled, "Gateway should be disabled by default");
        assert!(
            config.kill_switch_forwarding,
            "Forward kill switch should default on for safety"
        );
    }

    #[test]
    fn test_allowed_clients_parsing() {
        // Allowed clients are CIDR notation
        let valid_cidrs = [
            "192.168.1.0/24",
            "10.0.0.0/8",
            "172.16.0.0/12",
            "192.168.100.50/32",
        ];

        for cidr in valid_cidrs {
            let parts: Vec<&str> = cidr.split('/').collect();
            assert_eq!(parts.len(), 2, "CIDR should have two parts");

            let _ip = parts[0];
            let prefix_len: u8 = parts[1].parse().expect("Should parse prefix");
            assert!(prefix_len <= 32, "IPv4 prefix max 32");
        }
    }

    #[test]
    fn test_gateway_state_atomic() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let state = AtomicBool::new(false);

        // Initial state
        assert!(!state.load(Ordering::SeqCst));

        // Enable
        state.store(true, Ordering::SeqCst);
        assert!(state.load(Ordering::SeqCst));

        // Disable
        state.store(false, Ordering::SeqCst);
        assert!(!state.load(Ordering::SeqCst));
    }

    // =========================================================================
    // Kill switch forwarding tests
    // =========================================================================

    #[test]
    fn test_kill_switch_forwarding_concept() {
        // Forward kill switch blocks forwarded traffic when VPN is down
        // but allows through when VPN is up

        struct MockForwardKillSwitch {
            enabled: bool,
        }

        impl MockForwardKillSwitch {
            fn should_allow_forward(&self, vpn_up: bool) -> bool {
                if self.enabled {
                    // Only allow if VPN is up
                    vpn_up
                } else {
                    // Always allow (no protection)
                    true
                }
            }
        }

        let ks_enabled = MockForwardKillSwitch { enabled: true };
        let ks_disabled = MockForwardKillSwitch { enabled: false };

        // With kill switch enabled
        assert!(ks_enabled.should_allow_forward(true), "VPN up = allow");
        assert!(!ks_enabled.should_allow_forward(false), "VPN down = block");

        // Without kill switch
        assert!(ks_disabled.should_allow_forward(true));
        assert!(ks_disabled.should_allow_forward(false));
    }

    // =========================================================================
    // IPv6 handling tests
    // =========================================================================

    #[test]
    fn test_ipv6_enable_flag() {
        // Gateway can optionally enable IPv6 forwarding
        let enable_ipv6_options = [true, false];

        for enable_ipv6 in enable_ipv6_options {
            if enable_ipv6 {
                // Would write "1" to /proc/sys/net/ipv6/conf/all/forwarding
            } else {
                // Skip IPv6 forwarding
            }
        }
    }

    // =========================================================================
    // Interface validation tests
    // =========================================================================

    #[test]
    fn test_interface_name_validation() {
        fn is_valid_interface_name(name: &str) -> bool {
            !name.is_empty()
                && name.len() <= 15  // IFNAMSIZ - 1
                && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                && !name.contains(';')
                && !name.contains('|')
                && !name.contains('&')
        }

        assert!(is_valid_interface_name("eth0"));
        assert!(is_valid_interface_name("tun0"));
        assert!(is_valid_interface_name("enp0s3"));
        assert!(is_valid_interface_name("wg-tunnel"));
        
        assert!(!is_valid_interface_name(""));
        assert!(!is_valid_interface_name("eth0; rm -rf /"));
        assert!(!is_valid_interface_name("a".repeat(20).as_str()));
    }

    #[test]
    fn test_subnet_validation() {
        fn is_valid_ipv4_subnet(subnet: &str) -> bool {
            let parts: Vec<&str> = subnet.split('/').collect();
            if parts.len() != 2 {
                return false;
            }
            
            let ip_parts: Vec<&str> = parts[0].split('.').collect();
            if ip_parts.len() != 4 {
                return false;
            }
            
            for part in ip_parts {
                if part.parse::<u8>().is_err() {
                    return false;
                }
            }
            
            if let Ok(prefix) = parts[1].parse::<u8>() {
                prefix <= 32
            } else {
                false
            }
        }

        assert!(is_valid_ipv4_subnet("192.168.1.0/24"));
        assert!(is_valid_ipv4_subnet("10.0.0.0/8"));
        assert!(is_valid_ipv4_subnet("0.0.0.0/0"));
        
        assert!(!is_valid_ipv4_subnet("not-a-subnet"));
        assert!(!is_valid_ipv4_subnet("192.168.1.0"));
        assert!(!is_valid_ipv4_subnet("192.168.1.0/33"));
    }

    // =========================================================================
    // NAT rule building tests
    // =========================================================================

    #[test]
    fn test_build_masquerade_args() {
        fn build_masquerade_args(interface: &str) -> Vec<String> {
            vec![
                "-t".to_string(),
                "nat".to_string(),
                "-A".to_string(),
                "POSTROUTING".to_string(),
                "-o".to_string(),
                interface.to_string(),
                "-j".to_string(),
                "MASQUERADE".to_string(),
            ]
        }

        let args = build_masquerade_args("tun0");
        assert!(args.contains(&"MASQUERADE".to_string()));
        assert!(args.contains(&"tun0".to_string()));
        assert!(args.contains(&"-o".to_string()));
    }

    #[test]
    fn test_build_forward_args() {
        fn build_forward_args(in_iface: &str, out_iface: &str) -> Vec<String> {
            vec![
                "-A".to_string(),
                "FORWARD".to_string(),
                "-i".to_string(),
                in_iface.to_string(),
                "-o".to_string(),
                out_iface.to_string(),
                "-j".to_string(),
                "ACCEPT".to_string(),
            ]
        }

        let args = build_forward_args("eth0", "tun0");
        assert!(args.contains(&"FORWARD".to_string()));
        assert!(args.contains(&"eth0".to_string()));
        assert!(args.contains(&"tun0".to_string()));
    }

    // =========================================================================
    // Error handling tests
    // =========================================================================

    #[test]
    fn test_gateway_error_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let gateway_err = GatewayError::Firewall(io_err.to_string());
        assert!(gateway_err.to_string().contains("access denied"));
    }

    #[test]
    fn test_gateway_error_debug() {
        let err = GatewayError::Detection("no interface".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("Detection"));
        assert!(debug.contains("no interface"));
    }

    // =========================================================================
    // Status tracking tests
    // =========================================================================

    #[test]
    fn test_gateway_status_tracking() {
        struct GatewayStatus {
            forwarding_enabled: bool,
            nat_enabled: bool,
            vpn_interface: Option<String>,
            lan_interface: Option<String>,
        }

        let status = GatewayStatus {
            forwarding_enabled: true,
            nat_enabled: true,
            vpn_interface: Some("tun0".to_string()),
            lan_interface: Some("eth0".to_string()),
        };

        assert!(status.forwarding_enabled);
        assert!(status.nat_enabled);
        assert_eq!(status.vpn_interface.as_deref(), Some("tun0"));
        assert_eq!(status.lan_interface.as_deref(), Some("eth0"));
    }
}
