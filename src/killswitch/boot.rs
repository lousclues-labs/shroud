//! Boot-time kill switch for headless mode.
//!
//! This module enables a minimal kill switch during system boot,
//! before the VPN connection is established. This ensures no traffic
//! leaks to the ISP during the startup window.

use super::paths::{ip6tables, iptables};
use super::KillSwitchError;
use log::info;
use std::process::Command;

const BOOT_CHAIN: &str = "SHROUD_BOOT_KS";

/// Enable the boot-time kill switch.
///
/// This is more restrictive than the runtime kill switch:
/// - Only allows loopback and (optionally) LAN
/// - Blocks everything else including DNS
/// - No VPN exceptions (tunnel doesn't exist yet)
pub fn enable_boot_killswitch(allow_lan: bool) -> Result<(), KillSwitchError> {
    info!("Enabling boot kill switch");

    create_boot_chain()?;
    add_boot_rules(allow_lan)?;
    insert_boot_chain_jump()?;

    info!("Boot kill switch enabled");
    Ok(())
}

/// Disable the boot kill switch.
///
/// Called when the full runtime kill switch takes over.
pub fn disable_boot_killswitch() -> Result<(), KillSwitchError> {
    info!("Disabling boot kill switch");

    let _ = run_iptables(&["-D", "OUTPUT", "-j", BOOT_CHAIN]);
    let _ = run_ip6tables(&["-D", "OUTPUT", "-j", BOOT_CHAIN]);
    let _ = run_iptables(&["-F", BOOT_CHAIN]);
    let _ = run_iptables(&["-X", BOOT_CHAIN]);
    let _ = run_ip6tables(&["-F", BOOT_CHAIN]);
    let _ = run_ip6tables(&["-X", BOOT_CHAIN]);

    info!("Boot kill switch disabled");
    Ok(())
}

/// Check if boot kill switch is active.
pub fn is_boot_killswitch_active() -> bool {
    run_iptables(&["-L", BOOT_CHAIN]).is_ok()
}

fn create_boot_chain() -> Result<(), KillSwitchError> {
    let result = run_iptables(&["-N", BOOT_CHAIN]);
    if result.is_err() {
        run_iptables(&["-F", BOOT_CHAIN])?;
    }

    let result = run_ip6tables(&["-N", BOOT_CHAIN]);
    if result.is_err() {
        let _ = run_ip6tables(&["-F", BOOT_CHAIN]);
    }

    Ok(())
}

fn add_boot_rules(allow_lan: bool) -> Result<(), KillSwitchError> {
    // Allow loopback
    run_iptables(&["-A", BOOT_CHAIN, "-o", "lo", "-j", "ACCEPT"])?;
    let _ = run_ip6tables(&["-A", BOOT_CHAIN, "-o", "lo", "-j", "ACCEPT"]);

    // Allow established
    run_iptables(&[
        "-A",
        BOOT_CHAIN,
        "-m",
        "state",
        "--state",
        "ESTABLISHED,RELATED",
        "-j",
        "ACCEPT",
    ])?;
    let _ = run_ip6tables(&[
        "-A",
        BOOT_CHAIN,
        "-m",
        "state",
        "--state",
        "ESTABLISHED,RELATED",
        "-j",
        "ACCEPT",
    ]);

    // Allow DHCP
    run_iptables(&[
        "-A", BOOT_CHAIN, "-p", "udp", "--dport", "67:68", "-j", "ACCEPT",
    ])?;

    // Allow LAN if configured
    if allow_lan {
        for network in &["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"] {
            run_iptables(&["-A", BOOT_CHAIN, "-d", network, "-j", "ACCEPT"])?;
        }
        let _ = run_ip6tables(&["-A", BOOT_CHAIN, "-d", "fe80::/10", "-j", "ACCEPT"]);
    }

    // Drop everything else
    run_iptables(&["-A", BOOT_CHAIN, "-j", "DROP"])?;
    let _ = run_ip6tables(&["-A", BOOT_CHAIN, "-j", "DROP"]);

    Ok(())
}

fn insert_boot_chain_jump() -> Result<(), KillSwitchError> {
    run_iptables(&["-I", "OUTPUT", "1", "-j", BOOT_CHAIN])?;
    let _ = run_ip6tables(&["-I", "OUTPUT", "1", "-j", BOOT_CHAIN]);
    Ok(())
}

fn run_iptables(args: &[&str]) -> Result<(), KillSwitchError> {
    let output = Command::new("sudo")
        .arg(iptables())
        .args(args)
        .output()
        .map_err(|e| KillSwitchError::Command(format!("Failed to run iptables: {}", e)))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(KillSwitchError::Command(format!(
            "iptables {} failed: {}",
            args.join(" "),
            stderr.trim()
        )))
    }
}

fn run_ip6tables(args: &[&str]) -> Result<(), KillSwitchError> {
    let output = Command::new("sudo")
        .arg(ip6tables())
        .args(args)
        .output()
        .map_err(|e| KillSwitchError::Command(format!("Failed to run ip6tables: {}", e)))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(KillSwitchError::Command(format!(
            "ip6tables {} failed: {}",
            args.join(" "),
            stderr.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boot_chain_name_is_different() {
        assert_ne!(BOOT_CHAIN, "SHROUD_KILLSWITCH");
    }
}
