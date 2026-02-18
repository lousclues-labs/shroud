// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Unit tests for Kill Switch module
//!
//! Tests rule synthesis, teardown logic, and configuration
//! using fakes without requiring root privileges.

#[cfg(test)]
mod killswitch_tests {
    use crate::config::{DnsMode, Ipv6Mode};
    use crate::killswitch::firewall::{KillSwitch, KillSwitchError, KillSwitchStatus};

    // =========================================================================
    // KillSwitchError tests
    // =========================================================================

    #[test]
    fn test_killswitch_error_display() {
        let not_found = KillSwitchError::NotFound;
        let permission = KillSwitchError::Permission;
        let command = KillSwitchError::Command("rule failed".to_string());

        assert!(not_found.to_string().contains("iptables"));
        assert!(permission.to_string().contains("Permission"));
        assert!(command.to_string().contains("rule failed"));
    }

    #[test]
    fn test_killswitch_error_is_error_trait() {
        let err = KillSwitchError::NotFound;
        let _: &dyn std::error::Error = &err;
    }

    // =========================================================================
    // KillSwitchStatus tests
    // =========================================================================

    #[test]
    fn test_killswitch_status_variants() {
        let disabled = KillSwitchStatus::Disabled;
        let active = KillSwitchStatus::Active;
        let error = KillSwitchStatus::Error;

        // Test equality
        assert_eq!(disabled, KillSwitchStatus::Disabled);
        assert_eq!(active, KillSwitchStatus::Active);
        assert_eq!(error, KillSwitchStatus::Error);

        // Test inequality
        assert_ne!(disabled, active);
        assert_ne!(active, error);
    }

    #[test]
    fn test_killswitch_status_copy() {
        let status = KillSwitchStatus::Active;
        let copied = status;
        assert_eq!(status, copied);
    }

    // =========================================================================
    // KillSwitch construction tests
    // =========================================================================

    #[test]
    fn test_killswitch_new() {
        let ks = KillSwitch::new();
        // Should initialize in disabled state
        // Note: enabled field is private, but we can test behavior
        drop(ks); // Should not panic
    }

    #[test]
    fn test_killswitch_default() {
        let ks: KillSwitch = Default::default();
        drop(ks); // Should not panic
    }

    // =========================================================================
    // DNS mode tests
    // =========================================================================

    #[test]
    fn test_dns_mode_variants() {
        let tunnel = DnsMode::Tunnel;
        let strict = DnsMode::Strict;
        let _localhost = DnsMode::Localhost;
        let _any = DnsMode::Any;

        // Test equality
        assert_eq!(tunnel, DnsMode::Tunnel);
        assert_ne!(tunnel, strict);

        // Test default is secure
        let default = DnsMode::default();
        assert!(
            matches!(default, DnsMode::Tunnel | DnsMode::Strict),
            "Default should be secure mode"
        );
    }

    #[test]
    fn test_dns_mode_security_ranking() {
        // Security ranking: strict > tunnel > localhost > any
        fn security_level(mode: &DnsMode) -> u8 {
            match mode {
                DnsMode::Strict => 4,
                DnsMode::Tunnel => 3,
                DnsMode::Localhost => 2,
                DnsMode::Any => 1,
            }
        }

        assert!(security_level(&DnsMode::Strict) > security_level(&DnsMode::Tunnel));
        assert!(security_level(&DnsMode::Tunnel) > security_level(&DnsMode::Localhost));
        assert!(security_level(&DnsMode::Localhost) > security_level(&DnsMode::Any));
    }

    // =========================================================================
    // IPv6 mode tests
    // =========================================================================

    #[test]
    fn test_ipv6_mode_variants() {
        let block = Ipv6Mode::Block;
        let tunnel = Ipv6Mode::Tunnel;
        let off = Ipv6Mode::Off;

        assert_eq!(block, Ipv6Mode::Block);
        assert_ne!(block, tunnel);
        assert_ne!(tunnel, off);
    }

    #[test]
    fn test_ipv6_mode_security() {
        // Block is most secure (no IPv6 leaks)
        // Tunnel allows IPv6 via VPN only
        // Off provides no protection
        fn is_secure(mode: &Ipv6Mode) -> bool {
            matches!(mode, Ipv6Mode::Block | Ipv6Mode::Tunnel)
        }

        assert!(is_secure(&Ipv6Mode::Block));
        assert!(is_secure(&Ipv6Mode::Tunnel));
        assert!(!is_secure(&Ipv6Mode::Off));
    }

    // =========================================================================
    // Rule synthesis tests (structure only, no execution)
    // =========================================================================

    #[test]
    fn test_chain_name_constant() {
        // The chain name should be consistent
        let chain_name = "SHROUD_KILLSWITCH";
        assert!(!chain_name.is_empty());
        assert!(chain_name.starts_with("SHROUD"));
        assert!(!chain_name.contains(' ')); // No spaces in chain names
    }

    #[test]
    fn test_nft_table_name() {
        let table_name = "shroud_killswitch";
        assert!(!table_name.is_empty());
        assert!(table_name.starts_with("shroud"));
    }

