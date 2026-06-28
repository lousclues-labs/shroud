// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Fuzz target for VPN server-endpoint parsing (kill switch allowlist).
//!
//! Feeds arbitrary bytes as raw nmcli output to the pure endpoint parsers used
//! to build the kill switch server-IP allowlist (SHROUD-VULN-055). These run on
//! attacker-influenceable data (a malicious VPN profile), so they must never
//! panic and must never classify a non-IP as a whitelist-able IP.
//!
//! Verifies:
//! - `endpoint_host`, `classify_endpoint_host`, `parse_wireguard_endpoints`,
//!   and `parse_openvpn_endpoints` never panic.
//! - Every `VpnEndpoint::Ip` round-trips through `IpAddr` parsing (so it is
//!   always safe to interpolate into a firewall rule).
//! - Every `VpnEndpoint::Hostname` is genuinely NOT an IP literal (no DNS
//!   resolution is ever performed — SHROUD-VULN-041).

#![no_main]

use std::net::IpAddr;

use libfuzzer_sys::fuzz_target;
use shroud::nm::parsing::{
    classify_endpoint_host, endpoint_host, parse_openvpn_endpoints, parse_wireguard_endpoints,
    VpnEndpoint,
};

fn check_endpoints(endpoints: Vec<VpnEndpoint>) {
    for endpoint in endpoints {
        match endpoint {
            // An Ip endpoint must always be a valid IP literal — it will be
            // interpolated directly into iptables/nft rules.
            VpnEndpoint::Ip(ip) => {
                let rendered = ip.to_string();
                assert!(
                    rendered.parse::<IpAddr>().is_ok(),
                    "VpnEndpoint::Ip must render to a parseable IP: {rendered}"
                );
            }
            // A Hostname endpoint must NOT be an IP literal (otherwise it would
            // have been classified as Ip). This guards the no-DNS contract.
            VpnEndpoint::Hostname(host) => {
                assert!(
                    host.parse::<IpAddr>().is_err(),
                    "VpnEndpoint::Hostname must not be an IP literal: {host}"
                );
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let input = match std::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return,
    };

    // Low-level helpers must not panic on any input.
    let host = endpoint_host(input);
    let _ = classify_endpoint_host(host);

    // The line-oriented parsers must not panic and must uphold the
    // Ip/Hostname classification invariants.
    check_endpoints(parse_wireguard_endpoints(input));
    check_endpoints(parse_openvpn_endpoints(input));
});
