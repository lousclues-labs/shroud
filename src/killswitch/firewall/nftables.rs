// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! nftables backend.
//!
//! nftables applies the entire ruleset atomically (`nft -f -`), eliminating
//! the brief traffic gap that iptables would otherwise produce while the
//! kill switch chain is being populated. It is preferred over iptables when
//! `nft` is available.
//!
//! Layout:
//! - Detection: [`KillSwitch::nft_is_available`].
//! - Ruleset construction: [`KillSwitch::build_nft_ruleset`].
//! - Apply / remove / verify: [`KillSwitch::enable_nft`],
//!   [`KillSwitch::disable_nft`], [`KillSwitch::cleanup_nft_table`],
//!   [`KillSwitch::verify_nft_rules_exist`].
//! - Process spawning: [`KillSwitch::run_nft`] — uses
//!   `Command::new("sudo").arg("-n").arg(nft()).args(...)`; the ruleset is
//!   piped on stdin, never via shell interpolation.

use std::net::IpAddr;
use std::process::Stdio;
use tokio::process::Command;
use tracing::warn;

use crate::config::{DnsMode, Ipv6Mode};
use crate::killswitch::paths::nft;
use crate::killswitch::rules::DOH_PROVIDERS as DOH_PROVIDER_IPS;

use super::{KillSwitch, KillSwitchError, NFT_TABLE};

impl KillSwitch {
    pub(super) async fn nft_is_available() -> bool {
        if Command::new("nft")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }

