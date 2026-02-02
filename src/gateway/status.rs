//! Gateway status reporting.

use super::detect::{
    detect_lan_interface, detect_vpn_interface, get_interface_ip, get_interface_subnet,
};
use super::firewall::{get_forward_rules, is_forward_killswitch_active};
use super::{is_enabled, is_forwarding_enabled};
use std::fmt;

/// Gateway status information.
#[derive(Debug, Clone)]
pub struct GatewayStatus {
    /// Whether gateway mode is enabled
    pub enabled: bool,
    /// Whether IP forwarding is enabled
    pub forwarding_enabled: bool,
    /// Detected LAN interface
    pub lan_interface: Option<String>,
    /// LAN interface IP address
    pub lan_ip: Option<String>,
    /// LAN subnet
    pub lan_subnet: Option<String>,
    /// Detected VPN interface
    pub vpn_interface: Option<String>,
    /// VPN interface IP address
    pub vpn_ip: Option<String>,
    /// Whether forward kill switch is active
    pub kill_switch_active: bool,
    /// Current FORWARD chain rules
    pub forward_rules: Vec<String>,
}

impl GatewayStatus {
    /// Collect current gateway status.
    pub fn collect() -> Self {
        let lan_interface = detect_lan_interface();
        let vpn_interface = detect_vpn_interface();

        let lan_ip = lan_interface.as_ref().and_then(|i| get_interface_ip(i));
        let lan_subnet = lan_interface.as_ref().and_then(|i| get_interface_subnet(i));
        let vpn_ip = vpn_interface.as_ref().and_then(|i| get_interface_ip(i));

        Self {
            enabled: is_enabled(),
            forwarding_enabled: is_forwarding_enabled(),
            lan_interface,
            lan_ip,
            lan_subnet,
            vpn_interface,
            vpn_ip,
            kill_switch_active: is_forward_killswitch_active(),
            forward_rules: get_forward_rules(),
        }
    }
}

impl fmt::Display for GatewayStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Gateway Status")?;
        writeln!(f, "==============")?;
        writeln!(f)?;

        let enabled_str = if self.enabled {
            "✓ enabled"
        } else {
            "✗ disabled"
        };
        writeln!(f, "Gateway:           {}", enabled_str)?;

        let fwd_str = if self.forwarding_enabled {
            "✓ enabled"
        } else {
            "✗ disabled"
        };
        writeln!(f, "IP Forwarding:     {}", fwd_str)?;

        let ks_str = if self.kill_switch_active {
            "✓ active"
        } else {
            "✗ inactive"
        };
        writeln!(f, "Forward Kill SW:   {}", ks_str)?;

        writeln!(f)?;

        writeln!(f, "LAN Interface")?;
        writeln!(f, "-------------")?;
        if let Some(ref iface) = self.lan_interface {
            writeln!(f, "  Interface:       {}", iface)?;
            if let Some(ref ip) = self.lan_ip {
                writeln!(f, "  IP Address:      {}", ip)?;
            }
            if let Some(ref subnet) = self.lan_subnet {
                writeln!(f, "  Subnet:          {}", subnet)?;
            }
        } else {
            writeln!(f, "  Not detected")?;
        }

        writeln!(f)?;

        writeln!(f, "VPN Interface")?;
        writeln!(f, "-------------")?;
        if let Some(ref iface) = self.vpn_interface {
            writeln!(f, "  Interface:       {}", iface)?;
            if let Some(ref ip) = self.vpn_ip {
                writeln!(f, "  IP Address:      {}", ip)?;
            }
        } else {
            writeln!(f, "  Not detected (VPN not connected?)")?;
        }

        if !self.forward_rules.is_empty() && self.enabled {
            writeln!(f)?;
            writeln!(f, "FORWARD Chain Rules")?;
            writeln!(f, "-------------------")?;
            for rule in &self.forward_rules {
                writeln!(f, "  {}", rule)?;
            }
        }

        Ok(())
    }
}
