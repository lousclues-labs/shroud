//! nftables-based VPN kill switch
//!
//! Uses a dedicated nftables table to block all traffic except:
//! - Traffic through VPN tunnel interfaces (tun*)
//! - Traffic to the VPN server IP (to establish connection)
//! - Local loopback traffic
//! - Established/related connections
//!
//! The kill switch uses a separate table "vpn_killswitch" to avoid
//! interfering with other firewall rules.

#![allow(dead_code)]

use log::{debug, info, warn};
use std::net::IpAddr;
use std::process::Stdio;
use tokio::process::Command;

/// Name of the nftables table for the kill switch
const NFT_TABLE: &str = "vpn_killswitch";

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
}

impl KillSwitch {
    /// Create a new kill switch instance
    pub fn new() -> Self {
        Self {
            enabled: false,
            vpn_server_ip: None,
            vpn_interface: None,
        }
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
    pub async fn enable(&mut self) -> Result<(), String> {
        if self.enabled {
            debug!("Kill switch already enabled");
            return Ok(());
        }

        info!("Enabling VPN kill switch");

        // Auto-detect VPN server IPs from NetworkManager configs
        let vpn_server_ips = Self::detect_all_vpn_server_ips().await;
        if !vpn_server_ips.is_empty() {
            info!("Detected {} VPN server IPs to whitelist", vpn_server_ips.len());
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
    pub async fn disable(&mut self) -> Result<(), String> {
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
    pub async fn update(&mut self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }

        debug!("Updating kill switch rules");
        let vpn_server_ips = Self::detect_all_vpn_server_ips().await;
        let _ = self.cleanup_table().await;
        self.create_table_with_servers(&vpn_server_ips).await
    }

    /// Create the nftables table and rules with allowed VPN server IPs
    async fn create_table_with_servers(&self, vpn_server_ips: &[IpAddr]) -> Result<(), String> {
        // Use wildcard for tun interfaces - nftables uses * for wildcard
        let vpn_iface = self.vpn_interface.as_deref().unwrap_or("tun*");
        
        // Build the nftables ruleset
        let mut rules = format!(
            r#"
table inet {table} {{
    chain output {{
        type filter hook output priority 0; policy drop;
        
        # Allow loopback
        oifname "lo" accept
        
        # Allow established/related connections
        ct state established,related accept
        
        # Allow DHCP
        udp dport 67 accept
        udp sport 68 accept
        
        # Allow DNS to localhost (for local resolvers like systemd-resolved)
        ip daddr 127.0.0.0/8 accept
        ip6 daddr ::1 accept
        
        # Allow local network access (prevents lockout)
        ip daddr 192.168.0.0/16 accept
        ip daddr 10.0.0.0/8 accept
        ip daddr 172.16.0.0/12 accept
        ip6 daddr fe80::/10 accept
        
        # Allow DNS to VPN DNS servers (commonly in 10.x.x.x range)
        udp dport 53 accept
        tcp dport 53 accept
        
        # Allow traffic through VPN tunnel (tun/tap/wg interfaces)
        oifname "{vpn_iface}" accept
        oifname "tap*" accept
        oifname "wg*" accept
"#,
            table = NFT_TABLE,
            vpn_iface = vpn_iface
        );

        // Add rules for all known VPN server IPs (so VPN can establish connection)
        for ip in vpn_server_ips {
            match ip {
                IpAddr::V4(v4) => {
                    rules.push_str(&format!(
                        "        # Allow traffic to VPN server {}\n        ip daddr {} accept\n",
                        v4, v4
                    ));
                }
                IpAddr::V6(v6) => {
                    rules.push_str(&format!(
                        "        # Allow traffic to VPN server {}\n        ip6 daddr {} accept\n",
                        v6, v6
                    ));
                }
            }
        }

        // Also add the currently configured VPN server if set
        if let Some(ip) = self.vpn_server_ip {
            match ip {
                IpAddr::V4(v4) => {
                    rules.push_str(&format!(
                        "        # Allow traffic to current VPN server\n        ip daddr {} accept\n",
                        v4
                    ));
                }
                IpAddr::V6(v6) => {
                    rules.push_str(&format!(
                        "        # Allow traffic to current VPN server\n        ip6 daddr {} accept\n",
                        v6
                    ));
                }
            }
        }

        // Add input chain for incoming traffic (closing output chain first)
        rules.push_str(
            r#"
        # Log dropped packets (rate limited)
        limit rate 1/second log prefix "[VPN-KS DROP OUT] " drop
    }
    
    # Input chain: more permissive to avoid lockouts
    # We're primarily concerned with OUTPUT (preventing leaks)
    chain input {
        type filter hook input priority 0; policy accept;
        
        # Accept everything on input - kill switch is about preventing
        # outbound traffic leaks, not blocking incoming
    }
}
"#
        );

        // Apply the rules
        self.run_nft(&["-f", "-"], Some(&rules)).await
    }

    /// Remove the kill switch table
    async fn cleanup_table(&self) -> Result<(), String> {
        // Delete the table if it exists (this removes all chains and rules)
        let result = self.run_nft(&["delete", "table", "inet", NFT_TABLE], None).await;
        
        // Ignore "No such file or directory" errors (table doesn't exist)
        match result {
            Ok(_) => Ok(()),
            Err(e) if e.contains("No such file") || e.contains("does not exist") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Run an nft command (via pkexec for GUI privilege escalation)
    async fn run_nft(&self, args: &[&str], stdin_data: Option<&str>) -> Result<(), String> {
        // Use pkexec for GUI password prompt instead of sudo (which blocks on TTY)
        let mut cmd = Command::new("pkexec");
        cmd.arg("nft");
        cmd.args(args);
        
        if stdin_data.is_some() {
            cmd.stdin(Stdio::piped());
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn pkexec nft: {}", e))?;

        if let Some(data) = stdin_data {
            use tokio::io::AsyncWriteExt;
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(data.as_bytes())
                    .await
                    .map_err(|e| format!("Failed to write to nft stdin: {}", e))?;
            }
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| format!("Failed to wait for nft: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("nft command failed: {}", stderr.trim()))
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
}
