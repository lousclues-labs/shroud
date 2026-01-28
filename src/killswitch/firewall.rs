//! nftables-based VPN kill switch
//!
//! Uses a dedicated nftables table to block all traffic except:
//! - Traffic through VPN tunnel interfaces (tun*, wg*)
//! - Traffic to the VPN server IP (to establish connection)
//! - Local loopback traffic
//! - Established/related connections
//!
//! The kill switch uses a separate table "shroud_killswitch" to avoid
//! interfering with other firewall rules.
//!
//! ## DNS Leak Protection
//!
//! Controlled by `dns_mode` config:
//! - `tunnel`: DNS only via VPN tunnel interfaces (most secure)
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

/// Errors that can occur during kill switch operations.
#[derive(Error, Debug)]
pub enum KillSwitchError {
    /// nftables is not installed or not in PATH
    #[error("nftables (nft) is not available. Install with: sudo apt install nftables")]
    NftablesNotFound,

    /// Permission denied - need elevated privileges
    #[error("Permission denied. Kill switch requires root privileges via pkexec.")]
    PermissionDenied,

    /// Failed to spawn nft/pkexec process
    #[error("Failed to spawn nft process: {0}")]
    SpawnFailed(#[source] std::io::Error),

    /// nft command returned error
    #[error("nft command failed: {0}")]
    CommandFailed(String),

    /// Failed to write to nft stdin
    #[error("Failed to write rules to nft: {0}")]
    WriteFailed(#[source] std::io::Error),

    /// Failed waiting for nft process
    #[error("Failed waiting for nft process: {0}")]
    WaitFailed(#[source] std::io::Error),
}
use tokio::process::Command;

use crate::config::{DnsMode, Ipv6Mode};

/// Name of the nftables table for the kill switch
const NFT_TABLE: &str = "shroud_killswitch";

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

/// VPN Kill Switch using nftables
pub struct KillSwitch {
    /// Whether the kill switch is currently enabled
    enabled: bool,
    /// Current VPN server IP (allowed through even when kill switch is on)
    vpn_server_ip: Option<IpAddr>,
    /// VPN tunnel interface name (e.g., "tun0")
    vpn_interface: Option<String>,
    /// DNS leak protection mode
    dns_mode: DnsMode,
    /// IPv6 leak protection mode
    ipv6_mode: Ipv6Mode,
}

/// Build the nftables ruleset string (pure function for testing)
fn build_ruleset(
    dns_mode: DnsMode,
    ipv6_mode: Ipv6Mode,
    vpn_iface: &str,
    vpn_server_ips: &[IpAddr],
    current_vpn_server: Option<IpAddr>,
) -> String {
    let mut rules = format!(
        r#"
table inet {table} {{
    chain output {{
        type filter hook output priority 0; policy drop;
        
        # === LOOPBACK ===
        # Always allow loopback (both IPv4 and IPv6)
        oifname "lo" accept
        
        # === ESTABLISHED/RELATED ===
        # Allow responses to existing connections
        ct state established,related accept
"#,
        table = NFT_TABLE
    );

    // === IPv6 HANDLING ===
    match ipv6_mode {
        Ipv6Mode::Block => {
            rules.push_str(
                r#"
        # === IPv6 LEAK PROTECTION (block mode) ===
        # Drop all IPv6 traffic except loopback (already accepted above)
        # This prevents IPv6 leaks when VPN doesn't tunnel IPv6
        meta nfproto ipv6 drop
"#,
            );
        }
        Ipv6Mode::Tunnel => {
            rules.push_str(
                r#"
        # === IPv6 LEAK PROTECTION (tunnel mode) ===
        # IPv6 only allowed via VPN tunnel interfaces
        # Link-local for neighbor discovery is allowed
        ip6 daddr fe80::/10 accept
"#,
            );
        }
        Ipv6Mode::Off => {
            rules.push_str(
                r#"
        # === IPv6 (off - no special handling) ===
        # WARNING: IPv6 may leak outside VPN tunnel
        # Allow IPv6 link-local for basic functionality
        ip6 daddr fe80::/10 accept
"#,
            );
        }
    }

    // === DHCP ===
    rules.push_str(
        r#"
        # === DHCP ===
        # Allow DHCP for network configuration
        udp dport 67 accept
        udp sport 68 accept
"#,
    );

    // === DNS HANDLING ===
    match dns_mode {
        DnsMode::Tunnel => {
            rules.push_str(
                r#"
        # === DNS LEAK PROTECTION (tunnel mode) ===
        # DNS is ONLY allowed via VPN tunnel interfaces
        # No explicit DNS rules here - tunnel interface accept rules below handle it
        # This is the most secure option
"#,
            );
        }
        DnsMode::Localhost => {
            rules.push_str(
                r#"
        # === DNS LEAK PROTECTION (localhost mode) ===
        # DNS only to local resolver (systemd-resolved, dnsmasq, etc.)
        ip daddr 127.0.0.0/8 udp dport 53 accept
        ip daddr 127.0.0.0/8 tcp dport 53 accept
        # systemd-resolved stub listener
        ip daddr 127.0.0.53 udp dport 53 accept
        ip daddr 127.0.0.53 tcp dport 53 accept
"#,
            );
            if ipv6_mode != Ipv6Mode::Block {
                rules.push_str(
                    r#"
        ip6 daddr ::1 udp dport 53 accept
        ip6 daddr ::1 tcp dport 53 accept
"#,
                );
            }
        }
        DnsMode::Any => {
            rules.push_str(
                r#"
        # === DNS (any mode - LEGACY/INSECURE) ===
        # WARNING: DNS can leak to any destination outside VPN!
        # This mode is provided for compatibility only
        udp dport 53 accept
        tcp dport 53 accept
"#,
            );
        }
    }

    // === LOCAL NETWORK ===
    rules.push_str(
        r#"
        # === LOCAL NETWORK ===
        # Allow local network access (prevents lockout, printers, etc.)
        ip daddr 192.168.0.0/16 accept
        ip daddr 10.0.0.0/8 accept
        ip daddr 172.16.0.0/12 accept
"#,
    );

    // === VPN TUNNEL INTERFACES ===
    rules.push_str(&format!(
        r#"
        # === VPN TUNNEL INTERFACES ===
        # Allow all traffic through VPN tunnels
        oifname "{vpn_iface}" accept
        oifname "tap*" accept
        oifname "wg*" accept
"#,
        vpn_iface = vpn_iface
    ));

    // === VPN SERVER IPs ===
    if !vpn_server_ips.is_empty() {
        rules.push_str("\n        # === VPN SERVER ALLOWLIST ===\n");
        rules.push_str("        # Allow traffic to VPN servers for connection establishment\n");
    }
    for ip in vpn_server_ips {
        match ip {
            IpAddr::V4(v4) => {
                rules.push_str(&format!("        ip daddr {} accept  # VPN server\n", v4));
            }
            IpAddr::V6(v6) => {
                if ipv6_mode != Ipv6Mode::Block {
                    rules.push_str(&format!("        ip6 daddr {} accept  # VPN server\n", v6));
                }
            }
        }
    }

    // Also add the currently configured VPN server if set
    if let Some(ip) = current_vpn_server {
        match ip {
            IpAddr::V4(v4) => {
                rules.push_str(&format!(
                    "        ip daddr {} accept  # Current VPN server\n",
                    v4
                ));
            }
            IpAddr::V6(v6) => {
                if ipv6_mode != Ipv6Mode::Block {
                    rules.push_str(&format!(
                        "        ip6 daddr {} accept  # Current VPN server\n",
                        v6
                    ));
                }
            }
        }
    }

    // === DROP AND LOG ===
    rules.push_str(
        r#"
        # === DEFAULT DROP ===
        # Log and drop everything else (rate limited to prevent log spam)
        limit rate 1/second log prefix "[VPN-KS DROP] " drop
    }
    
    # === INPUT CHAIN ===
    # More permissive - kill switch is primarily about preventing outbound leaks
    chain input {
        type filter hook input priority 0; policy accept;
        # Accept all input - we're focused on OUTPUT leak prevention
    }
}
"#,
    );

    rules
}

impl KillSwitch {
    /// Create a new kill switch instance with default (secure) settings
    pub fn new() -> Self {
        Self {
            enabled: false,
            vpn_server_ip: None,
            vpn_interface: None,
            dns_mode: DnsMode::default(),
            ipv6_mode: Ipv6Mode::default(),
        }
    }

    /// Create a kill switch with specific DNS and IPv6 modes
    pub fn with_config(dns_mode: DnsMode, ipv6_mode: Ipv6Mode) -> Self {
        Self {
            enabled: false,
            vpn_server_ip: None,
            vpn_interface: None,
            dns_mode,
            ipv6_mode,
        }
    }

    /// Update configuration (DNS and IPv6 modes)
    pub fn set_config(&mut self, dns_mode: DnsMode, ipv6_mode: Ipv6Mode) {
        self.dns_mode = dns_mode;
        self.ipv6_mode = ipv6_mode;
    }

    /// Check if nftables is available
    pub async fn is_available() -> bool {
        Command::new("nft")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check if we have permission to modify nftables
    pub async fn has_permission() -> bool {
        // Try to list tables - this will fail if we don't have permission
        Command::new("nft")
            .args(["list", "tables"])
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

    /// Enable the kill switch
    ///
    /// Creates nftables rules that:
    /// 1. Allow loopback traffic
    /// 2. Allow established/related connections
    /// 3. Allow traffic to VPN server IPs (to establish connection)
    /// 4. Allow traffic through VPN tunnel interface
    /// 5. Drop everything else
    pub async fn enable(&mut self) -> Result<(), KillSwitchError> {
        if self.enabled {
            debug!("Kill switch already enabled");
            return Ok(());
        }

        info!("Enabling VPN kill switch");

        // Auto-detect VPN server IPs from NetworkManager configs
        let vpn_server_ips = Self::detect_all_vpn_server_ips().await;
        if !vpn_server_ips.is_empty() {
            info!(
                "Detected {} VPN server IPs to whitelist",
                vpn_server_ips.len()
            );
        }

        // First, ensure any old rules are cleaned up
        let _ = self.cleanup_table().await;

        // Create the kill switch table and chains
        self.create_table_with_servers(&vpn_server_ips).await?;

        self.enabled = true;
        info!("VPN kill switch enabled");
        Ok(())
    }

    /// Disable the kill switch
    pub async fn disable(&mut self) -> Result<(), KillSwitchError> {
        if !self.enabled {
            debug!("Kill switch already disabled");
            return Ok(());
        }

        info!("Disabling VPN kill switch");
        self.cleanup_table().await?;
        self.enabled = false;
        info!("VPN kill switch disabled");
        Ok(())
    }

    /// Update the kill switch rules (e.g., when VPN interface changes)
    pub async fn update(&mut self) -> Result<(), KillSwitchError> {
        if !self.enabled {
            return Ok(());
        }

        debug!("Updating kill switch rules");
        let vpn_server_ips = Self::detect_all_vpn_server_ips().await;
        let _ = self.cleanup_table().await;
        self.create_table_with_servers(&vpn_server_ips).await
    }

    /// Create the nftables table and rules with allowed VPN server IPs
    async fn create_table_with_servers(&self, vpn_server_ips: &[IpAddr]) -> Result<(), KillSwitchError> {
        // Use wildcard for tun interfaces - nftables uses * for wildcard
        let vpn_iface = self.vpn_interface.as_deref().unwrap_or("tun*");

        // Build the nftables ruleset using the pure function
        let rules = build_ruleset(
            self.dns_mode,
            self.ipv6_mode,
            vpn_iface,
            vpn_server_ips,
            self.vpn_server_ip,
        );

        // Apply the rules
        self.run_nft(&["-f", "-"], Some(&rules)).await
    }

    /// Remove the kill switch table
    async fn cleanup_table(&self) -> Result<(), KillSwitchError> {
        // Delete the table if it exists (this removes all chains and rules)
        let result = self
            .run_nft(&["delete", "table", "inet", NFT_TABLE], None)
            .await;

        // Ignore "No such file or directory" errors (table doesn't exist)
        match result {
            Ok(_) => Ok(()),
            Err(KillSwitchError::CommandFailed(msg)) if msg.contains("No such file") || msg.contains("does not exist") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Run an nft command (via pkexec for GUI privilege escalation)
    async fn run_nft(&self, args: &[&str], stdin_data: Option<&str>) -> Result<(), KillSwitchError> {
        // Use pkexec for GUI password prompt instead of sudo (which blocks on TTY)
        let mut cmd = Command::new("pkexec");
        cmd.arg("nft");
        cmd.args(args);

        if stdin_data.is_some() {
            cmd.stdin(Stdio::piped());
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(KillSwitchError::SpawnFailed)?;

        if let Some(data) = stdin_data {
            use tokio::io::AsyncWriteExt;
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(data.as_bytes())
                    .await
                    .map_err(KillSwitchError::WriteFailed)?;
            }
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(KillSwitchError::WaitFailed)?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Check for specific error messages map to specific errors if needed
            if stderr.contains("not found") {
                // heuristic for nft not found although we called pkexec nft
                // actually pkexec would fail if nft not found?
            }
            if output.status.code() == Some(126) || output.status.code() == Some(127) {
                 use std::process::Command as StdCommand;
                 // Check if nft exists in path
                 if StdCommand::new("which").arg("nft").output().map(|o| !o.status.success()).unwrap_or(true) {
                     return Err(KillSwitchError::NftablesNotFound);
                 }
                 return Err(KillSwitchError::PermissionDenied); // pkexec failed / cancelled
            }
            Err(KillSwitchError::CommandFailed(stderr.trim().to_string()))
        }
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
            // Look for tun interfaces
            if line.contains("tun") && line.contains("state UP") {
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
    /// This allows the kill switch to permit traffic to VPN servers for connection establishment
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

        // Parse vpn.data which contains key=value pairs separated by commas
        // Format is: "remote = IP:PORT" or "remote = hostname:PORT"
        for line in stdout.lines() {
            if line.starts_with("vpn.data:") {
                let data = line.trim_start_matches("vpn.data:");
                for item in data.split(',') {
                    let item = item.trim();
                    // Handle both "remote=X" and "remote = X" formats
                    if item.starts_with("remote") {
                        // Split on '=' and get the value part
                        if let Some(value) = item.split('=').nth(1) {
                            let remote = value.trim();
                            // Remove port if present (format: "IP:PORT" or "hostname:PORT")
                            let host = if let Some(colon_pos) = remote.rfind(':') {
                                // Check if what's after colon is a port number
                                if remote[colon_pos + 1..].parse::<u16>().is_ok() {
                                    &remote[..colon_pos]
                                } else {
                                    remote
                                }
                            } else {
                                remote
                            };

                            // Try to parse as IP directly
                            if let Ok(ip) = host.parse::<IpAddr>() {
                                return Some(ip);
                            }
                            // If it's a hostname, try to resolve it
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
        // getent ahosts returns lines like: "1.2.3.4      STREAM hostname"
        // Take the first IPv4 address
        for line in stdout.lines() {
            if let Some(ip_str) = line.split_whitespace().next() {
                if let Ok(ip) = ip_str.parse::<IpAddr>() {
                    // Prefer IPv4
                    if ip.is_ipv4() {
                        return Some(ip);
                    }
                }
            }
        }

        None
    }
}

/// Synchronously clean up any stale kill switch rules
///
/// This is a standalone function that can be called from:
/// - Signal handlers (which are synchronous)
/// - Startup cleanup (before async runtime is available)
/// - Emergency cleanup
///
/// Uses blocking std::process::Command instead of tokio::process::Command
pub fn cleanup_stale_rules() {
    use std::process::{Command, Stdio};

    // Try to delete the kill switch table
    let result = Command::new("pkexec")
        .args(["nft", "delete", "table", "inet", NFT_TABLE])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match result {
        Ok(status) if status.success() => {
            info!("Cleaned up stale kill switch rules");
        }
        Ok(_) => {
            // Table doesn't exist, or permission denied - that's fine
        }
        Err(e) => {
            warn!("Failed to clean up kill switch rules: {}", e);
        }
    }
}

/// Check if kill switch rules exist (synchronous, for startup check)
pub fn rules_exist() -> bool {
    use std::process::{Command, Stdio};

    let result = Command::new("nft")
        .args(["list", "table", "inet", NFT_TABLE])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    matches!(result, Ok(status) if status.success())
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
            warn!("Run 'sudo nft delete table inet {}' to clean up", NFT_TABLE);
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
    fn test_kill_switch_set_interface() {
        let mut ks = KillSwitch::new();
        ks.set_vpn_interface(Some("tun0".to_string()));
        assert_eq!(ks.vpn_interface, Some("tun0".to_string()));
    }

    #[test]
    fn test_kill_switch_with_config() {
        let ks = KillSwitch::with_config(DnsMode::Localhost, Ipv6Mode::Tunnel);
        assert_eq!(ks.dns_mode, DnsMode::Localhost);
        assert_eq!(ks.ipv6_mode, Ipv6Mode::Tunnel);
    }

    #[test]
    fn test_kill_switch_set_config() {
        let mut ks = KillSwitch::new();
        assert_eq!(ks.dns_mode, DnsMode::Tunnel); // default
        assert_eq!(ks.ipv6_mode, Ipv6Mode::Block); // default

        ks.set_config(DnsMode::Any, Ipv6Mode::Off);
        assert_eq!(ks.dns_mode, DnsMode::Any);
        assert_eq!(ks.ipv6_mode, Ipv6Mode::Off);
    }

    // --- build_ruleset tests ---

    #[test]
    fn test_ruleset_contains_table_declaration() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[], None);
        assert!(rules.contains("table inet shroud_killswitch"));
    }

    #[test]
    fn test_ruleset_contains_loopback_accept() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[], None);
        assert!(rules.contains(r#"oifname "lo" accept"#));
    }

    #[test]
    fn test_ruleset_contains_established_related() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[], None);
        assert!(rules.contains("ct state established,related accept"));
    }

    #[test]
    fn test_ruleset_contains_dhcp() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[], None);
        assert!(rules.contains("udp dport 67 accept"));
        assert!(rules.contains("udp sport 68 accept"));
    }

    #[test]
    fn test_ruleset_contains_local_network() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[], None);
        assert!(rules.contains("ip daddr 192.168.0.0/16 accept"));
        assert!(rules.contains("ip daddr 10.0.0.0/8 accept"));
        assert!(rules.contains("ip daddr 172.16.0.0/12 accept"));
    }

    #[test]
    fn test_ruleset_contains_vpn_interface() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[], None);
        assert!(rules.contains(r#"oifname "tun0" accept"#));
        assert!(rules.contains(r#"oifname "tap*" accept"#));
        assert!(rules.contains(r#"oifname "wg*" accept"#));
    }

    #[test]
    fn test_ruleset_contains_drop_default() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[], None);
        assert!(rules.contains(r#"log prefix "[VPN-KS DROP] " drop"#));
    }

    // DNS mode tests

    #[test]
    fn test_dns_tunnel_mode_no_explicit_dns_rules() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[], None);
        // In tunnel mode, DNS goes through VPN interface, no explicit port 53 rules
        assert!(rules.contains("DNS is ONLY allowed via VPN tunnel"));
        // Should NOT have udp/tcp dport 53 accept outside of comments
        let lines: Vec<&str> = rules
            .lines()
            .filter(|l| !l.trim().starts_with('#'))
            .collect();
        let non_comment_rules = lines.join("\n");
        assert!(!non_comment_rules.contains("dport 53 accept"));
    }

    #[test]
    fn test_dns_localhost_mode_allows_local_dns() {
        let rules = build_ruleset(DnsMode::Localhost, Ipv6Mode::Off, "tun0", &[], None);
        assert!(rules.contains("ip daddr 127.0.0.0/8 udp dport 53 accept"));
        assert!(rules.contains("ip daddr 127.0.0.0/8 tcp dport 53 accept"));
        assert!(rules.contains("ip daddr 127.0.0.53 udp dport 53 accept"));
    }

    #[test]
    fn test_dns_localhost_mode_with_ipv6_block_no_ipv6_dns() {
        let rules = build_ruleset(DnsMode::Localhost, Ipv6Mode::Block, "tun0", &[], None);
        assert!(rules.contains("ip daddr 127.0.0.0/8 udp dport 53 accept"));
        // Should NOT have IPv6 DNS rules when IPv6 is blocked
        assert!(!rules.contains("ip6 daddr ::1 udp dport 53 accept"));
    }

    #[test]
    fn test_dns_localhost_mode_with_ipv6_tunnel_has_ipv6_dns() {
        let rules = build_ruleset(DnsMode::Localhost, Ipv6Mode::Tunnel, "tun0", &[], None);
        assert!(rules.contains("ip6 daddr ::1 udp dport 53 accept"));
        assert!(rules.contains("ip6 daddr ::1 tcp dport 53 accept"));
    }

    #[test]
    fn test_dns_any_mode_allows_all_dns() {
        let rules = build_ruleset(DnsMode::Any, Ipv6Mode::Block, "tun0", &[], None);
        assert!(rules.contains("udp dport 53 accept"));
        assert!(rules.contains("tcp dport 53 accept"));
        assert!(rules.contains("LEGACY/INSECURE"));
    }

    // IPv6 mode tests

    #[test]
    fn test_ipv6_block_mode_drops_ipv6() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[], None);
        assert!(rules.contains("meta nfproto ipv6 drop"));
    }

    #[test]
    fn test_ipv6_tunnel_mode_allows_link_local() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Tunnel, "tun0", &[], None);
        assert!(rules.contains("ip6 daddr fe80::/10 accept"));
        assert!(!rules.contains("meta nfproto ipv6 drop"));
    }

    #[test]
    fn test_ipv6_off_mode_allows_link_local() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Off, "tun0", &[], None);
        assert!(rules.contains("ip6 daddr fe80::/10 accept"));
        assert!(rules.contains("WARNING: IPv6 may leak"));
    }

    // VPN server allowlist tests

    #[test]
    fn test_vpn_server_ipv4_in_allowlist() {
        let ip: IpAddr = "203.0.113.1".parse().unwrap();
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[ip], None);
        assert!(rules.contains("ip daddr 203.0.113.1 accept"));
        assert!(rules.contains("VPN SERVER ALLOWLIST"));
    }

    #[test]
    fn test_vpn_server_ipv6_in_allowlist_when_not_blocked() {
        let ip: IpAddr = "2001:db8::1".parse().unwrap();
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Tunnel, "tun0", &[ip], None);
        assert!(rules.contains("ip6 daddr 2001:db8::1 accept"));
    }

    #[test]
    fn test_vpn_server_ipv6_not_in_allowlist_when_blocked() {
        let ip: IpAddr = "2001:db8::1".parse().unwrap();
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[ip], None);
        // IPv6 server should NOT be added when IPv6 is blocked
        assert!(!rules.contains("ip6 daddr 2001:db8::1 accept"));
    }

