// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Boot-time kill switch for headless mode.
//!
//! This module enables a minimal kill switch during system boot,
//! before the VPN connection is established. This ensures no traffic
//! leaks to the ISP during the startup window.

use super::paths::{ip6tables, iptables};
use super::KillSwitchError;
use std::process::Command;
use tracing::{info, warn};

const BOOT_CHAIN: &str = "SHROUD_BOOT_KS";

/// Enable the boot-time kill switch.
///
/// This is more restrictive than the runtime kill switch:
/// - Only allows loopback and (optionally) LAN
/// - Blocks everything else including DNS
/// - No VPN exceptions (tunnel doesn't exist yet)
///
/// # Errors
///
/// Returns [`KillSwitchError::Spawn`] if the iptables binary cannot be executed (missing binary or not in `$PATH`).
///
/// Returns [`KillSwitchError::Command`] if iptables exits with a non-zero status (e.g., missing `sudo` privileges).
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
/// Uses loop to remove ALL duplicate jump rules that may have accumulated.
///
/// # Errors
///
/// Returns [`KillSwitchError::Spawn`] if the iptables binary cannot be executed.
///
/// Returns [`KillSwitchError::Command`] if iptables exits with a non-zero status while removing rules.
pub fn disable_boot_killswitch() -> Result<(), KillSwitchError> {
    info!("Disabling boot kill switch");

    // Remove ALL jump rules (there may be duplicates from crashes/race conditions)
    // Loop until -D fails (meaning no more rules to delete)
    for _ in 0..100 {
        // Safety limit
        if run_iptables(&["-D", "OUTPUT", "-j", BOOT_CHAIN]).is_err() {
            break;
        }
    }
    for _ in 0..100 {
        if run_ip6tables(&["-D", "OUTPUT", "-j", BOOT_CHAIN]).is_err() {
            break;
        }
    }

    // Now flush and delete the chains
    let _ = run_iptables(&["-F", BOOT_CHAIN]);
    let _ = run_iptables(&["-X", BOOT_CHAIN]);
    let _ = run_ip6tables(&["-F", BOOT_CHAIN]);
    let _ = run_ip6tables(&["-X", BOOT_CHAIN]);

    info!("Boot kill switch disabled");
    Ok(())
}

/// Check if boot kill switch is active.
#[allow(dead_code)]
pub fn is_boot_killswitch_active() -> bool {
    run_iptables(&["-L", BOOT_CHAIN]).is_ok()
}

fn create_boot_chain() -> Result<(), KillSwitchError> {
    let result = run_iptables(&["-N", BOOT_CHAIN]);
    if result.is_err() {
        run_iptables(&["-F", BOOT_CHAIN])?;
    }

    // IPv6 rules are best-effort — systems without ip6tables (minimal
    // containers, disabled IPv6) will skip IPv6 protection silently.
    let result = run_ip6tables(&["-N", BOOT_CHAIN]);
    if result.is_err() && run_ip6tables(&["-F", BOOT_CHAIN]).is_err() {
        warn!("IPv6 boot kill switch chain could not be created or flushed — IPv6 traffic will not be blocked");
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

    // Allow LAN if configured — use detected subnets with RFC1918 fallback
    // (SHROUD-VULN-025: consistent with runtime kill switch)
    // NOTE: detect_local_subnets() is synchronous (shells out to `ip addr`).
    // This is intentional at boot — no async runtime is required here.
    if allow_lan {
        let subnets = crate::killswitch::rules::detect_local_subnets();
        for subnet in &subnets {
            if crate::killswitch::rules::is_valid_private_cidr(subnet) {
                run_iptables(&["-A", BOOT_CHAIN, "-d", subnet, "-j", "ACCEPT"])?;
            }
        }
        let _ = run_ip6tables(&["-A", BOOT_CHAIN, "-d", "fe80::/10", "-j", "ACCEPT"]);
    }

    // Drop everything else
    run_iptables(&["-A", BOOT_CHAIN, "-j", "DROP"])?;
    let _ = run_ip6tables(&["-A", BOOT_CHAIN, "-j", "DROP"]);

    Ok(())
}

fn insert_boot_chain_jump() -> Result<(), KillSwitchError> {
    // CRITICAL: First remove any existing jump rules to prevent duplicates
    // This handles race conditions where enable is called multiple times
    for _ in 0..10 {
        if run_iptables(&["-D", "OUTPUT", "-j", BOOT_CHAIN]).is_err() {
            break;
        }
    }
    for _ in 0..10 {
        if run_ip6tables(&["-D", "OUTPUT", "-j", BOOT_CHAIN]).is_err() {
            break;
        }
    }

    // Now insert fresh jump rules
    run_iptables(&["-I", "OUTPUT", "1", "-j", BOOT_CHAIN])?;
    let _ = run_ip6tables(&["-I", "OUTPUT", "1", "-j", BOOT_CHAIN]);
    Ok(())
}

fn run_iptables(args: &[&str]) -> Result<(), KillSwitchError> {
    // Use sudo -n to avoid password prompts that would cause hangs
    let output = Command::new("sudo")
        .arg("-n")
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
    // Use sudo -n to avoid password prompts that would cause hangs
    let output = Command::new("sudo")
        .arg("-n")
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
