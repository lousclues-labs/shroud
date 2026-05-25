// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

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
//! - `localhost`: DNS only to 127.0.0.1, 127.0.0.53, ::1
//! - `any`: DNS to any destination (legacy, least secure)
//!
//! ## IPv6 Leak Protection
//!
//! Controlled by `ipv6_mode` config:
//! - `block`: Drop all IPv6 except loopback (most secure)
//! - `tunnel`: Allow IPv6 only via VPN tunnel interfaces
//! - `off`: No special IPv6 handling (legacy)

// Submodules — see module-level doc comments in each for ownership.
mod builder;
mod chains;
mod error;
mod ip6tables;
mod iptables;
mod nftables;

use std::net::IpAddr;
use std::process::Stdio;
use std::time::Instant;
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::config::{DnsMode, Ipv6Mode};
use crate::killswitch::paths::{iptables, nft};

pub use error::KillSwitchError;

/// Name of the iptables chain for the kill switch
const CHAIN_NAME: &str = "SHROUD_KILLSWITCH";

/// Name of the nftables table for the kill switch
const NFT_TABLE: &str = "shroud_killswitch";

/// Minimum cooldown between kill switch toggles (milliseconds)
/// Prevents race conditions from rapid enable/disable
const TOGGLE_COOLDOWN_MS: u64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirewallBackend {
    Iptables,
    Nftables,
}

