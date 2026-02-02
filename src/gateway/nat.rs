//! NAT (Network Address Translation) for gateway mode.
//!
//! Implements MASQUERADE rules so LAN clients can use the VPN tunnel.

use super::GatewayError;
use crate::killswitch::paths::{ip6tables, iptables};
use log::{debug, info};
use std::process::Command;

/// Enable NAT for the VPN interface.
pub fn enable_nat(vpn_interface: &str) -> Result<(), GatewayError> {
    info!("Enabling NAT on interface: {}", vpn_interface);

    // IPv4 MASQUERADE
    run_iptables(&[
        "-t",
        "nat",
        "-A",
        "POSTROUTING",
        "-o",
        vpn_interface,
        "-j",
        "MASQUERADE",
    ])?;
    debug!("IPv4 MASQUERADE enabled on {}", vpn_interface);

    // IPv6 MASQUERADE (best effort)
    let _ = run_ip6tables(&[
        "-t",
        "nat",
        "-A",
        "POSTROUTING",
        "-o",
        vpn_interface,
        "-j",
        "MASQUERADE",
    ]);
    debug!("IPv6 MASQUERADE enabled on {}", vpn_interface);

    Ok(())
}

/// Disable NAT rules.
pub fn disable_nat() -> Result<(), GatewayError> {
    info!("Disabling NAT");

    // Remove all MASQUERADE rules (brute force cleanup)
    loop {
        let result = run_iptables(&["-t", "nat", "-D", "POSTROUTING", "-j", "MASQUERADE"]);
        if result.is_err() {
            break;
        }
    }

    loop {
        let result = run_ip6tables(&["-t", "nat", "-D", "POSTROUTING", "-j", "MASQUERADE"]);
        if result.is_err() {
            break;
        }
    }

    Ok(())
}

/// Disable NAT for a specific interface.
#[allow(dead_code)]
pub fn disable_nat_for_interface(vpn_interface: &str) -> Result<(), GatewayError> {
    info!("Disabling NAT for interface: {}", vpn_interface);

    let _ = run_iptables(&[
        "-t",
        "nat",
        "-D",
        "POSTROUTING",
        "-o",
        vpn_interface,
        "-j",
        "MASQUERADE",
    ]);
    let _ = run_ip6tables(&[
        "-t",
        "nat",
        "-D",
        "POSTROUTING",
        "-o",
        vpn_interface,
        "-j",
        "MASQUERADE",
    ]);

    Ok(())
}

fn run_iptables(args: &[&str]) -> Result<(), GatewayError> {
    let output = Command::new("sudo")
        .arg(iptables())
        .args(args)
        .output()
        .map_err(|e| GatewayError::Nat(format!("Failed to run iptables: {}", e)))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GatewayError::Nat(format!(
            "iptables {} failed: {}",
            args.join(" "),
            stderr.trim()
        )))
    }
}

fn run_ip6tables(args: &[&str]) -> Result<(), GatewayError> {
    let output = Command::new("sudo")
        .arg(ip6tables())
        .args(args)
        .output()
        .map_err(|e| GatewayError::Nat(format!("Failed to run ip6tables: {}", e)))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GatewayError::Nat(format!(
            "ip6tables {} failed: {}",
            args.join(" "),
            stderr.trim()
        )))
    }
}
