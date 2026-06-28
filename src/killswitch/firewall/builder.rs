// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! iptables rule script builders.
//!
//! Pure functions that construct the textual iptables script consumed by
//! [`super::KillSwitch::run_single_script`]. Every command in the script is
//! a `Command::new("sudo").arg("-n").arg(iptables()).args(...)` invocation
//! once split on whitespace by the runner — this module only emits the
//! source script. **There is no shell interpolation anywhere in this path.**
//!
//! Rule order is significant. The script:
//!
//! 1. Creates the `SHROUD_KILLSWITCH` chain (no jump rule yet).
//! 2. Populates the chain in this order: loopback → established/related →
//!    DNS → tunnel interfaces → DoH blocklist → VPN server allowlist →
//!    LAN allowlist → DHCP → log → DROP.
//! 3. Inserts the OUTPUT-chain jump rule **last**, so traffic is never
//!    directed through a partially populated chain (SHROUD-VULN-021 et al.).
//! 4. Appends the IPv6 fragment from
//!    [`super::ip6tables::build_ipv6_script`].

use std::net::IpAddr;
use tracing::warn;

use crate::config::DnsMode;
use crate::killswitch::paths::iptables;
use crate::killswitch::rules::DOH_PROVIDERS as DOH_PROVIDER_IPS;

use super::ip6tables::build_ipv6_script;
use super::KillSwitch;

impl KillSwitch {
    pub(super) fn build_complete_script(&self, vpn_ips: &[IpAddr]) -> String {
        let mut s = String::new();

        // Note: Cleanup is handled by robust_iptables_cleanup() before this is called
        // We just need to create the chain and add rules

        // Create chain (no jump rule yet — chain must be fully populated first
        // to avoid a window where traffic hits an incomplete chain)
        s.push_str(&format!("{} -N SHROUD_KILLSWITCH\n", iptables()));

        // Rules
        s.push_str(&format!(
            "{} -A SHROUD_KILLSWITCH -o lo -j ACCEPT\n",
            iptables()
        ));
        s.push_str(&format!(
            "{} -A SHROUD_KILLSWITCH -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT\n",
            iptables()
        ));
        // DNS rules must come before VPN interface allow rules
        s.push_str(&self.build_dns_rules());

        s.push_str(&format!(
            "{} -A SHROUD_KILLSWITCH -o tun+ -j ACCEPT\n",
            iptables()
        ));
        s.push_str(&format!(
            "{} -A SHROUD_KILLSWITCH -o tap+ -j ACCEPT\n",
            iptables()
        ));
        s.push_str(&format!(
            "{} -A SHROUD_KILLSWITCH -o wg+ -j ACCEPT\n",
            iptables()
        ));

        s.push_str(&self.build_doh_blocking_rules());

        for ip in vpn_ips {
            match ip {
                IpAddr::V4(v4) => {
                    // SECURITY: Validate before interpolation, consistent with
                    // every other IP placed into a rule (SHROUD-VULN-022 family).
                    if !crate::killswitch::rules::is_valid_ipv4(&v4.to_string()) {
                        warn!(
                            "Rejected invalid VPN server IP (possible injection): {}",
                            v4
                        );
                        continue;
                    }
                    s.push_str(&format!(
                        "{} -A SHROUD_KILLSWITCH -d {} -j ACCEPT\n",
                        iptables(),
                        v4
                    ));
                }
                IpAddr::V6(v6) => {
                    // The iptables backend keeps IPv6 handling in a separate
                    // ip6tables OUTPUT script with a *static* cleanup list
                    // (ip6tables::IPV6_OUTPUT_RULES). A dynamic per-server IPv6
                    // allow rule there would be orphaned on cleanup (Principle
                    // III — Leave No Trace). The atomic nftables backend
                    // whitelists IPv6 server IPs correctly and is preferred
                    // (select_backend). Fail loud rather than drop silently.
                    warn!(
                        "VPN server IP {} is IPv6; the iptables backend cannot pre-whitelist \
                         it. Use the nftables backend for IPv6 endpoint support.",
                        v6
                    );
                }
            }
        }

        // LAN access rules — use detected subnets instead of full RFC1918
        let lan_subnets = crate::killswitch::rules::detect_local_subnets();
        for subnet in &lan_subnets {
            // SECURITY: Double-check that detected subnets are valid private CIDRs.
            // Rejects 0.0.0.0/0 or public ranges that would open the kill switch
            // to all traffic (SHROUD-VULN-021).
            if !crate::killswitch::rules::is_valid_private_cidr(subnet) {
                warn!("Rejected non-private subnet in iptables rules: {}", subnet);
                continue;
            }
            s.push_str(&format!(
                "{} -A SHROUD_KILLSWITCH -d {} -j ACCEPT\n",
                iptables(),
                subnet
            ));
        }
        s.push_str(&format!(
            "{} -A SHROUD_KILLSWITCH -p udp --dport 67 -j ACCEPT\n",
            iptables()
        ));
        s.push_str(&format!(
            "{} -A SHROUD_KILLSWITCH -p udp --sport 68 -j ACCEPT\n",
            iptables()
        ));

        s.push_str(&format!(
            "{} -A SHROUD_KILLSWITCH -m limit --limit 1/sec -j LOG --log-prefix SHROUD-KS-DROP --log-level 4\n",
            iptables()
        ));
        s.push_str(&format!("{} -A SHROUD_KILLSWITCH -j DROP\n", iptables()));

        // SECURITY: Insert jump rule LAST — chain is now fully populated,
        // so traffic is never directed to an incomplete rule chain.
        s.push_str(&format!(
            "{} -I OUTPUT 1 -j SHROUD_KILLSWITCH\n",
            iptables()
        ));

        // IPv6
        s.push_str(&build_ipv6_script(self.ipv6_mode));

        s
    }