        if Command::new(nft())
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }

        Command::new("sudo")
            .args(["-n", nft(), "--version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub(super) async fn verify_nft_rules_exist(&self) -> bool {
        let output = Command::new("sudo")
            .args(["-n", nft(), "list", "table", "inet", NFT_TABLE])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        matches!(output, Ok(status) if status.success())
    }

    pub(super) async fn enable_nft(
        &mut self,
        vpn_server_ips: &[IpAddr],
    ) -> Result<(), KillSwitchError> {
        let _ = self.cleanup_nft_table().await;
        let rules = self.build_nft_ruleset(vpn_server_ips);
        self.run_nft(&["-f", "-"], Some(&rules)).await?;
        self.enabled = true;
        Ok(())
    }

    pub(super) async fn disable_nft(&self) -> Result<(), KillSwitchError> {
        self.cleanup_nft_table().await
    }

    pub(super) async fn cleanup_nft_table(&self) -> Result<(), KillSwitchError> {
        let result = self
            .run_nft(&["delete", "table", "inet", NFT_TABLE], None)
            .await;

        match result {
            Ok(_) => Ok(()),
            Err(KillSwitchError::Command(msg))
                if msg.contains("No such file") || msg.contains("does not exist") =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Timeout for nft commands (same as iptables)
    const NFT_CMD_TIMEOUT_SECS: u64 = 30;

    pub(super) async fn run_nft(
        &self,
        args: &[&str],
        stdin_data: Option<&str>,
    ) -> Result<(), KillSwitchError> {
        use std::time::Duration;
        use tokio::time::timeout;

        let mut cmd = Command::new("sudo");
        cmd.arg("-n"); // Non-interactive to avoid password prompts
        cmd.arg(nft());
        cmd.args(args);

        if stdin_data.is_some() {
            cmd.stdin(Stdio::piped());
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let result = timeout(Duration::from_secs(Self::NFT_CMD_TIMEOUT_SECS), async {
            let mut child = cmd.spawn().map_err(KillSwitchError::Spawn)?;

            if let Some(data) = stdin_data {
                use tokio::io::AsyncWriteExt;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin
                        .write_all(data.as_bytes())
                        .await
                        .map_err(KillSwitchError::Write)?;
                }
            }

            child
                .wait_with_output()
                .await
                .map_err(KillSwitchError::Wait)
        })
        .await;

        match result {
            Ok(Ok(output)) => {
                if output.status.success() {
                    Ok(())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(KillSwitchError::Command(stderr.trim().to_string()))
                }
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                warn!(
                    "nft command timed out after {}s",
                    Self::NFT_CMD_TIMEOUT_SECS
                );
                Err(KillSwitchError::Command(format!(
                    "nft command timed out after {}s",
                    Self::NFT_CMD_TIMEOUT_SECS
                )))
            }
        }
    }

    pub(super) fn build_nft_ruleset(&self, vpn_server_ips: &[IpAddr]) -> String {
        let mut rules = format!(
            r#"
table inet {table} {{
    chain output {{
        type filter hook output priority 0; policy drop;

        # === LOOPBACK ===
        oifname "lo" accept

        # === ESTABLISHED/RELATED ===
        ct state established,related accept
"#,
            table = NFT_TABLE
        );

        match self.ipv6_mode {
            Ipv6Mode::Block => {
                rules.push_str(
                    r#"
        # === IPv6 LEAK PROTECTION (block mode) ===
        meta nfproto ipv6 drop
"#,
                );
            }
            Ipv6Mode::Tunnel => {
                rules.push_str(
                    r#"
        # === IPv6 LEAK PROTECTION (tunnel mode) ===
        ip6 daddr fe80::/10 accept
"#,
                );
            }
            Ipv6Mode::Off => {
                rules.push_str(
                    r#"
        # === IPv6 (off - no special handling) ===
        ip6 daddr fe80::/10 accept
"#,
                );
            }
        }

        rules.push_str(
            r#"
        # === DHCP ===
        udp dport 67 accept
        udp sport 68 accept
"#,
        );

        match self.dns_mode {
            DnsMode::Tunnel | DnsMode::Strict => {
                rules.push_str(
                    r#"
        # === DNS LEAK PROTECTION (tunnel/strict) ===
        oifname "tun*" udp dport 53 accept
        oifname "tun*" tcp dport 53 accept
        oifname "wg*" udp dport 53 accept
        oifname "wg*" tcp dport 53 accept
        oifname "tap*" udp dport 53 accept
        oifname "tap*" tcp dport 53 accept
        udp dport 53 drop
        tcp dport 53 drop
        tcp dport 853 drop
"#,
                );
            }
            DnsMode::Localhost => {
                rules.push_str(
                    r#"
        # === DNS LEAK PROTECTION (localhost) ===
        ip daddr 127.0.0.1 udp dport 53 accept
        ip daddr 127.0.0.1 tcp dport 53 accept
        ip daddr 127.0.0.53 udp dport 53 accept
        ip daddr 127.0.0.53 tcp dport 53 accept
"#,
                );
                if self.ipv6_mode != Ipv6Mode::Block {
                    rules.push_str(
                        r#"
        ip6 daddr ::1 udp dport 53 accept
        ip6 daddr ::1 tcp dport 53 accept
"#,
                    );
                }
                rules.push_str(
                    r#"
        udp dport 53 drop
        tcp dport 53 drop
        tcp dport 853 drop
"#,
                );
            }
            DnsMode::Any => {
                rules.push_str(
                    r#"
        # === DNS (any mode - LEGACY/INSECURE) ===
        udp dport 53 accept
        tcp dport 53 accept
"#,
                );
            }
        }

        if self.block_doh && matches!(self.dns_mode, DnsMode::Tunnel | DnsMode::Strict) {
            rules.push_str("\n        # === Block DNS-over-HTTPS (DoH) ===\n");
            for ip in DOH_PROVIDER_IPS
                .iter()
                .copied()
                .chain(self.custom_doh_blocklist.iter().map(|s| s.as_str()))
            {
                // SECURITY: Validate each IP to prevent nft ruleset injection.
                // custom_doh_blocklist comes from config.toml (SHROUD-VULN-022).
                if !crate::killswitch::rules::is_valid_ipv4(ip) {
                    warn!(
                        "Rejected invalid DoH blocklist IP in nft (possible injection): {}",
                        ip
                    );
                    continue;
                }
                rules.push_str(&format!("        ip daddr {} tcp dport 443 drop\n", ip));
            }
        }

        // === LOCAL NETWORK (auto-detected subnets) ===
        let lan_subnets = crate::killswitch::rules::detect_local_subnets();
        rules.push_str("\n        # === LOCAL NETWORK ===\n");
        for subnet in &lan_subnets {
            // SECURITY: Double-check that detected subnets are valid private CIDRs
            if !crate::killswitch::rules::is_valid_private_cidr(subnet) {
                warn!("Rejected non-private subnet in nft rules: {}", subnet);
                continue;
            }
            rules.push_str(&format!("        ip daddr {} accept\n", subnet));
        }

        rules.push_str(
            r#"
        # === VPN TUNNEL INTERFACES ===
        oifname "tun*" accept
        oifname "tap*" accept
        oifname "wg*" accept
"#,
        );

        if !vpn_server_ips.is_empty() {
            rules.push_str("\n        # === VPN SERVER ALLOWLIST ===\n");
        }
        for ip in vpn_server_ips {
            match ip {
                IpAddr::V4(v4) => {
                    // SECURITY: Validate before interpolation, consistent with
                    // every other IP placed into the nft ruleset (SHROUD-VULN-022).
                    if !crate::killswitch::rules::is_valid_ipv4(&v4.to_string()) {
                        warn!(
                            "Rejected invalid VPN server IP in nft (possible injection): {}",
                            v4
                        );
                        continue;
                    }
                    rules.push_str(&format!("        ip daddr {} accept\n", v4));
                }
                IpAddr::V6(v6) => {
                    if self.ipv6_mode != Ipv6Mode::Block {
                        rules.push_str(&format!("        ip6 daddr {} accept\n", v6));
                    }
                }
            }
        }

        if let Some(ip) = self.vpn_server_ip {
            match ip {
                IpAddr::V4(v4) => {
                    rules.push_str(&format!("        ip daddr {} accept\n", v4));
                }
                IpAddr::V6(v6) => {
                    if self.ipv6_mode != Ipv6Mode::Block {
                        rules.push_str(&format!("        ip6 daddr {} accept\n", v6));
                    }
                }
            }
        }

        rules.push_str(
            r#"
        # === DEFAULT DROP ===
        limit rate 1/second log prefix "SHROUD-KS-DROP" drop
    }

    chain input {
        type filter hook input priority 0; policy accept;
    }
}
"#,
        );

        rules
    }
}

// =========================================================================
// nftables ruleset tests
// =========================================================================

#[cfg(test)]
mod nft_tests {
    use super::*;

    #[test]
    fn test_nft_ruleset_basic_structure() {
        let ks = KillSwitch::new();
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("table inet shroud_killswitch"));
        assert!(rules.contains("chain output"));
        assert!(rules.contains("policy drop"));
        assert!(rules.contains("oifname \"lo\" accept"));
        assert!(rules.contains("ct state established,related accept"));
    }

    #[test]
    fn test_nft_ruleset_vpn_interfaces() {
        let ks = KillSwitch::new();
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("oifname \"tun*\" accept"));
        assert!(rules.contains("oifname \"tap*\" accept"));
        assert!(rules.contains("oifname \"wg*\" accept"));
    }

    #[test]
    fn test_nft_ruleset_local_network() {
        let ks = KillSwitch::new();
        let rules = ks.build_nft_ruleset(&[]);
        // LAN rules now use auto-detected subnets (or RFC1918 fallback)
        assert!(
            rules.contains("ip daddr")
                && (rules.contains("192.168.")
                    || rules.contains("10.0.")
                    || rules.contains("172.16.")
                    || rules.contains("169.254.")),
            "NFT ruleset should contain LAN subnet accept rules"
        );
    }

    #[test]
    fn test_nft_ruleset_dhcp() {
        let ks = KillSwitch::new();
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("udp dport 67 accept"));
        assert!(rules.contains("udp sport 68 accept"));
    }

    #[test]
    fn test_nft_ruleset_ipv6_block() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Block, true, Vec::new());
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("meta nfproto ipv6 drop"));
    }

    #[test]
    fn test_nft_ruleset_ipv6_tunnel() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Tunnel, true, Vec::new());
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("fe80::/10 accept"));
        assert!(!rules.contains("meta nfproto ipv6 drop"));
    }

    #[test]
    fn test_nft_ruleset_ipv6_off() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Off, true, Vec::new());
        let rules = ks.build_nft_ruleset(&[]);
        assert!(!rules.contains("meta nfproto ipv6 drop"));
        assert!(rules.contains("fe80::/10 accept"));
    }

    #[test]
    fn test_nft_dns_tunnel_mode() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Block, true, Vec::new());
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("oifname \"tun*\" udp dport 53 accept"));
        assert!(rules.contains("oifname \"wg*\" udp dport 53 accept"));
        assert!(rules.contains("udp dport 53 drop"));
        assert!(rules.contains("tcp dport 53 drop"));
        assert!(rules.contains("tcp dport 853 drop"));
    }

    #[test]
    fn test_nft_dns_localhost_mode() {
        let ks = KillSwitch::with_config(DnsMode::Localhost, Ipv6Mode::Off, true, Vec::new());
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("ip daddr 127.0.0.1 udp dport 53 accept"));
        assert!(rules.contains("ip daddr 127.0.0.53 udp dport 53 accept"));
        assert!(rules.contains("udp dport 53 drop"));
    }

    #[test]
    fn test_nft_dns_localhost_with_ipv6() {
        let ks = KillSwitch::with_config(DnsMode::Localhost, Ipv6Mode::Tunnel, true, Vec::new());
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("ip6 daddr ::1 udp dport 53 accept"));
    }

    #[test]
    fn test_nft_dns_localhost_without_ipv6() {
        let ks = KillSwitch::with_config(DnsMode::Localhost, Ipv6Mode::Block, true, Vec::new());
        let rules = ks.build_nft_ruleset(&[]);
        assert!(!rules.contains("ip6 daddr ::1"));
    }

    #[test]
    fn test_nft_dns_any_mode() {
        let ks = KillSwitch::with_config(DnsMode::Any, Ipv6Mode::Block, false, Vec::new());
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("udp dport 53 accept"));
        assert!(rules.contains("tcp dport 53 accept"));
        assert!(!rules.contains("udp dport 53 drop"));
    }

    #[test]
    fn test_nft_doh_blocking() {
        let ks = KillSwitch::with_config(DnsMode::Strict, Ipv6Mode::Block, true, Vec::new());
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("ip daddr 1.1.1.1 tcp dport 443 drop"));
        assert!(rules.contains("ip daddr 8.8.8.8 tcp dport 443 drop"));
        assert!(rules.contains("ip daddr 9.9.9.9 tcp dport 443 drop"));
    }

    #[test]
    fn test_nft_doh_blocking_disabled() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Block, false, Vec::new());
        let rules = ks.build_nft_ruleset(&[]);
        assert!(!rules.contains("tcp dport 443 drop"));
    }

    #[test]
    fn test_nft_custom_doh_blocklist() {
        let custom = vec!["100.100.100.100".into(), "200.200.200.200".into()];
        let ks = KillSwitch::with_config(DnsMode::Strict, Ipv6Mode::Block, true, custom);
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("ip daddr 100.100.100.100 tcp dport 443 drop"));
        assert!(rules.contains("ip daddr 200.200.200.200 tcp dport 443 drop"));
    }

    #[test]
    fn test_nft_vpn_server_ips() {
        let ks = KillSwitch::new();
        let server_ip: IpAddr = "203.0.113.50".parse().unwrap();
        let rules = ks.build_nft_ruleset(&[server_ip]);
        assert!(rules.contains("ip daddr 203.0.113.50 accept"));
    }

    #[test]
    fn test_nft_vpn_server_ipv6() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Tunnel, true, Vec::new());
        let server_ip: IpAddr = "2001:db8::1".parse().unwrap();
        let rules = ks.build_nft_ruleset(&[server_ip]);
        assert!(rules.contains("ip6 daddr 2001:db8::1 accept"));
    }

    #[test]
    fn test_nft_vpn_server_ipv6_blocked_when_block_mode() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Block, true, Vec::new());
        let server_ip: IpAddr = "2001:db8::1".parse().unwrap();
        let rules = ks.build_nft_ruleset(&[server_ip]);
        assert!(!rules.contains("ip6 daddr 2001:db8::1 accept"));
    }

    #[test]
    fn test_nft_configured_vpn_server_ip() {
        let mut ks = KillSwitch::new();
        let ip: IpAddr = "198.51.100.1".parse().unwrap();
        ks.set_vpn_server(Some(ip));
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("ip daddr 198.51.100.1 accept"));
    }

    #[test]
    fn test_nft_log_prefix() {
        let ks = KillSwitch::new();
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("SHROUD-KS-DROP"));
    }

    #[test]
    fn test_nft_input_chain() {
        let ks = KillSwitch::new();
        let rules = ks.build_nft_ruleset(&[]);
        assert!(rules.contains("chain input"));
        assert!(rules.contains("policy accept"));
    }
}
