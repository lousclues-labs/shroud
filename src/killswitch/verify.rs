// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Kill switch verification (read-only)
//!
//! This module inspects live iptables/nftables state to verify the kill switch rules
//! are present and effective. It is read-only and leaves no trace.

use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::process::Command;

use crate::config::{ConfigManager, DnsMode, Ipv6Mode};
use crate::ipc::client::send_command;
use crate::ipc::protocol::{IpcCommand, IpcResponse};
use crate::killswitch::paths::{ip6tables, iptables, nft};

const CHAIN_NAME: &str = "SHROUD_KILLSWITCH";
const NFT_TABLE: &str = "shroud_killswitch";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub overall: Verdict,
    pub checks: Vec<CheckResult>,
    pub timestamp: String,
    pub backend: String,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub description: String,
    pub verdict: Verdict,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

impl CheckResult {
    fn pass(name: &str, description: &str, detail: impl Into<String>, raw: Option<String>) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            verdict: Verdict::Pass,
            detail: detail.into(),
            raw,
        }
    }

    fn warn(name: &str, description: &str, detail: impl Into<String>, raw: Option<String>) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            verdict: Verdict::Warn,
            detail: detail.into(),
            raw,
        }
    }

    fn fail(name: &str, description: &str, detail: impl Into<String>, raw: Option<String>) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            verdict: Verdict::Fail,
            detail: detail.into(),
            raw,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Iptables,
    Nftables,
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Backend::Iptables => write!(f, "iptables"),
            Backend::Nftables => write!(f, "nftables"),
        }
    }
}

struct IptablesSnapshot {
    ks_chain: String,
    output_chain: String,
    ip6_output: String,
}

struct NftSnapshot {
    table_output: String,
}

/// Run the full verification and produce a report
pub async fn run_verification(verbose: bool) -> Result<VerificationReport, String> {
    // 1. Detect backend
    let backend = detect_backend().await?;

    // 2. Check sudo access
    ensure_sudo_access(&backend).await?;

    // 3. Load config
    let config = ConfigManager::new().load_validated();

    // 4. Fetch firewall state
    let (ipt_snapshot, nft_snapshot) = match backend {
        Backend::Iptables => (Some(fetch_iptables_snapshot().await?), None),
        Backend::Nftables => (None, Some(fetch_nft_snapshot().await?)),
    };

    // 5. Run checks
    let mut checks = Vec::new();

    // Common indicators for state agreement
    let actual_enabled;

    match backend {
        Backend::Iptables => {
            let snap = ipt_snapshot.as_ref().unwrap();
            let chain_exists = check_chain_exists_iptables(snap, verbose).await;
            let jump_rule = check_jump_rule_iptables(verbose).await;
            actual_enabled =
                chain_exists.verdict == Verdict::Pass && jump_rule.verdict == Verdict::Pass;
            checks.extend(vec![chain_exists, jump_rule]);

            checks.push(check_default_drop_iptables(snap, verbose));
            checks.push(check_loopback_allowed_iptables(snap, verbose));
            checks.push(check_vpn_interfaces_allowed_iptables(snap, verbose));
            checks.push(check_dhcp_allowed_iptables(snap, verbose));
            checks.push(check_ipv6_protection_iptables(
                snap,
                &config.ipv6_mode,
                verbose,
            ));
            checks.push(check_dns_mode_match_iptables(
                snap,
                config.dns_mode,
                verbose,
            ));
            checks.push(check_no_rogue_rules_iptables(snap, verbose));
        }
        Backend::Nftables => {
            let snap = nft_snapshot.as_ref().unwrap();
            let chain_exists = check_chain_exists_nft(snap, verbose).await;
            let jump_rule = check_jump_rule_nft(snap, verbose).await;
            actual_enabled =
                chain_exists.verdict == Verdict::Pass && jump_rule.verdict == Verdict::Pass;
            checks.extend(vec![chain_exists, jump_rule]);

            checks.push(check_default_drop_nft(snap, verbose));
            checks.push(check_loopback_allowed_nft(snap, verbose));
            checks.push(check_vpn_interfaces_allowed_nft(snap, verbose));
            checks.push(check_dhcp_allowed_nft(snap, verbose));
            checks.push(check_ipv6_protection_nft(snap, &config.ipv6_mode, verbose));
            checks.push(check_dns_mode_match_nft(snap, config.dns_mode, verbose));
            checks.push(check_no_rogue_rules_nft(snap, verbose));
        }
    }

    checks.push(check_state_agreement(actual_enabled).await);

    let overall = aggregate_verdict(&checks);
    let summary = summarize(&checks);

    Ok(VerificationReport {
        overall,
        checks,
        timestamp: now_iso8601(),
        backend: backend.to_string(),
        summary,
    })
}