    /// Build a preview of the kill switch rules script (for diagnostics/tests)
    #[allow(dead_code)]
    pub fn build_rules_preview(&self, vpn_ips: &[IpAddr]) -> String {
        self.build_complete_script(vpn_ips)
    }

    pub(super) fn build_dns_rules(&self) -> String {
        let mut s = String::new();

        match self.dns_mode {
            DnsMode::Tunnel | DnsMode::Strict => {
                s.push_str("# DNS Leak Protection (Tunnel/Strict)\n");
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -o tun+ -p udp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -o tun+ -p tcp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -o wg+ -p udp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -o wg+ -p tcp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -o tap+ -p udp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -o tap+ -p tcp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -p udp --dport 53 -j DROP\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -p tcp --dport 53 -j DROP\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -p tcp --dport 853 -j DROP\n",
                    iptables()
                ));
            }
            DnsMode::Localhost => {
                s.push_str("# DNS Leak Protection (Localhost)\n");
                // SECURITY: Only allow DNS to 127.0.0.1 and 127.0.0.53 (systemd-resolved),
                // not the entire 127.0.0.0/8 range, to prevent rogue resolvers on other
                // loopback addresses.
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -d 127.0.0.1 -p udp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -d 127.0.0.1 -p tcp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -d 127.0.0.53 -p udp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -d 127.0.0.53 -p tcp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -d ::1 -p udp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -d ::1 -p tcp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -p udp --dport 53 -j DROP\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -p tcp --dport 53 -j DROP\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -p tcp --dport 853 -j DROP\n",
                    iptables()
                ));
            }
            DnsMode::Any => {
                s.push_str("# DNS (Any Mode - NOT RECOMMENDED)\n");
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -p udp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
                s.push_str(&format!(
                    "{} -A SHROUD_KILLSWITCH -p tcp --dport 53 -j ACCEPT\n",
                    iptables()
                ));
            }
        }

        s
    }

    pub(super) fn build_doh_blocking_rules(&self) -> String {
        if !self.block_doh {
            return String::new();
        }

        if !matches!(self.dns_mode, DnsMode::Tunnel | DnsMode::Strict) {
            return String::new();
        }

        let mut s = String::new();
        s.push_str("# Block DNS-over-HTTPS (DoH) to known providers\n");

        for ip in DOH_PROVIDER_IPS
            .iter()
            .copied()
            .chain(self.custom_doh_blocklist.iter().map(|s| s.as_str()))
        {
            // SECURITY: Validate each IP to prevent injection into iptables
            // commands. custom_doh_blocklist comes from config.toml and could
            // contain crafted strings (SHROUD-VULN-022).
            if !crate::killswitch::rules::is_valid_ipv4(ip) {
                warn!(
                    "Rejected invalid DoH blocklist IP (possible injection): {}",
                    ip
                );
                continue;
            }
            s.push_str(&format!(
                "{} -A SHROUD_KILLSWITCH -d {} -p tcp --dport 443 -j DROP\n",
                iptables(),
                ip
            ));
        }

        s
    }
}