    #[test]
    fn test_current_vpn_server_in_allowlist() {
        let current: IpAddr = "198.51.100.1".parse().unwrap();
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[], Some(current));
        assert!(rules.contains("ip daddr 198.51.100.1 accept"));
        assert!(rules.contains("Current VPN server"));
    }

    #[test]
    fn test_multiple_vpn_servers_in_allowlist() {
        let ip1: IpAddr = "203.0.113.1".parse().unwrap();
        let ip2: IpAddr = "203.0.113.2".parse().unwrap();
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[ip1, ip2], None);
        assert!(rules.contains("ip daddr 203.0.113.1 accept"));
        assert!(rules.contains("ip daddr 203.0.113.2 accept"));
    }

    #[test]
    fn test_empty_vpn_server_list_no_allowlist_header() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun0", &[], None);
        assert!(!rules.contains("VPN SERVER ALLOWLIST"));
    }

    // Interface tests

    #[test]
    fn test_custom_vpn_interface() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "wg0", &[], None);
        assert!(rules.contains(r#"oifname "wg0" accept"#));
    }

    #[test]
    fn test_wildcard_vpn_interface() {
        let rules = build_ruleset(DnsMode::Tunnel, Ipv6Mode::Block, "tun*", &[], None);
        assert!(rules.contains(r#"oifname "tun*" accept"#));
    }
}