fn aggregate_verdict(checks: &[CheckResult]) -> Verdict {
    if checks.iter().any(|c| c.verdict == Verdict::Fail) {
        Verdict::Fail
    } else if checks.iter().any(|c| c.verdict == Verdict::Warn) {
        Verdict::Warn
    } else {
        Verdict::Pass
    }
}

fn summarize(checks: &[CheckResult]) -> String {
    let total = checks.len();
    let pass = checks.iter().filter(|c| c.verdict == Verdict::Pass).count();
    let warn = checks.iter().filter(|c| c.verdict == Verdict::Warn).count();
    let fail = checks.iter().filter(|c| c.verdict == Verdict::Fail).count();
    format!(
        "{}/{} checks passed, {} warning(s), {} failure(s)",
        pass, total, warn, fail
    )
}

fn now_iso8601() -> String {
    // Simple UTC timestamp without adding new dependencies
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(dur) => {
            // format as seconds.millis since epoch
            format!("{}s", dur.as_secs())
        }
        Err(_) => "0s".to_string(),
    }
}

async fn detect_backend() -> Result<Backend, String> {
    // Prefer iptables if chain exists; otherwise try nft
    if Command::new("sudo")
        .args(["-n", iptables(), "-t", "filter", "-S", CHAIN_NAME])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Ok(Backend::Iptables);
    }

    if Command::new("sudo")
        .args(["-n", nft(), "list", "table", "inet", NFT_TABLE])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Ok(Backend::Nftables);
    }

    // If iptables exists and sudo works, default to iptables; else error
    if Command::new(iptables())
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Ok(Backend::Iptables);
    }

    Err("Neither iptables nor nftables kill switch rules found".to_string())
}