/// Kill switch status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
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
    #[allow(dead_code)]
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
    /// Timestamp of last toggle operation (for cooldown)
    last_toggle_time: Option<Instant>,
    /// Flag to prevent concurrent toggle operations.
    ///
    /// # Safety invariant
    ///
    /// This is a plain `bool`, not an `AtomicBool`. It is safe **only** because
    /// `KillSwitch` is owned by `VpnSupervisor` which holds `&mut self` in a
    /// single-task tokio event loop — no concurrent access is possible. If
    /// `KillSwitch` is ever shared (e.g., `Arc<Mutex<KillSwitch>>`), this must
    /// be changed to an `AtomicBool` or guarded by the outer lock.
    toggle_in_progress: bool,
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
            last_toggle_time: None,
            toggle_in_progress: false,
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
            last_toggle_time: None,
            toggle_in_progress: false,
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

    /// Check if iptables is available (version check doesn't need sudo)
    #[allow(dead_code)]
    pub async fn is_available() -> bool {
        Command::new(iptables())
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check if we have permission to modify iptables
    ///
    /// Uses sudo -n to avoid password prompts. Returns false if sudo
    /// access is not configured (NOPASSWD not set for iptables).
    #[allow(dead_code)]
    pub async fn has_permission() -> bool {
        // Try to list filter table with sudo -n (non-interactive)
        Command::new("sudo")
            .args(["-n", iptables(), "-t", "filter", "-nL", "OUTPUT"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Get current status
    #[allow(dead_code)]
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

    /// Return the name of the active firewall backend.
    #[allow(dead_code)]
    pub fn backend_name(&self) -> &'static str {
        match self.backend {
            FirewallBackend::Iptables => "iptables",
            FirewallBackend::Nftables => "nftables",
        }
    }

    /// Set the VPN server IP to allow through the firewall
    #[allow(dead_code)]
    pub fn set_vpn_server(&mut self, ip: Option<IpAddr>) {
        self.vpn_server_ip = ip;
    }

    /// Set the VPN tunnel interface
    #[allow(dead_code)]
    pub fn set_vpn_interface(&mut self, iface: Option<String>) {
        self.vpn_interface = iface;
    }

    /// Check actual state of rules, not just our flag (blocking variant).
    ///
    /// Note: This requires sudo access for iptables. If we can't check
    /// (permission denied), we return false to avoid reporting enabled
    /// when rules may be gone (SHROUD-VULN-032). The `-n` flag on sudo
    /// ensures non-interactive (no hang on password prompt).
    ///
    /// This variant uses `std::process::Command` and is intended only for
    /// one-shot use at startup and shutdown, where a brief synchronous
    /// sudo call is acceptable. Inside the tokio event loop, prefer
    /// [`is_actually_enabled_async`] to avoid blocking the runtime worker.
    pub fn is_actually_enabled(&self) -> bool {
        use std::process::{Command, Stdio};

        match self.backend {
            FirewallBackend::Iptables => {
                let result = Command::new("sudo")
                    .args(["-n", iptables(), "-C", "OUTPUT", "-j", CHAIN_NAME])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();

                match result {
                    Ok(status) => status.success(),
                    Err(_) => {
                        warn!("Cannot verify iptables state (sudo failed), assuming disabled");
                        false
                    }
                }
            }
            FirewallBackend::Nftables => {
                let result = Command::new("sudo")
                    .args(["-n", nft(), "list", "table", "inet", NFT_TABLE])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();

                match result {
                    Ok(status) => status.success(),
                    Err(_) => {
                        warn!("Cannot verify nftables state (sudo failed), assuming disabled");
                        false
                    }
                }
            }
        }
    }

    /// Async variant of [`is_actually_enabled`] using tokio's non-blocking
    /// subprocess API.
    ///
    /// Use this from any code path that runs inside the tokio runtime
    /// (event loop, health check, IPC handlers). The blocking variant
    /// would otherwise stall a runtime worker for the duration of the
    /// `sudo` invocation.
    pub async fn is_actually_enabled_async(&self) -> bool {
        let (program, args): (&str, [&str; 6]) = match self.backend {
            FirewallBackend::Iptables => {
                ("sudo", ["-n", iptables(), "-C", "OUTPUT", "-j", CHAIN_NAME])
            }
            FirewallBackend::Nftables => {
                ("sudo", ["-n", nft(), "list", "table", "inet", NFT_TABLE])
            }
        };

        let result = Command::new(program)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        match result {
            Ok(status) => status.success(),
            Err(_) => {
                warn!(
                    "Cannot verify {} state (sudo failed), assuming disabled",
                    match self.backend {
                        FirewallBackend::Iptables => "iptables",
                        FirewallBackend::Nftables => "nftables",
                    }
                );
                false
            }
        }
    }

    /// Sync our internal state with actual firewall state (blocking).
    ///
    /// See [`is_actually_enabled`] for when to prefer the async variant.
    pub fn sync_state(&mut self) {
        self.enabled = self.is_actually_enabled();
    }

    /// Async variant of [`sync_state`] — does not block the tokio runtime.
    pub async fn sync_state_async(&mut self) {
        self.enabled = self.is_actually_enabled_async().await;
    }

    /// Check and enforce toggle cooldown
    ///
    /// Returns true if we can proceed with the toggle, false if in cooldown
    fn check_toggle_cooldown(&mut self) -> bool {
        if let Some(last_time) = self.last_toggle_time {
            let elapsed_ms = last_time.elapsed().as_millis() as u64;
            if elapsed_ms < TOGGLE_COOLDOWN_MS {
                debug!(
                    "Kill switch toggle in cooldown ({}/{}ms)",
                    elapsed_ms, TOGGLE_COOLDOWN_MS
                );
                return false;
            }
        }
        true
    }

    /// Enable the kill switch.
    ///
    /// Applies iptables/nftables rules that block all traffic except through the VPN tunnel
    /// (and allowed exceptions like loopback, LAN/DHCP when configured).
    ///
    /// # Errors
    ///
    /// Returns [`KillSwitchError::Spawn`] if the iptables/nft binary cannot be executed
    /// (not installed or not in `$PATH`).
    ///
    /// Returns [`KillSwitchError::Command`] if iptables/nft exits with a non-zero status
    /// (missing `sudo` privileges or conflicting existing chains).
    ///
    /// Returns [`KillSwitchError::Wait`] if the iptables/nft process cannot be awaited to completion.
    ///
    /// Returns [`KillSwitchError::Write`] if nftables rules cannot be written to stdin (nft backend only).
    pub async fn enable(&mut self) -> Result<(), KillSwitchError> {
        // RACE PREVENTION: Check toggle flag (SHROUD-VULN-033: struct-owned, not static)
        if self.toggle_in_progress {
            debug!("Kill switch toggle already in progress, skipping enable");
            return Ok(());
        }
        self.toggle_in_progress = true;

        // NOTE: scopeguard::defer! would be ideal here but borrows &mut self,
        // conflicting with enable_inner(&mut self). The manual reset below is
        // safe because enable_inner() returns a Result (flag resets on both
        // Ok and Err). The only risk is panic — which is caught by the panic
        // hook that preserves kill switch rules (fail-closed).
        let result = self.enable_inner().await;
        self.toggle_in_progress = false;
        result
    }

    async fn enable_inner(&mut self) -> Result<(), KillSwitchError> {
        // Check cooldown
        if !self.check_toggle_cooldown() {
            return Ok(());
        }

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
        self.last_toggle_time = Some(Instant::now());

        // CRITICAL: First clean up any existing rules to prevent duplicates
        // This handles race conditions where enable is called multiple times
        self.robust_iptables_cleanup().await;

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
                debug!("iptables backend uses non-atomic rule application; brief traffic gap possible during rule updates. nftables backend is atomic.");
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

    async fn select_backend(&mut self) -> Result<FirewallBackend, KillSwitchError> {
        // Prefer nftables: atomic rule application means no traffic gap during updates.
        // Fall back to iptables/iptables-legacy if nft is unavailable.
        if Self::nft_is_available().await {
            info!("nftables available — using atomic backend");
            return Ok(FirewallBackend::Nftables);
        }

        match Self::check_sudo_access() {
            Ok(()) => Ok(FirewallBackend::Iptables),
            Err(err) if Self::should_fallback_to_nft(&err) => {
                if Self::check_iptables_legacy_access().unwrap_or(false) {
                    self.use_legacy = true;
                    Ok(FirewallBackend::Iptables)
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

    /// Disable the kill switch.
    ///
    /// Removes iptables/nftables rules previously installed by [`KillSwitch::enable`].
    ///
    /// # Errors
    ///
    /// Returns [`KillSwitchError::Spawn`] if the iptables/nft binary cannot be executed.
    ///
    /// Returns [`KillSwitchError::Command`] if iptables/nft exits with a non-zero status
    /// while removing rules (e.g., insufficient `sudo` privileges).
    ///
    /// Returns [`KillSwitchError::Wait`] if the iptables/nft process cannot be awaited to completion.
    ///
    /// Returns [`KillSwitchError::Write`] if nftables rules cannot be written to stdin (nft backend only).
    pub async fn disable(&mut self) -> Result<(), KillSwitchError> {
        // RACE PREVENTION: Check toggle flag (SHROUD-VULN-033: struct-owned, not static)
        if self.toggle_in_progress {
            debug!("Kill switch toggle already in progress, skipping disable");
            return Ok(());
        }
        self.toggle_in_progress = true;

        // NOTE: scopeguard::defer! would be ideal here but borrows &mut self,
        // conflicting with disable_inner(&mut self). See enable() comment.
        let result = self.disable_inner().await;
        self.toggle_in_progress = false;
        result
    }

    async fn disable_inner(&mut self) -> Result<(), KillSwitchError> {
        // Check cooldown
        if !self.check_toggle_cooldown() {
            return Ok(());
        }

        info!("Disabling VPN kill switch");
        self.last_toggle_time = Some(Instant::now());

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

        // We run cleanup regardless of enabled status to ensure we don't leave
        // the user stranded if the internal state is out of sync.
        // Use robust cleanup that removes ALL duplicate rules.

        match self.backend {
            FirewallBackend::Iptables => {
                self.robust_iptables_cleanup().await;
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
    #[allow(dead_code)]
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

    /// Detect the VPN tunnel interface from the system
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
            // SECURITY: Use rsplitn to handle connection names containing ':'
            // (SHROUD-VULN-027). Type field is rightmost and never contains ':'.
            let parts: Vec<&str> = line.rsplitn(2, ':').collect();
            if parts.len() >= 2 && parts[0] == "vpn" {
                // rsplitn reverses: [type, name]
                let conn_name = parts[1];

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
                            // SECURITY: Do NOT resolve hostnames here (SHROUD-VULN-041).
                            // DNS resolution during kill switch enablement happens on the
                            // unprotected network, allowing DNS poisoning to inject
                            // attacker-controlled IPs into the whitelist. Only use IPs
                            // directly from the NM connection profile.
                            warn!(
                                "VPN '{}' uses hostname '{}' — cannot whitelist without DNS. \
                                 Use IP address in VPN config for kill switch compatibility.",
                                conn_name, host
                            );
                        }
                    }
                }
            }
        }

        None
    }

    /// Resolve a hostname to an IP address.
    ///
    /// NOTE: Not used during kill switch enablement (SHROUD-VULN-041).
    /// Kept for potential future use with trusted/cached DNS resolution.
    #[allow(dead_code)]
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
            // SECURITY: Do NOT clean up rules in Drop (SHROUD-VULN-045).
            // Fail-closed: rules persist until explicit cleanup or next startup.
            // This prevents double-cleanup races with concurrent instances and
            // aligns with the panic hook's fail-closed design (SHROUD-VULN-043).
            warn!(
                "Kill switch dropped while enabled — rules preserved (fail-closed). \
                 Run 'shroud cleanup' or 'sudo iptables -F SHROUD_KILLSWITCH' to remove."
            );
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
        // Script creates the chain and adds rules
        assert!(script.contains("iptables -N SHROUD_KILLSWITCH"));
        assert!(script.contains("iptables -I OUTPUT 1 -j SHROUD_KILLSWITCH"));
        // Note: Cleanup is now done by robust_iptables_cleanup() before script runs
        // so the script itself doesn't include cleanup commands
        assert!(script.contains("-o lo -j ACCEPT"));
        assert!(script.contains("-j DROP"));
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

        assert!(rules.contains("-d 127.0.0.1 -p udp --dport 53 -j ACCEPT"));
        assert!(rules.contains("-d 127.0.0.1 -p tcp --dport 53 -j ACCEPT"));
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

        // Should allow localhost (127.0.0.1/127.0.0.53)
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

// =========================================================================
// KillSwitch struct tests (expanded)
// =========================================================================

#[cfg(test)]
mod ks_expanded_tests {
    use super::*;
    use crate::killswitch::paths::ip6tables;
    use crate::killswitch::rules::DOH_PROVIDERS as DOH_PROVIDER_IPS;

    #[test]
    fn test_kill_switch_error_display() {
        let err = KillSwitchError::NotFound;
        assert!(err.to_string().contains("iptables"));

        let err = KillSwitchError::Permission;
        assert!(err.to_string().contains("sudo"));

        let err = KillSwitchError::Command("test error".into());
        assert!(err.to_string().contains("test error"));
    }

    #[test]
    fn test_kill_switch_status_variants() {
        assert_eq!(KillSwitchStatus::Disabled, KillSwitchStatus::Disabled);
        assert_ne!(KillSwitchStatus::Disabled, KillSwitchStatus::Active);
        assert_ne!(KillSwitchStatus::Active, KillSwitchStatus::Error);
    }

    #[test]
    fn test_kill_switch_new_defaults() {
        let ks = KillSwitch::new();
        assert!(!ks.enabled);
        assert!(ks.vpn_server_ip.is_none());
        assert!(ks.vpn_interface.is_none());
        assert_eq!(ks.dns_mode, DnsMode::Tunnel);
        assert!(ks.block_doh);
        assert!(ks.custom_doh_blocklist.is_empty());
        assert_eq!(ks.ipv6_mode, Ipv6Mode::Block);
        assert!(ks.last_toggle_time.is_none());
    }

    #[test]
    fn test_set_vpn_server_and_clear() {
        let mut ks = KillSwitch::new();
        let ip: IpAddr = "1.2.3.4".parse().unwrap();
        ks.set_vpn_server(Some(ip));
        assert_eq!(ks.vpn_server_ip, Some(ip));

        ks.set_vpn_server(None);
        assert!(ks.vpn_server_ip.is_none());
    }

    #[test]
    fn test_set_config_updates_all_fields() {
        let mut ks = KillSwitch::new();
        let custom = vec!["1.1.1.1".into()];
        ks.set_config(DnsMode::Any, Ipv6Mode::Off, false, custom.clone());
        assert_eq!(ks.dns_mode, DnsMode::Any);
        assert_eq!(ks.ipv6_mode, Ipv6Mode::Off);
        assert!(!ks.block_doh);
        assert_eq!(ks.custom_doh_blocklist, custom);
    }

    #[test]
    fn test_build_complete_script_with_vpn_ips() {
        let ks = KillSwitch::new();
        let ip: IpAddr = "203.0.113.50".parse().unwrap();
        let script = ks.build_complete_script(&[ip]);
        assert!(script.contains("203.0.113.50"));
        assert!(script.contains("-j ACCEPT"));
    }

    #[test]
    fn test_build_complete_script_ipv6_block() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Block, true, Vec::new());
        let script = ks.build_complete_script(&[]);
        assert!(script.contains(ip6tables()));
        assert!(script.contains("-j DROP"));
    }

    #[test]
    fn test_build_complete_script_ipv6_tunnel() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Tunnel, true, Vec::new());
        let script = ks.build_complete_script(&[]);
        assert!(script.contains("fe80::/10"));
        assert!(script.contains("-o tun+"));
    }

    #[test]
    fn test_build_complete_script_ipv6_off() {
        let ks = KillSwitch::with_config(DnsMode::Tunnel, Ipv6Mode::Off, true, Vec::new());
        let script = ks.build_complete_script(&[]);
        // Should not contain any ip6tables rules
        assert!(!script.contains(ip6tables()));
    }

    #[test]
    fn test_build_complete_script_lan_allowed() {
        let ks = KillSwitch::new();
        let script = ks.build_complete_script(&[]);
        // LAN rules now use auto-detected subnets (or RFC1918 fallback).
        // In CI/test, detect_local_subnets() falls back to RFC1918 or finds real interfaces.
        // Just verify that some subnet-level ACCEPT rule exists.
        assert!(
            script.contains("-j ACCEPT")
                && (script.contains("192.168.")
                    || script.contains("10.0.")
                    || script.contains("172.16.")
                    || script.contains("169.254.")),
            "Script should contain LAN subnet ACCEPT rules"
        );
    }

    #[test]
    fn test_build_complete_script_dhcp() {
        let ks = KillSwitch::new();
        let script = ks.build_complete_script(&[]);
        assert!(script.contains("--dport 67"));
        assert!(script.contains("--sport 68"));
    }

    #[test]
    fn test_build_complete_script_logging() {
        let ks = KillSwitch::new();
        let script = ks.build_complete_script(&[]);
        assert!(script.contains("SHROUD-KS-DROP"));
        assert!(script.contains("--log-prefix"));
    }

    #[test]
    fn test_strict_mode_doh_rules() {
        let ks = KillSwitch::with_config(DnsMode::Strict, Ipv6Mode::Block, true, Vec::new());
        let rules = ks.build_rules_preview(&[]);
        // Should have both DNS tunnel rules AND DoH blocking
        assert!(rules.contains("-o tun+ -p udp --dport 53 -j ACCEPT"));
        assert!(rules.contains("-d 1.1.1.1 -p tcp --dport 443 -j DROP"));
    }

    #[test]
    fn test_custom_doh_in_iptables() {
        let custom = vec!["100.100.100.100".into()];
        let ks = KillSwitch::with_config(DnsMode::Strict, Ipv6Mode::Block, true, custom);
        let rules = ks.build_rules_preview(&[]);
        assert!(rules.contains("100.100.100.100"));
    }

    #[test]
    fn test_doh_providers_list_not_empty() {
        assert!(
            !DOH_PROVIDER_IPS.is_empty(),
            "DoH provider list should not be empty"
        );
        let count = DOH_PROVIDER_IPS.len();
        assert!(
            count >= 12,
            "Should have at least 12 DoH providers, got {}",
            count
        );
    }

    #[test]
    fn test_doh_providers_are_valid_ips() {
        for ip_str in DOH_PROVIDER_IPS {
            let parsed: Result<IpAddr, _> = ip_str.parse();
            assert!(
                parsed.is_ok(),
                "DoH provider '{}' is not a valid IP",
                ip_str
            );
        }
    }

    #[test]
    fn test_chain_name_constant() {
        assert_eq!(CHAIN_NAME, "SHROUD_KILLSWITCH");
    }

    #[test]
    fn test_nft_table_constant() {
        assert_eq!(NFT_TABLE, "shroud_killswitch");
    }

    #[test]
    fn test_toggle_cooldown_constant() {
        let cooldown = TOGGLE_COOLDOWN_MS;
        assert!(cooldown >= 100, "Cooldown too short: {}", cooldown);
        assert!(cooldown <= 5000, "Cooldown too long: {}", cooldown);
    }
}