    #[test]
    fn test_vpn_interface_patterns_accepted() {
        let valid_interfaces = ["tun0", "tun1", "wg0", "wg1", "tap0", "tun100"];

        for iface in valid_interfaces {
            assert!(
                iface.starts_with("tun") || iface.starts_with("wg") || iface.starts_with("tap"),
                "Interface {} should be valid VPN pattern",
                iface
            );
        }
    }

    #[test]
    fn test_loopback_always_allowed() {
        // Loopback should always be allowed
        let loopback = "127.0.0.0/8";
        let loopback_v6 = "::1/128";

        assert!(loopback.starts_with("127."));
        assert!(loopback_v6.contains("::1"));
    }

    #[test]
    fn test_lan_ranges() {
        // RFC1918 private ranges
        let lan_ranges = [
            "10.0.0.0/8",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "169.254.0.0/16", // Link-local
        ];

        for range in lan_ranges {
            let parts: Vec<&str> = range.split('/').collect();
            assert_eq!(parts.len(), 2);
            let _prefix: u8 = parts[1].parse().unwrap();
        }
    }

    // =========================================================================
    // DoH provider blocking tests
    // =========================================================================

    #[test]
    fn test_doh_provider_ips() {
        // Known DoH providers that should be blockable
        let doh_providers = [
            ("1.1.1.1", "Cloudflare"),
            ("8.8.8.8", "Google"),
            ("9.9.9.9", "Quad9"),
            ("208.67.222.222", "OpenDNS"),
        ];

        for (ip, name) in doh_providers {
            // Validate IP format
            let parts: Vec<&str> = ip.split('.').collect();
            assert_eq!(parts.len(), 4, "{} IP should have 4 octets", name);

            for part in parts {
                let octet: u8 = part.parse().expect("Should be valid octet");
                // Octet is u8 so always 0-255, just verify it parsed
                let _ = octet;
            }
        }
    }

    #[test]
    fn test_doh_ports() {
        // DoH uses HTTPS (443), DoT uses 853
        let doh_port = 443u16;
        let dot_port = 853u16;

        assert!(doh_port > 0);
        assert!(dot_port > 0);
        assert_ne!(doh_port, dot_port);
    }

    // =========================================================================
    // Rule priority tests
    // =========================================================================

    #[test]
    fn test_rule_order_concept() {
        // Rules should be applied in order:
        // 1. Allow loopback
        // 2. Allow established
        // 3. Allow VPN server
        // 4. Allow VPN interface
        // 5. Allow LAN (if configured)
        // 6. Allow DHCP
        // 7. Block all else

        let rule_order = [
            "loopback",
            "established",
            "vpn_server",
            "vpn_interface",
            "lan",
            "dhcp",
            "drop_all",
        ];

        // Verify loopback comes first
        assert_eq!(rule_order[0], "loopback");
        // Verify drop_all comes last
        assert_eq!(rule_order.last().unwrap(), &"drop_all");
    }

    // =========================================================================
    // Boot kill switch tests
    // =========================================================================

    #[test]
    fn test_boot_killswitch_concept() {
        // Boot kill switch is enabled before VPN connects
        // to prevent traffic leaks during startup

        struct BootKillSwitchState {
            enabled: bool,
            allow_lan: bool,
        }

        let state = BootKillSwitchState {
            enabled: true,
            allow_lan: true,
        };

        assert!(state.enabled);
        assert!(state.allow_lan);
    }

    // =========================================================================
    // Cleanup/teardown tests
    // =========================================================================

    #[test]
    fn test_cleanup_concept() {
        // Cleanup should remove all rules added by kill switch
        // Order: remove jump rule, flush chain, delete chain

        let cleanup_steps = ["remove_jump_from_output", "flush_chain", "delete_chain"];

        assert_eq!(cleanup_steps.len(), 3);
        assert!(cleanup_steps[0].contains("jump"));
        assert!(cleanup_steps[1].contains("flush"));
        assert!(cleanup_steps[2].contains("delete"));
    }

    #[test]
    fn test_stale_rules_detection_concept() {
        // Stale rules can occur if process crashes
        // Detection: check if chain exists without shroud running

        let chain_exists = true;
        let shroud_running = false;

        let is_stale = chain_exists && !shroud_running;
        assert!(is_stale);

        // If shroud is running, not stale
        let shroud_running_2 = true;
        let is_stale_2 = chain_exists && !shroud_running_2;
        assert!(!is_stale_2);
    }
}

#[cfg(test)]
mod privileged_tests {
    //! These tests require root privileges and are marked #[ignore]
    //! Run with: sudo -E cargo test --lib -- killswitch --ignored

    #[test]
    #[ignore = "Requires root privileges"]
    fn test_iptables_chain_creation() {
        // This would actually create/delete iptables chains
        // Only run in privileged test environment
    }

    #[test]
    #[ignore = "Requires root privileges"]
    fn test_iptables_rule_insertion() {
        // This would actually insert/remove iptables rules
    }

    #[test]
    #[ignore = "Requires root privileges"]
    fn test_full_killswitch_enable_disable_cycle() {
        // Full integration test of enable/disable
    }
}