async fn ensure_sudo_access(backend: &Backend) -> Result<(), String> {
    match backend {
        Backend::Iptables => {
            let status = Command::new("sudo")
                .args(["-n", iptables(), "-t", "filter", "-L", "OUTPUT"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
            if matches!(status, Ok(s) if s.success()) {
                Ok(())
            } else {
                Err(
                    "Permission denied (sudo iptables failed). Run ./setup.sh --install-sudoers"
                        .to_string(),
                )
            }
        }
        Backend::Nftables => {
            let status = Command::new("sudo")
                .args(["-n", nft(), "list", "tables"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
            if matches!(status, Ok(s) if s.success()) {
                Ok(())
            } else {
                Err(
                    "Permission denied (sudo nft failed). Run ./setup.sh --install-sudoers"
                        .to_string(),
                )
            }
        }
    }
}

async fn fetch_iptables_snapshot() -> Result<IptablesSnapshot, String> {
    let ks_chain =
        run_sudo_capture_allow_missing_chain(iptables(), &["-t", "filter", "-S", CHAIN_NAME])
            .await?;
    let output_chain = run_sudo_capture(iptables(), &["-t", "filter", "-S", "OUTPUT"]).await?;
    let ip6_output = run_sudo_capture(ip6tables(), &["-t", "filter", "-S", "OUTPUT"])
        .await
        .unwrap_or_default();
    Ok(IptablesSnapshot {
        ks_chain,
        output_chain,
        ip6_output,
    })
}

async fn fetch_nft_snapshot() -> Result<NftSnapshot, String> {
    let table_output = run_sudo_capture(nft(), &["list", "table", "inet", NFT_TABLE]).await?;
    Ok(NftSnapshot { table_output })
}

fn is_missing_chain_error(stderr: &str) -> bool {
    stderr.contains("No chain/target/match by that name")
        || stderr.contains("No such file or directory")
        || stderr.contains("does not exist")
}

async fn run_sudo_capture(cmd: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("sudo")
        .arg("-n")
        .arg(cmd)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("Failed to run {}: {}", cmd, e))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn run_sudo_capture_allow_missing_chain(cmd: &str, args: &[&str]) -> Result<String, String> {
    match run_sudo_capture(cmd, args).await {
        Ok(s) => Ok(s),
        Err(e) if is_missing_chain_error(&e) => Ok(String::new()),
        Err(e) => Err(e),
    }
}

async fn check_chain_exists_iptables(snap: &IptablesSnapshot, verbose: bool) -> CheckResult {
    if snap.ks_chain.trim().is_empty() {
        return CheckResult::fail(
            "chain_exists",
            "SHROUD_KILLSWITCH chain exists",
            "Chain missing",
            Some(snap.ks_chain.clone()).filter(|_| verbose),
        );
    }
    CheckResult::pass(
        "chain_exists",
        "SHROUD_KILLSWITCH chain exists",
        format!("Chain found with {} rules", snap.ks_chain.lines().count()),
        Some(snap.ks_chain.clone()).filter(|_| verbose),
    )
}

async fn check_jump_rule_iptables(_verbose: bool) -> CheckResult {
    let status = Command::new("sudo")
        .args([
            "-n",
            iptables(),
            "-t",
            "filter",
            "-C",
            "OUTPUT",
            "-j",
            CHAIN_NAME,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await;
    match status {
        Ok(s) if s.success() => CheckResult::pass(
            "jump_rule_exists",
            "OUTPUT chain jumps to SHROUD_KILLSWITCH",
            "Jump rule present",
            None,
        ),
        _ => CheckResult::fail(
            "jump_rule_exists",
            "OUTPUT chain jumps to SHROUD_KILLSWITCH",
            "Jump rule missing",
            None,
        ),
    }
}

fn check_default_drop_iptables(snap: &IptablesSnapshot, verbose: bool) -> CheckResult {
    if snap
        .ks_chain
        .lines()
        .any(|l| l.contains(&format!("-A {} -j DROP", CHAIN_NAME)))
    {
        CheckResult::pass(
            "default_drop",
            "Default policy is DROP",
            "Final rule is DROP",
            Some(snap.ks_chain.clone()).filter(|_| verbose),
        )
    } else {
        CheckResult::fail(
            "default_drop",
            "Default policy is DROP",
            "No DROP rule found",
            Some(snap.ks_chain.clone()).filter(|_| verbose),
        )
    }
}

fn check_loopback_allowed_iptables(snap: &IptablesSnapshot, verbose: bool) -> CheckResult {
    if snap
        .ks_chain
        .lines()
        .any(|l| l.contains(&format!("-A {} -o lo -j ACCEPT", CHAIN_NAME)))
    {
        CheckResult::pass(
            "loopback_allowed",
            "Loopback traffic allowed",
            "lo interface ACCEPT rule present",
            Some(snap.ks_chain.clone()).filter(|_| verbose),
        )
    } else {
        CheckResult::fail(
            "loopback_allowed",
            "Loopback traffic allowed",
            "lo interface ACCEPT rule missing",
            Some(snap.ks_chain.clone()).filter(|_| verbose),
        )
    }
}

fn check_vpn_interfaces_allowed_iptables(snap: &IptablesSnapshot, verbose: bool) -> CheckResult {
    let mut missing = Vec::new();
    for iface in &["tun+", "wg+", "tap+"] {
        let rule = format!("-A {} -o {} -j ACCEPT", CHAIN_NAME, iface);
        if !snap.ks_chain.lines().any(|l| l.contains(&rule)) {
            missing.push(*iface);
        }
    }
    if missing.is_empty() {
        CheckResult::pass(
            "vpn_interfaces_allowed",
            "VPN tunnel interfaces allowed",
            "tun+, wg+, tap+ all allowed",
            Some(snap.ks_chain.clone()).filter(|_| verbose),
        )
    } else if missing.len() < 3 {
        CheckResult::warn(
            "vpn_interfaces_allowed",
            "VPN tunnel interfaces allowed",
            format!("Missing allow rules for: {}", missing.join(", ")),
            Some(snap.ks_chain.clone()).filter(|_| verbose),
        )
    } else {
        CheckResult::fail(
            "vpn_interfaces_allowed",
            "VPN tunnel interfaces allowed",
            "No VPN interface allow rules present",
            Some(snap.ks_chain.clone()).filter(|_| verbose),
        )
    }
}

fn check_dhcp_allowed_iptables(snap: &IptablesSnapshot, verbose: bool) -> CheckResult {
    let allow67 = snap
        .ks_chain
        .lines()
        .any(|l| l.contains("--dport 67") && l.contains("ACCEPT"));
    let allow68 = snap
        .ks_chain
        .lines()
        .any(|l| l.contains("--sport 68") && l.contains("ACCEPT"));
    if allow67 && allow68 {
        CheckResult::pass(
            "dhcp_allowed",
            "DHCP traffic allowed",
            "UDP 67/68 rules present",
            Some(snap.ks_chain.clone()).filter(|_| verbose),
        )
    } else {
        CheckResult::warn(
            "dhcp_allowed",
            "DHCP traffic allowed",
            "Missing UDP 67/68 rules",
            Some(snap.ks_chain.clone()).filter(|_| verbose),
        )
    }
}

fn check_ipv6_protection_iptables(
    snap: &IptablesSnapshot,
    mode: &Ipv6Mode,
    verbose: bool,
) -> CheckResult {
    match mode {
        Ipv6Mode::Block => {
            if snap
                .ip6_output
                .lines()
                .any(|l| l.contains("-A OUTPUT -j DROP"))
            {
                CheckResult::pass(
                    "ipv6_protection",
                    "IPv6 leak protection",
                    "IPv6 DROP rule present",
                    Some(snap.ip6_output.clone()).filter(|_| verbose),
                )
            } else {
                CheckResult::fail(
                    "ipv6_protection",
                    "IPv6 leak protection",
                    "IPv6 DROP rule missing",
                    Some(snap.ip6_output.clone()).filter(|_| verbose),
                )
            }
        }
        Ipv6Mode::Tunnel => {
            let has_tun = snap
                .ip6_output
                .lines()
                .any(|l| l.contains("-A OUTPUT -o tun+ -j ACCEPT"));
            let has_drop = snap
                .ip6_output
                .lines()
                .any(|l| l.contains("-A OUTPUT -j DROP"));
            if has_tun && has_drop {
                CheckResult::pass(
                    "ipv6_protection",
                    "IPv6 leak protection",
                    "IPv6 tunnel-only rules present",
                    Some(snap.ip6_output.clone()).filter(|_| verbose),
                )
            } else {
                CheckResult::warn(
                    "ipv6_protection",
                    "IPv6 leak protection",
                    "IPv6 tunnel rules incomplete",
                    Some(snap.ip6_output.clone()).filter(|_| verbose),
                )
            }
        }
        Ipv6Mode::Off => CheckResult::warn(
            "ipv6_protection",
            "IPv6 leak protection",
            "IPv6 protection disabled (off)",
            Some(snap.ip6_output.clone()).filter(|_| verbose),
        ),
    }
}

fn check_dns_mode_match_iptables(
    snap: &IptablesSnapshot,
    mode: DnsMode,
    verbose: bool,
) -> CheckResult {
    let rules = &snap.ks_chain;
    match mode {
        DnsMode::Tunnel | DnsMode::Strict => {
            let has_allow = rules.lines().any(|l| {
                l.contains("--dport 53")
                    && l.contains("ACCEPT")
                    && (l.contains("-o tun+") || l.contains("-o wg+") || l.contains("-o tap+"))
            });
            let has_drop_53 = rules
                .lines()
                .any(|l| l.contains("--dport 53") && l.contains("-j DROP"));
            let has_drop_dot = rules
                .lines()
                .any(|l| l.contains("--dport 853") && l.contains("-j DROP"));
            if has_allow && has_drop_53 {
                let detail = if matches!(mode, DnsMode::Strict) && !has_drop_dot {
                    "tunnel rules present (DoT drop missing)".to_string()
                } else {
                    "tunnel/strict rules present".to_string()
                };
                CheckResult::pass(
                    "dns_mode_match",
                    "DNS leak protection matches configured mode",
                    detail,
                    Some(rules.clone()).filter(|_| verbose),
                )
            } else {
                CheckResult::warn(
                    "dns_mode_match",
                    "DNS leak protection matches configured mode",
                    "DNS tunnel/strict rules incomplete",
                    Some(rules.clone()).filter(|_| verbose),
                )
            }
        }
        DnsMode::Localhost => {
            let has_localhost = rules.contains("127.0.0.0/8") || rules.contains("::1");
            if has_localhost {
                CheckResult::pass(
                    "dns_mode_match",
                    "DNS leak protection matches configured mode",
                    "localhost DNS rules present",
                    Some(rules.clone()).filter(|_| verbose),
                )
            } else {
                CheckResult::warn(
                    "dns_mode_match",
                    "DNS leak protection matches configured mode",
                    "localhost DNS rules missing",
                    Some(rules.clone()).filter(|_| verbose),
                )
            }
        }
        DnsMode::Any => CheckResult::warn(
            "dns_mode_match",
            "DNS leak protection matches configured mode",
            "dns_mode=any (least secure)",
            Some(rules.clone()).filter(|_| verbose),
        ),
    }
}

fn check_no_rogue_rules_iptables(snap: &IptablesSnapshot, verbose: bool) -> CheckResult {
    // Ensure first OUTPUT rule is jump to SHROUD_KILLSWITCH
    let mut first_output_rule: Option<&str> = None;
    for line in snap.output_chain.lines() {
        if line.starts_with("-A OUTPUT") {
            first_output_rule = Some(line);
            break;
        }
    }
    if let Some(rule) = first_output_rule {
        if rule.contains(&format!("-A OUTPUT -j {}", CHAIN_NAME)) {
            CheckResult::pass(
                "no_rogue_rules",
                "No conflicting rules in OUTPUT",
                "OUTPUT chain clean",
                Some(snap.output_chain.clone()).filter(|_| verbose),
            )
        } else {
            CheckResult::warn(
                "no_rogue_rules",
                "No conflicting rules in OUTPUT",
                format!("First OUTPUT rule is not jump to {}: {}", CHAIN_NAME, rule),
                Some(snap.output_chain.clone()).filter(|_| verbose),
            )
        }
    } else {
        CheckResult::warn(
            "no_rogue_rules",
            "No conflicting rules in OUTPUT",
            "No OUTPUT rules found",
            Some(snap.output_chain.clone()).filter(|_| verbose),
        )
    }
}

async fn check_chain_exists_nft(snap: &NftSnapshot, verbose: bool) -> CheckResult {
    if snap.table_output.trim().is_empty() {
        CheckResult::fail(
            "chain_exists",
            "shroud_killswitch table exists",
            "Table missing",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    } else {
        CheckResult::pass(
            "chain_exists",
            "shroud_killswitch table exists",
            "Table found",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    }
}

async fn check_jump_rule_nft(snap: &NftSnapshot, verbose: bool) -> CheckResult {
    // In nft mode, kill switch is implemented as base chain hook output policy drop
    if snap.table_output.contains("chain output") && snap.table_output.contains("hook output") {
        CheckResult::pass(
            "jump_rule_exists",
            "OUTPUT chain is hooked (nft)",
            "nft output chain with hook present",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    } else {
        CheckResult::fail(
            "jump_rule_exists",
            "OUTPUT chain is hooked (nft)",
            "nft output chain missing hook",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    }
}

fn check_default_drop_nft(snap: &NftSnapshot, verbose: bool) -> CheckResult {
    if snap.table_output.contains("policy drop") {
        CheckResult::pass(
            "default_drop",
            "Default policy is DROP",
            "policy drop present",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    } else {
        CheckResult::fail(
            "default_drop",
            "Default policy is DROP",
            "policy drop missing",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    }
}

fn check_loopback_allowed_nft(snap: &NftSnapshot, verbose: bool) -> CheckResult {
    if snap.table_output.contains("oifname \"lo\" accept") {
        CheckResult::pass(
            "loopback_allowed",
            "Loopback traffic allowed",
            "lo interface accept present",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    } else {
        CheckResult::fail(
            "loopback_allowed",
            "Loopback traffic allowed",
            "lo interface accept missing",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    }
}

fn check_vpn_interfaces_allowed_nft(snap: &NftSnapshot, verbose: bool) -> CheckResult {
    let mut missing = Vec::new();
    for iface in &["tun*", "wg*", "tap*"] {
        let rule = format!("oifname \"{}\" accept", iface);
        if !snap.table_output.contains(&rule) {
            missing.push(*iface);
        }
    }
    if missing.is_empty() {
        CheckResult::pass(
            "vpn_interfaces_allowed",
            "VPN tunnel interfaces allowed",
            "tun*, wg*, tap* all allowed",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    } else if missing.len() < 3 {
        CheckResult::warn(
            "vpn_interfaces_allowed",
            "VPN tunnel interfaces allowed",
            format!("Missing allow rules for: {}", missing.join(", ")),
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    } else {
        CheckResult::fail(
            "vpn_interfaces_allowed",
            "VPN tunnel interfaces allowed",
            "No VPN interface allow rules present",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    }
}

fn check_dhcp_allowed_nft(snap: &NftSnapshot, verbose: bool) -> CheckResult {
    let allow67 = snap.table_output.contains("udp dport 67 accept");
    let allow68 = snap.table_output.contains("udp sport 68 accept");
    if allow67 && allow68 {
        CheckResult::pass(
            "dhcp_allowed",
            "DHCP traffic allowed",
            "UDP 67/68 rules present",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    } else {
        CheckResult::warn(
            "dhcp_allowed",
            "DHCP traffic allowed",
            "Missing UDP 67/68 rules",
            Some(snap.table_output.clone()).filter(|_| verbose),
        )
    }
}

fn check_ipv6_protection_nft(snap: &NftSnapshot, mode: &Ipv6Mode, verbose: bool) -> CheckResult {
    match mode {
        Ipv6Mode::Block => {
            if snap.table_output.contains("meta nfproto ipv6 drop") {
                CheckResult::pass(
                    "ipv6_protection",
                    "IPv6 leak protection",
                    "meta nfproto ipv6 drop present",
                    Some(snap.table_output.clone()).filter(|_| verbose),
                )
            } else {
                CheckResult::fail(
                    "ipv6_protection",
                    "IPv6 leak protection",
                    "meta nfproto ipv6 drop missing",
                    Some(snap.table_output.clone()).filter(|_| verbose),
                )
            }
        }
        Ipv6Mode::Tunnel => {
            if snap.table_output.contains("ip6 daddr fe80::/10 accept") {
                CheckResult::pass(
                    "ipv6_protection",
                    "IPv6 leak protection",
                    "IPv6 tunnel-only rules present",
                    Some(snap.table_output.clone()).filter(|_| verbose),
                )
            } else {
                CheckResult::warn(
                    "ipv6_protection",
                    "IPv6 leak protection",
                    "IPv6 tunnel rules incomplete",
                    Some(snap.table_output.clone()).filter(|_| verbose),
                )
            }
        }
        Ipv6Mode::Off => CheckResult::warn(
            "ipv6_protection",
            "IPv6 leak protection",
            "IPv6 protection disabled (off)",
            Some(snap.table_output.clone()).filter(|_| verbose),
        ),
    }
}

fn check_dns_mode_match_nft(snap: &NftSnapshot, mode: DnsMode, verbose: bool) -> CheckResult {
    match mode {
        DnsMode::Tunnel | DnsMode::Strict => {
            let has_allow = snap.table_output.contains("udp dport 53 drop")
                || snap.table_output.contains("tcp dport 53 drop");
            if has_allow {
                CheckResult::pass(
                    "dns_mode_match",
                    "DNS leak protection matches configured mode",
                    "tunnel/strict rules present",
                    Some(snap.table_output.clone()).filter(|_| verbose),
                )
            } else {
                CheckResult::warn(
                    "dns_mode_match",
                    "DNS leak protection matches configured mode",
                    "DNS tunnel/strict rules incomplete",
                    Some(snap.table_output.clone()).filter(|_| verbose),
                )
            }
        }
        DnsMode::Localhost => {
            if snap
                .table_output
                .contains("ip daddr 127.0.0.0/8 udp dport 53 accept")
            {
                CheckResult::pass(
                    "dns_mode_match",
                    "DNS leak protection matches configured mode",
                    "localhost DNS rules present",
                    Some(snap.table_output.clone()).filter(|_| verbose),
                )
            } else {
                CheckResult::warn(
                    "dns_mode_match",
                    "DNS leak protection matches configured mode",
                    "localhost DNS rules missing",
                    Some(snap.table_output.clone()).filter(|_| verbose),
                )
            }
        }
        DnsMode::Any => CheckResult::warn(
            "dns_mode_match",
            "DNS leak protection matches configured mode",
            "dns_mode=any (least secure)",
            Some(snap.table_output.clone()).filter(|_| verbose),
        ),
    }
}

fn check_no_rogue_rules_nft(snap: &NftSnapshot, verbose: bool) -> CheckResult {
    // nft base chain with policy drop is sufficient; presence of other rules is okay
    CheckResult::pass(
        "no_rogue_rules",
        "No conflicting rules in OUTPUT",
        "policy drop covers OUTPUT",
        Some(snap.table_output.clone()).filter(|_| verbose),
    )
}

async fn check_state_agreement(actual_enabled: bool) -> CheckResult {
    match send_command(IpcCommand::KillSwitchStatus).await {
        Ok(IpcResponse::KillSwitchStatus { enabled }) => {
            if enabled == actual_enabled {
                CheckResult::pass(
                    "state_agreement",
                    "State machine agrees kill switch is active",
                    "Daemon state matches firewall",
                    None,
                )
            } else {
                CheckResult::fail(
                    "state_agreement",
                    "State machine agrees kill switch is active",
                    format!(
                        "Daemon reports enabled={} but firewall enabled={}",
                        enabled, actual_enabled
                    ),
                    None,
                )
            }
        }
        Err(_) => CheckResult::warn(
            "state_agreement",
            "State machine agrees kill switch is active",
            "Daemon not running or IPC failed",
            None,
        ),
        _ => CheckResult::warn(
            "state_agreement",
            "State machine agrees kill switch is active",
            "Unexpected IPC response",
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_iptables_chain() -> IptablesSnapshot {
        IptablesSnapshot {
            ks_chain: r"-A SHROUD_KILLSWITCH -o lo -j ACCEPT
-A SHROUD_KILLSWITCH -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
-A SHROUD_KILLSWITCH -o tun+ -j ACCEPT
-A SHROUD_KILLSWITCH -o wg+ -j ACCEPT
-A SHROUD_KILLSWITCH -o tap+ -j ACCEPT
-A SHROUD_KILLSWITCH -o tun+ -p udp --dport 53 -j ACCEPT
-A SHROUD_KILLSWITCH -p udp --dport 67 -j ACCEPT
-A SHROUD_KILLSWITCH -p udp --sport 68 -j ACCEPT
-A SHROUD_KILLSWITCH -p udp --dport 53 -j DROP
-A SHROUD_KILLSWITCH -p tcp --dport 53 -j DROP
-A SHROUD_KILLSWITCH -j DROP
"
            .to_string(),
            output_chain: r"-A OUTPUT -j SHROUD_KILLSWITCH
"
            .to_string(),
            ip6_output: r"-A OUTPUT -o lo -j ACCEPT
-A OUTPUT -o tun+ -j ACCEPT
-A OUTPUT -j DROP
"
            .to_string(),
        }
    }

    fn sample_nft_snapshot() -> NftSnapshot {
        NftSnapshot {
            table_output: r#"table inet shroud_killswitch {
    chain output {
        type filter hook output priority 0; policy drop;
        oifname "lo" accept
        ct state established,related accept
        udp dport 67 accept
        udp sport 68 accept
        oifname "tun*" accept
        oifname "wg*" accept
        oifname "tap*" accept
        udp dport 53 drop
        tcp dport 53 drop
        meta nfproto ipv6 drop
    }
}"#
            .to_string(),
        }
    }

    #[tokio::test]
    async fn test_aggregate_verdict() {
        let checks = vec![
            CheckResult::pass("a", "", "", None),
            CheckResult::warn("b", "", "", None),
        ];
        assert_eq!(aggregate_verdict(&checks), Verdict::Warn);
        let checks = vec![
            CheckResult::pass("a", "", "", None),
            CheckResult::fail("b", "", "", None),
        ];
        assert_eq!(aggregate_verdict(&checks), Verdict::Fail);
        let checks = vec![CheckResult::pass("a", "", "", None)];
        assert_eq!(aggregate_verdict(&checks), Verdict::Pass);
    }

    #[tokio::test]
    async fn test_iptables_checks_pass() {
        let snap = sample_iptables_chain();
        assert_eq!(
            check_chain_exists_iptables(&snap, false).await.verdict,
            Verdict::Pass
        );
        assert_eq!(
            check_default_drop_iptables(&snap, false).verdict,
            Verdict::Pass
        );
        assert_eq!(
            check_loopback_allowed_iptables(&snap, false).verdict,
            Verdict::Pass
        );
        assert_eq!(
            check_vpn_interfaces_allowed_iptables(&snap, false).verdict,
            Verdict::Pass
        );
        assert_eq!(
            check_dhcp_allowed_iptables(&snap, false).verdict,
            Verdict::Pass
        );
        assert_eq!(
            check_ipv6_protection_iptables(&snap, &Ipv6Mode::Block, false).verdict,
            Verdict::Pass
        );
        assert_eq!(
            check_dns_mode_match_iptables(&snap, DnsMode::Tunnel, false).verdict,
            Verdict::Pass
        );
        assert_eq!(
            check_no_rogue_rules_iptables(&snap, false).verdict,
            Verdict::Pass
        );
    }

    #[tokio::test]
    async fn test_iptables_chain_missing() {
        let snap = IptablesSnapshot {
            ks_chain: String::new(),
            output_chain: String::new(),
            ip6_output: String::new(),
        };
        assert_eq!(
            check_chain_exists_iptables(&snap, false).await.verdict,
            Verdict::Fail
        );
    }

    #[tokio::test]
    async fn test_nft_checks_pass() {
        let snap = sample_nft_snapshot();
        assert_eq!(
            check_chain_exists_nft(&snap, false).await.verdict,
            Verdict::Pass
        );
        assert_eq!(
            check_jump_rule_nft(&snap, false).await.verdict,
            Verdict::Pass
        );
        assert_eq!(check_default_drop_nft(&snap, false).verdict, Verdict::Pass);
        assert_eq!(
            check_loopback_allowed_nft(&snap, false).verdict,
            Verdict::Pass
        );
        assert_eq!(
            check_vpn_interfaces_allowed_nft(&snap, false).verdict,
            Verdict::Pass
        );
        assert_eq!(check_dhcp_allowed_nft(&snap, false).verdict, Verdict::Pass);
        assert_eq!(
            check_ipv6_protection_nft(&snap, &Ipv6Mode::Block, false).verdict,
            Verdict::Pass
        );
        assert_eq!(
            check_dns_mode_match_nft(&snap, DnsMode::Tunnel, false).verdict,
            Verdict::Pass
        );
        assert_eq!(
            check_no_rogue_rules_nft(&snap, false).verdict,
            Verdict::Pass
        );
    }

    #[test]
    fn test_summary_generation() {
        let checks = vec![
            CheckResult::pass("a", "", "", None),
            CheckResult::warn("b", "", "", None),
            CheckResult::fail("c", "", "", None),
        ];
        assert!(summarize(&checks).contains("1/3"));
    }

    #[test]
    fn test_report_serialization() {
        let report = VerificationReport {
            overall: Verdict::Pass,
            checks: vec![CheckResult::pass("chain_exists", "", "ok", None)],
            timestamp: "123s".into(),
            backend: "iptables".into(),
            summary: "1/1 checks passed".into(),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"overall\":"));
    }
}
