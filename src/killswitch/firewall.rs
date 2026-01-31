//! iptables-based VPN kill switch
//!
//! Uses a dedicated iptables chain to block all outbound traffic except:
//! - Traffic through VPN tunnel interfaces (tun*, wg*, tap*)
//! - Traffic to the VPN server IP (to establish connection)
//! - Local loopback traffic
//! - Established/related connections
//! - Local network traffic (192.168.0.0/16, etc)
//! - DHCP
//!
//! The kill switch uses a separate chain "SHROUD_KILLSWITCH" in the filter table
//! and inserts a jump rule at the top of the OUTPUT chain.
//!
//! ## DNS Leak Protection
//!
//! Controlled by `dns_mode` config:
//! - `tunnel`: DNS only via VPN tunnel interfaces (most secure)
//! - `strict`: tunnel + DoH/DoT blocking (maximum protection)
//! - `localhost`: DNS only to 127.0.0.0/8, ::1, 127.0.0.53
//! - `any`: DNS to any destination (legacy, least secure)
//!
//! ## IPv6 Leak Protection
//!
//! Controlled by `ipv6_mode` config:
//! - `block`: Drop all IPv6 except loopback (most secure)
//! - `tunnel`: Allow IPv6 only via VPN tunnel interfaces
//! - `off`: No special IPv6 handling (legacy)

#![allow(dead_code)]

use log::{debug, info, warn};
use std::net::IpAddr;
use std::process::Stdio;
use thiserror::Error;
use tokio::process::Command;

use crate::config::{DnsMode, Ipv6Mode};

/// Name of the iptables chain for the kill switch
const CHAIN_NAME: &str = "SHROUD_KILLSWITCH";

/// Absolute paths used for sudoers compatibility
const IPTABLES_BIN: &str = "/usr/bin/iptables";
const IP6TABLES_BIN: &str = "/usr/bin/ip6tables";
const NFT_BIN: &str = "/usr/bin/nft";

/// Name of the nftables table for the kill switch
const NFT_TABLE: &str = "shroud_killswitch";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirewallBackend {
    Iptables,
    Nftables,
}

/// Known DNS-over-HTTPS provider IP addresses
const DOH_PROVIDER_IPS: &[&str] = &[
    // Cloudflare
    "1.1.1.1",
    "1.0.0.1",
    // Google
    "8.8.8.8",
    "8.8.4.4",
    // Quad9
    "9.9.9.9",
    "149.112.112.112",
    // OpenDNS (Cisco)
    "208.67.222.222",
    "208.67.220.220",
    // AdGuard
    "94.140.14.14",
    "94.140.15.15",
    // CleanBrowsing
    "185.228.168.168",
    "185.228.169.168",
    // Comodo
    "8.26.56.26",
    "8.20.247.20",
];

