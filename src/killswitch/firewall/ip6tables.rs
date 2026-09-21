// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! ip6tables (IPv6) backend helpers.
//!
//! IPv6 leak protection is implemented in two pieces:
//! 1. A small, pure script-fragment generator ([`build_ipv6_script`]) that
//!    appends the `ip6tables` rules required by the configured [`Ipv6Mode`]
//!    onto the iptables script in [`super::builder`].
//! 2. The list of IPv6 rule patterns ([`IPV6_OUTPUT_RULES`]) that
//!    [`super::chains`]'s `robust_iptables_cleanup` iterates over to remove
//!    every duplicate IPv6 rule the kill switch may have inserted.
//!
//! All command construction uses `Command::new().args()` upstream — this
//! module only emits string fragments that are then split into argv by
//! `KillSwitch::run_single_script`.

use crate::config::Ipv6Mode;
use crate::killswitch::paths::ip6tables;

/// Build the ip6tables portion of the kill switch script for the given IPv6 mode.
///
/// The fragment is appended to the IPv4 iptables script and consumed by
/// [`super::KillSwitch::run_single_script`]. Trailing `2>/dev/null || true` is
/// stripped by the runner; failures here are tolerated because the IPv6 stack
/// may legitimately be absent on some kernels.
///
/// When `allow_lan` is set, detected ULA prefixes are permitted alongside
/// link-local so the LAN exception behaves the same on both address families.
/// Previously only `fe80::/10` was allowed, so a dual-stack host with a ULA
/// prefix lost LAN reachability over IPv6 despite `allow_lan = true`.
pub(super) fn build_ipv6_script(ipv6_mode: Ipv6Mode, allow_lan: bool) -> String {
    let mut s = String::new();
    match ipv6_mode {
        Ipv6Mode::Block => {
            s.push_str(&format!(
                "{} -I OUTPUT 1 -o lo -j ACCEPT 2>/dev/null || true\n",
                ip6tables()
            ));
            s.push_str(&format!(
                "{} -I OUTPUT 2 -j DROP 2>/dev/null || true\n",
                ip6tables()
            ));
        }
        Ipv6Mode::Tunnel => {
            s.push_str(&format!(
                "{} -I OUTPUT 1 -o lo -j ACCEPT 2>/dev/null || true\n",
                ip6tables()
            ));
            s.push_str(&format!(
                "{} -I OUTPUT 2 -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT 2>/dev/null || true\n",
                ip6tables()
            ));
            s.push_str(&format!(
                "{} -I OUTPUT 3 -o tun+ -j ACCEPT 2>/dev/null || true\n",
                ip6tables()
            ));

            // Insert at a fixed index so the terminal DROP stays last.
            let mut index = 4;
            if allow_lan {
                for subnet in crate::killswitch::rules::detect_local_subnets_v6() {
                    // Defence in depth: detection already validates, but these
                    // strings flow into firewall rules (SHROUD-VULN-021/022).
                    if !crate::killswitch::rules::is_valid_private_cidr_v6(&subnet) {
                        tracing::warn!("Rejected non-private IPv6 LAN subnet: {}", subnet);
                        continue;
                    }
                    s.push_str(&format!(
                        "{} -I OUTPUT {} -d {} -j ACCEPT 2>/dev/null || true\n",
                        ip6tables(),
                        index,
                        subnet
                    ));
                    index += 1;
                }
            } else {
                // Link-local is required for neighbour discovery regardless.
                s.push_str(&format!(
                    "{} -I OUTPUT {} -d fe80::/10 -j ACCEPT 2>/dev/null || true\n",
                    ip6tables(),
                    index
                ));
                index += 1;
            }

            s.push_str(&format!(
                "{} -I OUTPUT {} -j DROP 2>/dev/null || true\n",
                ip6tables(),
                index
            ));
        }
        Ipv6Mode::Off => {}
    }
    s
}

/// IPv6 OUTPUT-chain rule patterns the kill switch may have inserted.
///
/// Used by `super::chains::robust_iptables_cleanup` to remove every
/// duplicate copy of each rule (rules can accumulate if the kill switch is
/// repeatedly enabled across crashes — the cleanup loop deletes one at a
/// time until the kernel reports the rule is gone).
pub(super) const IPV6_OUTPUT_RULES: &[&[&str]] = &[
    &["-D", "OUTPUT", "-j", "DROP"],
    &["-D", "OUTPUT", "-o", "lo", "-j", "ACCEPT"],
    &[
        "-D",
        "OUTPUT",
        "-m",
        "conntrack",
        "--ctstate",
        "ESTABLISHED,RELATED",
        "-j",
        "ACCEPT",
    ],
    &["-D", "OUTPUT", "-o", "tun+", "-j", "ACCEPT"],
    &["-D", "OUTPUT", "-d", "fe80::/10", "-j", "ACCEPT"],
];