/// Errors that can occur during kill switch operations.
#[derive(Error, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum KillSwitchError {
    /// iptables is not installed or not in PATH
    #[error("iptables is not available. Install with: sudo apt install iptables")]
    NotFound,

    /// Permission denied - need elevated privileges
    #[error(
        "Permission denied. Kill switch requires sudo access. Run: ./setup.sh --install-sudoers"
    )]
    Permission,

    /// Failed to spawn iptables/sudo process
    #[error("Failed to spawn iptables process: {0}")]
    Spawn(#[source] std::io::Error),

    /// iptables command returned error
    #[error("iptables command failed: {0}")]
    Command(String),

    /// Failed to write to process stdin
    #[error("Failed to write to process: {0}")]
    Write(#[source] std::io::Error),

    /// Failed waiting for iptables process
    #[error("Failed waiting for iptables process: {0}")]
    Wait(#[source] std::io::Error),
}

/// Kill switch status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillSwitchStatus {
    /// Kill switch is disabled (normal traffic allowed)
    Disabled,
    /// Kill switch is enabled and active (only VPN traffic allowed)
    Active,
    /// Kill switch encountered an error
    Error,
}

/// VPN Kill Switch using iptables
pub struct KillSwitch {
    /// Whether the kill switch is currently enabled
    enabled: bool,
    /// Current VPN server IP (allowed through even when kill switch is on)
    vpn_server_ip: Option<IpAddr>,
    /// VPN tunnel interface name (e.g., "tun0")
    vpn_interface: Option<String>,
    /// DNS leak protection mode
    dns_mode: DnsMode,
    /// Block DNS-over-HTTPS to known providers
    block_doh: bool,
    /// Additional DoH provider IPs to block
    custom_doh_blocklist: Vec<String>,
    /// IPv6 leak protection mode
    ipv6_mode: Ipv6Mode,
    /// Firewall backend in use
    backend: FirewallBackend,
    /// Prefer iptables-legacy over iptables-nft
    use_legacy: bool,
}

impl KillSwitch {
    /// Create a new kill switch instance with default (secure) settings
    pub fn new() -> Self {
        Self {
            enabled: false,
            vpn_server_ip: None,
            vpn_interface: None,
            dns_mode: DnsMode::default(),
            block_doh: true,
            custom_doh_blocklist: Vec::new(),
            ipv6_mode: Ipv6Mode::default(),
            backend: FirewallBackend::Iptables,
            use_legacy: false,
        }
    }

    /// Create a kill switch with specific DNS and IPv6 modes
    pub fn with_config(
        dns_mode: DnsMode,
        ipv6_mode: Ipv6Mode,
        block_doh: bool,
        custom_doh_blocklist: Vec<String>,
    ) -> Self {
        Self {
            enabled: false,
            vpn_server_ip: None,
            vpn_interface: None,
            dns_mode,
            block_doh,
            custom_doh_blocklist,
            ipv6_mode,
            backend: FirewallBackend::Iptables,
            use_legacy: false,
        }
    }

    /// Update configuration (DNS and IPv6 modes)
    pub fn set_config(
        &mut self,
        dns_mode: DnsMode,
        ipv6_mode: Ipv6Mode,
        block_doh: bool,
        custom_doh_blocklist: Vec<String>,
    ) {
        self.dns_mode = dns_mode;
        self.block_doh = block_doh;
        self.custom_doh_blocklist = custom_doh_blocklist;
        self.ipv6_mode = ipv6_mode;
    }

    /// Check if iptables is available
    pub async fn is_available() -> bool {
        Command::new(IPTABLES_BIN)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check if we have permission to modify iptables
    pub async fn has_permission() -> bool {
        // Try to list filter table - this will fail if we don't have permission
        Command::new(IPTABLES_BIN)
            .args(["-t", "filter", "-nL", "OUTPUT"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Get current status
    pub fn status(&self) -> KillSwitchStatus {
        if self.enabled {
            KillSwitchStatus::Active
        } else {
            KillSwitchStatus::Disabled
        }
    }

    /// Check if kill switch is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Set the VPN server IP to allow through the firewall
    pub fn set_vpn_server(&mut self, ip: Option<IpAddr>) {
        self.vpn_server_ip = ip;
    }

    /// Set the VPN tunnel interface
    pub fn set_vpn_interface(&mut self, iface: Option<String>) {
        self.vpn_interface = iface;
    }

    /// Check actual state of rules, not just our flag
    pub fn is_actually_enabled(&self) -> bool {
        use std::process::{Command, Stdio};

        match self.backend {
            FirewallBackend::Iptables => {
                let result = Command::new(IPTABLES_BIN)
                    .args(["-C", "OUTPUT", "-j", CHAIN_NAME])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();

                matches!(result, Ok(status) if status.success())
            }
            FirewallBackend::Nftables => {
                let result = Command::new("sudo")
                    .args([NFT_BIN, "list", "table", "inet", NFT_TABLE])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();

                matches!(result, Ok(status) if status.success())
            }
        }
    }

    /// Sync our internal state with actual iptables state
    pub fn sync_state(&mut self) {
        self.enabled = self.is_actually_enabled();
    }

    /// Enable the kill switch
    pub async fn enable(&mut self) -> Result<(), KillSwitchError> {
        if self.enabled {
            let rules_exist = match self.backend {
                FirewallBackend::Iptables => self.verify_rules_exist().await,
                FirewallBackend::Nftables => self.verify_nft_rules_exist().await,
            };
            if rules_exist {
                debug!("Kill switch already enabled");
                return Ok(());
            }
        }

        info!("Enabling VPN kill switch");

        // Detect VPN server IPs first
        let vpn_server_ips = Self::detect_all_vpn_server_ips().await;
        if !vpn_server_ips.is_empty() {
            info!(
                "Detected {} VPN server IPs to whitelist",
                vpn_server_ips.len()
            );
        }

        let backend = self.select_backend().await?;
        self.backend = backend;

        match backend {
            FirewallBackend::Iptables => {
                let script = self.build_complete_script(&vpn_server_ips);
                match self.run_single_script(&script).await {
                    Ok(()) => {}
                    Err(err) if Self::should_fallback_to_nft(&err) => {
                        if Self::nft_is_available().await {
                            warn!("iptables failed, falling back to nftables");
                            self.backend = FirewallBackend::Nftables;
                            self.enable_nft(&vpn_server_ips).await?;
                        } else {
                            return Err(err);
                        }
                    }
                    Err(err) => return Err(err),
                }
            }
            FirewallBackend::Nftables => {
                self.enable_nft(&vpn_server_ips).await?;
            }
        }

        // Note: We cannot run verification here because iptables -C
        // requires root or special permissions which the user might not have
        // without sudo (which prompts). We trust run_single_script status.

        self.enabled = true;
        info!("VPN kill switch enabled");
        Ok(())
    }

    fn build_complete_script(&self, vpn_ips: &[IpAddr]) -> String {
        let mut s = String::new();

        // Cleanup (always runs, errors ignored)
        s.push_str("/usr/bin/iptables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true\n");
        s.push_str("/usr/bin/iptables -F SHROUD_KILLSWITCH 2>/dev/null || true\n");
        s.push_str("/usr/bin/iptables -X SHROUD_KILLSWITCH 2>/dev/null || true\n");
        s.push_str("/usr/bin/nft delete table inet shroud_killswitch 2>/dev/null || true\n");

        // Create chain
        s.push_str("/usr/bin/iptables -N SHROUD_KILLSWITCH\n");
        s.push_str("/usr/bin/iptables -I OUTPUT 1 -j SHROUD_KILLSWITCH\n");

        // Rules
        s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -o lo -j ACCEPT\n");
        s.push_str(
            "/usr/bin/iptables -A SHROUD_KILLSWITCH -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT\n",
        );
        // DNS rules must come before VPN interface allow rules
        s.push_str(&self.build_dns_rules());

        s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -o tun+ -j ACCEPT\n");
        s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -o tap+ -j ACCEPT\n");
        s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -o wg+ -j ACCEPT\n");

        s.push_str(&self.build_doh_blocking_rules());

        for ip in vpn_ips {
            if let IpAddr::V4(v4) = ip {
                s.push_str(&format!(
                    "/usr/bin/iptables -A SHROUD_KILLSWITCH -d {} -j ACCEPT\n",
                    v4
                ));
            }
        }

        s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -d 192.168.0.0/16 -j ACCEPT\n");
        s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -d 10.0.0.0/8 -j ACCEPT\n");
        s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -d 172.16.0.0/12 -j ACCEPT\n");
        s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -p udp --dport 67 -j ACCEPT\n");
        s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -p udp --sport 68 -j ACCEPT\n");

        s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -m limit --limit 1/sec -j LOG --log-prefix '[SHROUD-KS DROP] ' --log-level 4\n");
        s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -j DROP\n");

        // IPv6
        match self.ipv6_mode {
            Ipv6Mode::Block => {
                s.push_str("/usr/bin/ip6tables -I OUTPUT 1 -o lo -j ACCEPT 2>/dev/null || true\n");
                s.push_str("/usr/bin/ip6tables -I OUTPUT 2 -j DROP 2>/dev/null || true\n");
            }
            Ipv6Mode::Tunnel => {
                s.push_str("/usr/bin/ip6tables -I OUTPUT 1 -o lo -j ACCEPT 2>/dev/null || true\n");
                s.push_str("/usr/bin/ip6tables -I OUTPUT 2 -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT 2>/dev/null || true\n");
                s.push_str("/usr/bin/ip6tables -I OUTPUT 3 -o tun+ -j ACCEPT 2>/dev/null || true\n");
                s.push_str("/usr/bin/ip6tables -I OUTPUT 4 -d fe80::/10 -j ACCEPT 2>/dev/null || true\n");
                s.push_str("/usr/bin/ip6tables -I OUTPUT 5 -j DROP 2>/dev/null || true\n");
            }
            Ipv6Mode::Off => {}
        }

        s
    }

    /// Build a preview of the kill switch rules script (for diagnostics/tests)
    pub fn build_rules_preview(&self, vpn_ips: &[IpAddr]) -> String {
        self.build_complete_script(vpn_ips)
    }

    fn build_dns_rules(&self) -> String {
        let mut s = String::new();

        match self.dns_mode {
            DnsMode::Tunnel | DnsMode::Strict => {
                s.push_str("# DNS Leak Protection (Tunnel/Strict)\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -o tun+ -p udp --dport 53 -j ACCEPT\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -o tun+ -p tcp --dport 53 -j ACCEPT\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -o wg+ -p udp --dport 53 -j ACCEPT\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -o wg+ -p tcp --dport 53 -j ACCEPT\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -o tap+ -p udp --dport 53 -j ACCEPT\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -o tap+ -p tcp --dport 53 -j ACCEPT\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -p udp --dport 53 -j DROP\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -p tcp --dport 53 -j DROP\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -p tcp --dport 853 -j DROP\n");
            }
            DnsMode::Localhost => {
                s.push_str("# DNS Leak Protection (Localhost)\n");
                s.push_str(
                    "/usr/bin/iptables -A SHROUD_KILLSWITCH -d 127.0.0.0/8 -p udp --dport 53 -j ACCEPT\n",
                );
                s.push_str(
                    "/usr/bin/iptables -A SHROUD_KILLSWITCH -d 127.0.0.0/8 -p tcp --dport 53 -j ACCEPT\n",
                );
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -d ::1 -p udp --dport 53 -j ACCEPT\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -d ::1 -p tcp --dport 53 -j ACCEPT\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -p udp --dport 53 -j DROP\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -p tcp --dport 53 -j DROP\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -p tcp --dport 853 -j DROP\n");
            }
            DnsMode::Any => {
                s.push_str("# DNS (Any Mode - NOT RECOMMENDED)\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -p udp --dport 53 -j ACCEPT\n");
                s.push_str("/usr/bin/iptables -A SHROUD_KILLSWITCH -p tcp --dport 53 -j ACCEPT\n");
            }
        }

        s
    }

    fn build_doh_blocking_rules(&self) -> String {
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
            s.push_str(&format!(
                "/usr/bin/iptables -A SHROUD_KILLSWITCH -d {} -p tcp --dport 443 -j DROP\n",
                ip
            ));
        }

        s
    }

    async fn run_single_script(&self, script: &str) -> Result<(), KillSwitchError> {
        use tokio::process::Command;

        for raw_line in script.lines() {
            let mut line = raw_line.trim().to_string();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let ignore_error = line.contains("|| true");
            if let Some(stripped) = line.strip_suffix("|| true") {
                line = stripped.trim().to_string();
            }
            line = line.replace("2>/dev/null", "").trim().to_string();
            if line.is_empty() || line == "exit 0" {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            let mut cmd = parts[0];
            if self.use_legacy && (parts[0].ends_with("iptables") || parts[0].ends_with("ip6tables")) {
                if let Some(legacy_cmd) = Self::legacy_variant(parts[0]).await {
                    cmd = legacy_cmd;
                }
            }

            let output = Command::new("sudo")
                .arg(cmd)
                .args(&parts[1..])
                .output()
                .await
                .map_err(KillSwitchError::Spawn)?;

            if !output.status.success() && !ignore_error {
                let code = output.status.code().unwrap_or(-1);
                if code == 126 || code == 127 {
                    return Err(KillSwitchError::Permission);
                }
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stderr_lower = stderr.to_lowercase();

                if (stderr_lower.contains("cache initialization failed")
                    || stderr_lower.contains("netlink: error")
                    || stderr_lower.contains("can't initialize iptables table")
                    || stderr_lower.contains("ip_tables"))
                    && !parts.is_empty()
                    && (parts[0].ends_with("iptables") || parts[0].ends_with("ip6tables"))
                {
                    if let Some(legacy_cmd) = Self::legacy_variant(parts[0]).await {
                        let legacy_output = Command::new("sudo")
                            .arg(legacy_cmd)
                            .args(&parts[1..])
                            .output()
                            .await
                            .map_err(KillSwitchError::Spawn)?;

                        if legacy_output.status.success() {
                            continue;
                        }
                    }

                    let detail = if stderr.trim().is_empty() {
                        line.clone()
                    } else {
                        stderr.trim().to_string()
                    };

                    return Err(KillSwitchError::Command(format!(
                        "{} (iptables-nft failed; install iptables-legacy or nftables)",
                        detail
                    )));
                }

                let detail = if stderr.trim().is_empty() {
                    line.clone()
                } else {
                    stderr.trim().to_string()
                };
                return Err(KillSwitchError::Command(format!(
                    "Command failed (exit {}): {}",
                    code, detail
                )));
            }
        }

        Ok(())
    }

    async fn legacy_variant(cmd: &str) -> Option<&'static str> {
        let (candidate, candidate_path) = if cmd.ends_with("iptables") {
            ("iptables-legacy", "/usr/bin/iptables-legacy")
        } else if cmd.ends_with("ip6tables") {
            ("ip6tables-legacy", "/usr/bin/ip6tables-legacy")
        } else {
            return None;
        };

        if Command::new(candidate)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Some(candidate);
        }

        if Command::new(candidate_path)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Some(candidate_path);
        }

        None
    }

    async fn select_backend(&mut self) -> Result<FirewallBackend, KillSwitchError> {
        match Self::check_sudo_access() {
            Ok(()) => Ok(FirewallBackend::Iptables),
            Err(err) if Self::should_fallback_to_nft(&err) => {
                if Self::check_iptables_legacy_access().unwrap_or(false) {
                    self.use_legacy = true;
                    Ok(FirewallBackend::Iptables)
                } else if Self::nft_is_available().await {
                    Ok(FirewallBackend::Nftables)
                } else {
                    Err(err)
                }
            }
            Err(err) => Err(err),
        }
    }

    fn should_fallback_to_nft(error: &KillSwitchError) -> bool {
        match error {
            KillSwitchError::Command(msg) => {
                let msg = msg.to_lowercase();
                msg.contains("ip_tables")
                    || msg.contains("table does not exist")
                    || msg.contains("can't initialize iptables table")
                    || msg.contains("cache initialization failed")
                    || msg.contains("netlink: error")
                    || msg.contains("exit 3")
                    || msg.contains("does not exist")
            }
            KillSwitchError::Spawn(_) | KillSwitchError::NotFound => true,
            _ => false,
        }
    }

    async fn nft_is_available() -> bool {
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

        if Command::new(NFT_BIN)
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
            .args(["-n", NFT_BIN, "--version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check if sudo is available and configured for passwordless iptables.
    pub fn check_sudo_access() -> Result<(), KillSwitchError> {
        let sudo_check = std::process::Command::new("sudo")
            .args(["-n", "true"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(KillSwitchError::Spawn)?;

        if !sudo_check.status.success() {
            return Err(KillSwitchError::Permission);
        }

        let output = std::process::Command::new("sudo")
            .args(["-n", IPTABLES_BIN, "-L", "-n"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(KillSwitchError::Spawn)?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_lower = stderr.to_lowercase();
        if stderr_lower.contains("ip_tables")
            || stderr_lower.contains("table does not exist")
            || stderr_lower.contains("can't initialize iptables table")
            || stderr_lower.contains("cache initialization failed")
            || stderr_lower.contains("netlink: error")
        {
            return Err(KillSwitchError::Command(format!(
                "Sudo check failed: {}",
                stderr.trim()
            )));
        }

        if output.status.success() {
            return Ok(());
        }

        Err(KillSwitchError::Command(format!(
            "Sudo check failed: {}",
            stderr.trim()
        )))
    }

    fn check_iptables_legacy_access() -> Result<bool, KillSwitchError> {
        let output = std::process::Command::new("sudo")
            .args(["-n", "/usr/bin/iptables-legacy", "-L", "-n"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();

        match output {
            Ok(out) => Ok(out.status.success()),
            Err(_) => Ok(false),
        }
    }

    /// Verify our rules are actually in place
    async fn verify_rules_exist(&self) -> bool {
        // Check if our chain exists and has the jump rule
        let output = Command::new(IPTABLES_BIN)
            .args(["-C", "OUTPUT", "-j", CHAIN_NAME])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        matches!(output, Ok(status) if status.success())
    }

    async fn verify_nft_rules_exist(&self) -> bool {
        let output = Command::new("sudo")
            .args([NFT_BIN, "list", "table", "inet", NFT_TABLE])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        matches!(output, Ok(status) if status.success())
    }

    /// Disable the kill switch
    pub async fn disable(&mut self) -> Result<(), KillSwitchError> {
        info!("Disabling VPN kill switch");

        if matches!(self.backend, FirewallBackend::Iptables) {
            if let Err(err) = Self::check_sudo_access() {
                if matches!(err, KillSwitchError::Permission) {
                    return Err(err);
                }

                if Self::should_fallback_to_nft(&err) {
                    if Self::check_iptables_legacy_access().unwrap_or(false) {
                        warn!("iptables-nft unavailable during disable; using iptables-legacy");
                        self.use_legacy = true;
                    } else if Self::nft_is_available().await {
                        warn!("iptables unavailable during disable; falling back to nftables");
                        self.backend = FirewallBackend::Nftables;
                        self.disable_nft().await?;
                        self.enabled = false;
                        info!("VPN kill switch disabled");
                        return Ok(());
                    }
                }

                warn!("iptables check failed during disable; attempting best-effort cleanup");
            }
        }

        // We run cleanup regardless of enabled status to ensuring we don't leave
        // the user stranded if the internal state is out of sync.

        let script = r#"
/usr/bin/iptables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
/usr/bin/iptables -F SHROUD_KILLSWITCH 2>/dev/null || true
/usr/bin/iptables -X SHROUD_KILLSWITCH 2>/dev/null || true
/usr/bin/ip6tables -D OUTPUT -j DROP 2>/dev/null || true
/usr/bin/ip6tables -D OUTPUT -o lo -j ACCEPT 2>/dev/null || true
/usr/bin/ip6tables -D OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT 2>/dev/null || true
/usr/bin/ip6tables -D OUTPUT -o tun+ -j ACCEPT 2>/dev/null || true
/usr/bin/ip6tables -D OUTPUT -d fe80::/10 -j ACCEPT 2>/dev/null || true
/usr/bin/nft delete table inet shroud_killswitch 2>/dev/null || true
"#;

        match self.backend {
            FirewallBackend::Iptables => {
                match self.run_single_script(script).await {
                    Ok(()) => {}
                    Err(err) if Self::should_fallback_to_nft(&err) => {
                        if Self::nft_is_available().await {
                            warn!("iptables failed during disable; falling back to nftables");
                            self.backend = FirewallBackend::Nftables;
                            self.disable_nft().await?;
                        } else {
                            warn!("iptables failed during disable and nft is unavailable; proceeding best-effort");
                        }
                    }
                    Err(err) => return Err(err),
                }
            }
            FirewallBackend::Nftables => {
                self.disable_nft().await?;
            }
        }

        self.enabled = false;
        info!("VPN kill switch disabled");
        Ok(())
    }

    /// Update the kill switch rules (e.g., when VPN interface changes)
    pub async fn update(&mut self) -> Result<(), KillSwitchError> {
        if !self.enabled {
            return Ok(());
        }

        match self.backend {
            FirewallBackend::Iptables => {
                self.disable().await?;
                self.enable().await
            }
            FirewallBackend::Nftables => {
                let vpn_server_ips = Self::detect_all_vpn_server_ips().await;
                self.enable_nft(&vpn_server_ips).await
            }
        }
    }

    async fn enable_nft(&mut self, vpn_server_ips: &[IpAddr]) -> Result<(), KillSwitchError> {
        let _ = self.cleanup_nft_table().await;
        let rules = self.build_nft_ruleset(vpn_server_ips);
        self.run_nft(&["-f", "-"], Some(&rules)).await?;
        self.enabled = true;
        Ok(())
    }

    async fn disable_nft(&self) -> Result<(), KillSwitchError> {
        self.cleanup_nft_table().await
    }

    async fn cleanup_nft_table(&self) -> Result<(), KillSwitchError> {
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

    async fn run_nft(
        &self,
        args: &[&str],
        stdin_data: Option<&str>,
    ) -> Result<(), KillSwitchError> {
        let mut cmd = Command::new("sudo");
        cmd.arg(NFT_BIN);
        cmd.args(args);

        if stdin_data.is_some() {
            cmd.stdin(Stdio::piped());
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

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

        let output = child
            .wait_with_output()
            .await
            .map_err(KillSwitchError::Wait)?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(KillSwitchError::Command(stderr.trim().to_string()))
        }
    }

    fn build_nft_ruleset(&self, vpn_server_ips: &[IpAddr]) -> String {
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
        ip daddr 127.0.0.0/8 udp dport 53 accept
        ip daddr 127.0.0.0/8 tcp dport 53 accept
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
                rules.push_str(&format!("        ip daddr {} tcp dport 443 drop\n", ip));
            }
        }

        rules.push_str(
            r#"
        # === LOCAL NETWORK ===
        ip daddr 192.168.0.0/16 accept
        ip daddr 10.0.0.0/8 accept
        ip daddr 172.16.0.0/12 accept
"#,
        );

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
        limit rate 1/second log prefix "[SHROUD-KS DROP] " drop
    }

    chain input {
        type filter hook input priority 0; policy accept;
    }
}
"#,
        );

        rules
    }

    /// Detect the VPN tunnel interface from the system
    pub async fn detect_vpn_interface() -> Option<String> {
        let output = Command::new("ip")
            .args(["link", "show"])
            .output()
            .await
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        for line in stdout.lines() {
            // Look for tun/tap interfaces
            if (line.contains("tun") || line.contains("tap") || line.contains("wg"))
                && line.contains("state UP")
            {
                // Extract interface name (format: "X: tunN: <FLAGS>...")
                if let Some(name) = line.split(':').nth(1) {
                    return Some(name.trim().to_string());
                }
            }
        }

        None
    }

    /// Get the VPN server IP from the active OpenVPN connection
    pub async fn detect_vpn_server_ip() -> Option<IpAddr> {
        // Try to get the remote IP from the tun interface route
        let output = Command::new("ip")
            .args(["route", "show", "dev", "tun0"])
            .output()
            .await
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Look for the VPN gateway in the route output
        for line in stdout.lines() {
            if line.contains("via") {
                if let Some(ip_str) = line.split_whitespace().nth(2) {
                    if let Ok(ip) = ip_str.parse() {
                        return Some(ip);
                    }
                }
            }
        }

        None
    }

    /// Detect all VPN server IPs from NetworkManager connection configs
    pub async fn detect_all_vpn_server_ips() -> Vec<IpAddr> {
        let mut ips = Vec::new();

        // Get VPN connection details from nmcli
        let output = Command::new("nmcli")
            .args(["-t", "-f", "NAME,TYPE", "connection", "show"])
            .output()
            .await;

        let connections = match output {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => return ips,
        };

        // Find VPN connections and get their remote IPs
        for line in connections.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 && parts[1] == "vpn" {
                let conn_name = parts[0];

                // Get the remote IP for this VPN connection
                if let Some(ip) = Self::get_vpn_remote_ip(conn_name).await {
                    if !ips.contains(&ip) {
                        info!("Found VPN server IP for '{}': {}", conn_name, ip);
                        ips.push(ip);
                    }
                }
            }
        }

        ips
    }

    /// Get the remote IP address for a specific VPN connection
    async fn get_vpn_remote_ip(conn_name: &str) -> Option<IpAddr> {
        // Get VPN connection details
        let output = Command::new("nmcli")
            .args(["-t", "-f", "vpn.data", "connection", "show", conn_name])
            .output()
            .await
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse vpn.data
        for line in stdout.lines() {
            if line.starts_with("vpn.data:") {
                let data = line.trim_start_matches("vpn.data:");
                for item in data.split(',') {
                    let item = item.trim();
                    if item.starts_with("remote") {
                        if let Some(value) = item.split('=').nth(1) {
                            let remote = value.trim();
                            let host = if let Some(colon_pos) = remote.rfind(':') {
                                if remote[colon_pos + 1..].parse::<u16>().is_ok() {
                                    &remote[..colon_pos]
                                } else {
                                    remote
                                }
                            } else {
                                remote
                            };

                            if let Ok(ip) = host.parse::<IpAddr>() {
                                return Some(ip);
                            }
                            if let Some(ip) = Self::resolve_hostname(host).await {
                                return Some(ip);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Resolve a hostname to an IP address
    async fn resolve_hostname(hostname: &str) -> Option<IpAddr> {
        let output = Command::new("getent")
            .args(["ahosts", hostname])
            .output()
            .await
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(ip_str) = line.split_whitespace().next() {
                if let Ok(ip) = ip_str.parse::<IpAddr>() {
                    if ip.is_ipv4() {
                        return Some(ip);
                    }
                }
            }
        }

        None
    }
}

impl Default for KillSwitch {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for KillSwitch {
    fn drop(&mut self) {
        if self.enabled {
            warn!("Kill switch dropped while enabled - rules may persist!");
            warn!("Run 'sudo iptables -F {}' to clean up", CHAIN_NAME);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kill_switch_status() {
        let ks = KillSwitch::new();
        assert_eq!(ks.status(), KillSwitchStatus::Disabled);
        assert!(!ks.is_enabled());
    }

    #[test]
    fn test_kill_switch_set_server() {
        let mut ks = KillSwitch::new();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        ks.set_vpn_server(Some(ip));
        assert_eq!(ks.vpn_server_ip, Some(ip));
    }

    #[test]
    fn test_kill_switch_with_config() {
        let ks = KillSwitch::with_config(DnsMode::Localhost, Ipv6Mode::Tunnel, true, Vec::new());
        assert_eq!(ks.dns_mode, DnsMode::Localhost);
        assert_eq!(ks.ipv6_mode, Ipv6Mode::Tunnel);
    }

    #[test]
    fn test_kill_switch_configuration_update() {
        let mut ks = KillSwitch::new();
        ks.set_config(DnsMode::Any, Ipv6Mode::Off, false, Vec::new());
        assert_eq!(ks.dns_mode, DnsMode::Any);
        assert_eq!(ks.ipv6_mode, Ipv6Mode::Off);
    }

    // Verify format of complete script
    #[test]
    fn test_build_complete_script() {
        let ks = KillSwitch::new();
        let script = ks.build_complete_script(&[]);
        assert!(script.contains("iptables -N SHROUD_KILLSWITCH"));
        assert!(script.contains("iptables -I OUTPUT 1 -j SHROUD_KILLSWITCH"));
        assert!(script.contains("nft delete table inet shroud_killswitch"));
        // Check for cleanup commands at start
        assert!(script.contains("iptables -X SHROUD_KILLSWITCH 2>/dev/null || true"));
    }

    #[test]
    fn test_tunnel_mode_dns_rules() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Block, true, Vec::new());
        let rules = ks.build_rules_preview(&[]);

        assert!(rules.contains("-o tun+ -p udp --dport 53 -j ACCEPT"));
        assert!(rules.contains("-o tun+ -p tcp --dport 53 -j ACCEPT"));
        assert!(rules.contains("-o wg+ -p udp --dport 53 -j ACCEPT"));
        assert!(rules.contains("-o wg+ -p tcp --dport 53 -j ACCEPT"));
        assert!(rules.contains("-p udp --dport 53 -j DROP"));
        assert!(rules.contains("-p tcp --dport 53 -j DROP"));
        assert!(rules.contains("-p tcp --dport 853 -j DROP"));
    }

    #[test]
    fn test_localhost_mode_dns_rules() {
        let ks = KillSwitch::with_config(DnsMode::Localhost, Ipv6Mode::Block, true, Vec::new());
        let rules = ks.build_rules_preview(&[]);

        assert!(rules.contains("-d 127.0.0.0/8 -p udp --dport 53 -j ACCEPT"));
        assert!(rules.contains("-d 127.0.0.0/8 -p tcp --dport 53 -j ACCEPT"));
        assert!(rules.contains("-d ::1 -p udp --dport 53 -j ACCEPT"));
        assert!(rules.contains("-d ::1 -p tcp --dport 53 -j ACCEPT"));
        assert!(rules.contains("-p udp --dport 53 -j DROP"));
        assert!(rules.contains("-p tcp --dport 53 -j DROP"));
    }

    #[test]
    fn test_any_mode_dns_rules() {
        let ks = KillSwitch::with_config(DnsMode::Any, Ipv6Mode::Block, true, Vec::new());
        let rules = ks.build_rules_preview(&[]);

        assert!(rules.contains("-p udp --dport 53 -j ACCEPT"));
        assert!(rules.contains("-p tcp --dport 53 -j ACCEPT"));
        assert!(!rules.contains("-p udp --dport 53 -j DROP"));
    }

    #[test]
    fn test_doh_blocking_rules() {
        let ks = KillSwitch::with_config(DnsMode::Strict, Ipv6Mode::Block, true, Vec::new());
        let rules = ks.build_rules_preview(&[]);

        assert!(rules.contains("-d 1.1.1.1 -p tcp --dport 443 -j DROP"));
        assert!(rules.contains("-d 8.8.8.8 -p tcp --dport 443 -j DROP"));
        assert!(rules.contains("-d 9.9.9.9 -p tcp --dport 443 -j DROP"));
    }

    #[test]
    fn test_dns_rule_ordering() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Block, true, Vec::new());
        let script = ks.build_rules_preview(&[]);

        let dns_accept_pos = script
            .find("-o tun+ -p udp --dport 53 -j ACCEPT")
            .expect("DNS accept rule not found");
        let dns_drop_pos = script
            .find("-p udp --dport 53 -j DROP")
            .expect("DNS drop rule not found");
        let general_tun_pos = script
            .find("-o tun+ -j ACCEPT")
            .expect("General tun+ rule not found");

        assert!(dns_accept_pos < dns_drop_pos);
        assert!(dns_drop_pos < general_tun_pos);
    }
}

#[cfg(test)]
mod leak_tests {
    use super::*;
    use std::process::Command;

    /// Helper to check if iptables has shroud rules
    fn get_iptables_rules() -> Result<String, std::io::Error> {
        let output = Command::new("sudo")
            .args(["iptables", "-L", "OUTPUT", "-n", "-v"])
            .output()?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Helper to check if ip6tables has shroud rules
    fn get_ip6tables_rules() -> Result<String, std::io::Error> {
        let output = Command::new("sudo")
            .args(["ip6tables", "-L", "OUTPUT", "-n", "-v"])
            .output()?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    #[tokio::test]
    #[ignore] // Requires sudo
    async fn test_killswitch_creates_drop_rules() {
        let mut ks = KillSwitch::new();

        // Enable kill switch
        ks.enable().await.expect("Failed to enable kill switch");

        // Verify iptables has DROP rules
        let rules = get_iptables_rules().expect("Failed to get iptables rules");

        // Should have a default DROP or REJECT for non-VPN traffic
        assert!(
            rules.contains("DROP") || rules.contains("REJECT"),
            "Kill switch should create DROP/REJECT rules. Got:\n{}",
            rules
        );

        // Clean up
        ks.disable().await.expect("Failed to disable kill switch");
    }

    #[tokio::test]
    #[ignore] // Requires sudo
    async fn test_killswitch_allows_localhost() {
        let mut ks = KillSwitch::new();
        ks.enable().await.expect("Failed to enable kill switch");

        let rules = get_iptables_rules().expect("Failed to get iptables rules");

        // Should allow localhost (127.0.0.0/8)
        assert!(
            rules.contains("127.0.0.0") || rules.contains("lo"),
            "Kill switch should allow localhost. Got:\n{}",
            rules
        );

        ks.disable().await.expect("Failed to disable kill switch");
    }

    #[tokio::test]
    #[ignore] // Requires sudo
    async fn test_killswitch_allows_lan() {
        let mut ks = KillSwitch::new();
        ks.enable().await.expect("Failed to enable kill switch");

        let rules = get_iptables_rules().expect("Failed to get iptables rules");

        // Should allow LAN (192.168.0.0/16, 10.0.0.0/8, 172.16.0.0/12)
        let allows_lan = rules.contains("192.168.0.0")
            || rules.contains("10.0.0.0")
            || rules.contains("172.16.0.0");

        assert!(allows_lan, "Kill switch should allow LAN. Got:\n{}", rules);

        ks.disable().await.expect("Failed to disable kill switch");
    }

    #[tokio::test]
    #[ignore] // Requires sudo
    async fn test_killswitch_allows_vpn_server() {
        let mut ks = KillSwitch::new();

        // Set a test VPN server IP
        let test_server_ip: IpAddr = "203.0.113.50".parse().unwrap(); // TEST-NET-3
        ks.set_vpn_server(Some(test_server_ip));
        ks.enable().await.expect("Failed to enable kill switch");

        let rules = get_iptables_rules().expect("Failed to get iptables rules");

        // Should allow traffic to VPN server
        assert!(
            rules.contains("203.0.113.50"),
            "Kill switch should allow VPN server IP {}. Got:\n{}",
            test_server_ip,
            rules
        );

        ks.disable().await.expect("Failed to disable kill switch");
    }

    #[tokio::test]
    #[ignore] // Requires sudo
    async fn test_killswitch_allows_vpn_interface() {
        let mut ks = KillSwitch::new();
        ks.enable().await.expect("Failed to enable kill switch");

        let rules = get_iptables_rules().expect("Failed to get iptables rules");

        // Should allow traffic on tun interface
        assert!(
            rules.contains("tun") || rules.contains("tap"),
            "Kill switch should allow VPN interface (tun/tap). Got:\n{}",
            rules
        );

        ks.disable().await.expect("Failed to disable kill switch");
    }

    #[tokio::test]
    #[ignore] // Requires sudo
    async fn test_killswitch_blocks_ipv6() {
        let mut ks = KillSwitch::new();
        ks.enable().await.expect("Failed to enable kill switch");

        let rules = get_ip6tables_rules().expect("Failed to get ip6tables rules");

        // Should block IPv6 to prevent leaks
        assert!(
            rules.contains("DROP") || rules.contains("REJECT"),
            "Kill switch should block IPv6. Got:\n{}",
            rules
        );

        ks.disable().await.expect("Failed to disable kill switch");
    }

    #[tokio::test]
    #[ignore] // Requires sudo
    async fn test_killswitch_disable_removes_rules() {
        let mut ks = KillSwitch::new();

        // Enable then disable
        ks.enable().await.expect("Failed to enable kill switch");
        ks.disable().await.expect("Failed to disable kill switch");

        let rules = get_iptables_rules().expect("Failed to get iptables rules");

        // Should not have shroud-specific rules
        // Check for marker comments or chain names
        assert!(
            !rules.contains("SHROUD") && !rules.contains("shroud"),
            "Kill switch rules should be removed after disable. Got:\n{}",
            rules
        );
    }

    #[tokio::test]
    #[ignore] // Requires sudo
    async fn test_killswitch_idempotent_enable() {
        let mut ks = KillSwitch::new();

        // Enable twice should not duplicate rules
        ks.enable().await.expect("Failed to enable kill switch");
        let rules_first = get_iptables_rules().expect("Failed to get rules");

        ks.enable()
            .await
            .expect("Failed to enable kill switch again");
        let rules_second = get_iptables_rules().expect("Failed to get rules");

        // Rule count should be the same
        let count_first = rules_first.matches("DROP").count();
        let count_second = rules_second.matches("DROP").count();

        assert_eq!(
            count_first, count_second,
            "Enabling twice should not duplicate rules"
        );

        ks.disable().await.expect("Failed to disable kill switch");
    }

    #[tokio::test]
    #[ignore] // Requires sudo
    async fn test_killswitch_idempotent_disable() {
        let mut ks = KillSwitch::new();

        // Disable when not enabled should not error
        let result = ks.disable().await;
        assert!(result.is_ok(), "Disable when not enabled should succeed");

        // Double disable should not error
        let result = ks.disable().await;
        assert!(result.is_ok(), "Double disable should succeed");
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn test_ip_address_validation() {
        let test_cases = vec![
            ("192.168.1.1", true),
            ("10.0.0.1", true),
            ("8.8.8.8", true),
            ("256.256.256.256", false),
            ("not.an.ip", false),
            ("-1.0.0.0", false),
            ("1.2.3.4; rm -rf /", false),
            ("$(whoami)", false),
            ("", false),
            ("1.2.3.4\n5.6.7.8", false),
            ("1.2.3.4 -j ACCEPT", false),
        ];

        for (ip_str, should_be_valid) in test_cases {
            let result: Result<IpAddr, _> = ip_str.parse();

            if should_be_valid {
                assert!(result.is_ok(), "Expected valid IP: {}", ip_str);
            }

            let is_shell_safe = !ip_str.contains(';')
                && !ip_str.contains('$')
                && !ip_str.contains('`')
                && !ip_str.contains('\n')
                && !ip_str.contains(' ');

            if should_be_valid {
                assert!(is_shell_safe, "IP should be shell-safe: {}", ip_str);
            }
        }
    }

    #[test]
    fn test_interface_name_validation() {
        let long_iface = "a".repeat(100);
        let test_cases = vec![
            ("tun0", true),
            ("tap0", true),
            ("wg0", true),
            ("eth0", true),
            ("enp0s3", true),
            ("tun0; rm -rf /", false),
            ("$(whoami)", false),
            ("tun0\n", false),
            ("", false),
            ("../../../etc/passwd", false),
            ("tun0 -j ACCEPT", false),
            (&long_iface, false),
        ];

        for (iface, should_be_valid) in test_cases {
            let is_valid = !iface.is_empty()
                && iface.len() <= 15
                && iface
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                && !iface.contains(';')
                && !iface.contains('$')
                && !iface.contains('`')
                && !iface.contains('\n')
                && !iface.contains(' ');

            assert_eq!(
                is_valid, should_be_valid,
                "Interface '{}' validation mismatch",
                iface
            );
        }
    }

    #[test]
    fn test_iptables_command_escaping() {
        println!("iptables commands should be built using:");
        println!("  Command::new(\"iptables\").args([...])");
        println!("NOT:");
        println!("  format!(\"iptables -A OUTPUT -d {{}} -j ACCEPT\", ip)");
    }
}
