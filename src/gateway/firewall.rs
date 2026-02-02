//! FORWARD chain rules for gateway mode.
//!
//! Controls traffic flowing THROUGH the gateway (not to/from it).
//! Implements kill switch for forwarded traffic.

use super::GatewayError;
use crate::config::AllowedClients;
use crate::killswitch::paths::{ip6tables, iptables};
use log::info;
use std::process::Command;

const GATEWAY_CHAIN: &str = "SHROUD_GATEWAY";
const GATEWAY_KS_CHAIN: &str = "SHROUD_GATEWAY_KS";

/// Enable FORWARD rules for gateway traffic.
pub fn enable_forward_rules(
    lan_interface: &str,
    vpn_interface: &str,
    allowed_clients: &AllowedClients,
) -> Result<(), GatewayError> {
    info!("Enabling gateway forward rules");

    // Create gateway chain
    create_chain(GATEWAY_CHAIN)?;

    // Add rules based on allowed clients
    match allowed_clients {
        AllowedClients::All => {
            add_rule(
                GATEWAY_CHAIN,
                &["-i", lan_interface, "-o", vpn_interface, "-j", "ACCEPT"],
            )?;
        }
        AllowedClients::Cidr(cidr) => {
            add_rule(
                GATEWAY_CHAIN,
                &[
                    "-i",
                    lan_interface,
                    "-o",
                    vpn_interface,
                    "-s",
                    cidr,
                    "-j",
                    "ACCEPT",
                ],
            )?;
        }
        AllowedClients::List(ips) => {
            for ip in ips {
                add_rule(
                    GATEWAY_CHAIN,
                    &[
                        "-i",
                        lan_interface,
                        "-o",
                        vpn_interface,
                        "-s",
                        ip,
                        "-j",
                        "ACCEPT",
                    ],
                )?;
            }
        }
    }

    // Allow return traffic (established connections)
    add_rule(
        GATEWAY_CHAIN,
        &[
            "-i",
            vpn_interface,
            "-o",
            lan_interface,
            "-m",
            "state",
            "--state",
            "RELATED,ESTABLISHED",
            "-j",
            "ACCEPT",
        ],
    )?;

    // Insert jump to gateway chain at the beginning of FORWARD
    run_iptables(&["-I", "FORWARD", "1", "-j", GATEWAY_CHAIN])?;

    // IPv6: Simpler - just block all forwarding to prevent leaks
    let _ = run_ip6tables(&["-A", "FORWARD", "-j", "DROP"]);

    info!("Gateway forward rules enabled");
    Ok(())
}

/// Disable FORWARD rules.
pub fn disable_forward_rules() -> Result<(), GatewayError> {
    info!("Disabling gateway forward rules");

    // Remove jump rule
    let _ = run_iptables(&["-D", "FORWARD", "-j", GATEWAY_CHAIN]);

    // Flush and delete chain
    let _ = run_iptables(&["-F", GATEWAY_CHAIN]);
    let _ = run_iptables(&["-X", GATEWAY_CHAIN]);

    // IPv6
    let _ = run_ip6tables(&["-D", "FORWARD", "-j", "DROP"]);

    info!("Gateway forward rules disabled");
    Ok(())
}

/// Enable kill switch for forwarded traffic.
pub fn enable_forward_killswitch(lan_interface: &str) -> Result<(), GatewayError> {
    info!("Enabling gateway kill switch");

    // Create kill switch chain
    create_chain(GATEWAY_KS_CHAIN)?;

    // Block forwarded traffic going out the LAN interface
    add_rule(GATEWAY_KS_CHAIN, &["-o", lan_interface, "-j", "DROP"])?;

    // Insert at the END of FORWARD (after gateway rules)
    run_iptables(&["-A", "FORWARD", "-j", GATEWAY_KS_CHAIN])?;

    info!("Gateway kill switch enabled");
    Ok(())
}

/// Disable kill switch for forwarded traffic.
pub fn disable_forward_killswitch() -> Result<(), GatewayError> {
    info!("Disabling gateway kill switch");

    // Remove jump rule
    let _ = run_iptables(&["-D", "FORWARD", "-j", GATEWAY_KS_CHAIN]);

    // Flush and delete chain
    let _ = run_iptables(&["-F", GATEWAY_KS_CHAIN]);
    let _ = run_iptables(&["-X", GATEWAY_KS_CHAIN]);

    info!("Gateway kill switch disabled");
    Ok(())
}

/// Check if gateway kill switch is active.
pub fn is_forward_killswitch_active() -> bool {
    run_iptables(&["-L", GATEWAY_KS_CHAIN]).is_ok()
}

/// Get current FORWARD chain rules for status display.
pub fn get_forward_rules() -> Vec<String> {
    let output = Command::new("sudo")
        .arg(iptables())
        .args(["-L", "FORWARD", "-n", "-v", "--line-numbers"])
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.to_string())
            .collect(),
        _ => vec![],
    }
}

fn create_chain(chain: &str) -> Result<(), GatewayError> {
    let result = run_iptables(&["-N", chain]);
    if result.is_err() {
        // Chain exists, flush it
        run_iptables(&["-F", chain])?;
    }
    Ok(())
}

fn add_rule(chain: &str, args: &[&str]) -> Result<(), GatewayError> {
    let mut full_args = vec!["-A", chain];
    full_args.extend_from_slice(args);
    run_iptables(&full_args)
}

fn run_iptables(args: &[&str]) -> Result<(), GatewayError> {
    let output = Command::new("sudo")
        .arg(iptables())
        .args(args)
        .output()
        .map_err(|e| GatewayError::Firewall(format!("Failed to run iptables: {}", e)))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GatewayError::Firewall(format!(
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
        .map_err(|e| GatewayError::Firewall(format!("Failed to run ip6tables: {}", e)))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GatewayError::Firewall(format!(
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
    fn test_chain_names_are_unique() {
        assert_ne!(GATEWAY_CHAIN, GATEWAY_KS_CHAIN);
        assert_ne!(GATEWAY_CHAIN, "SHROUD_KILLSWITCH");
        assert_ne!(GATEWAY_CHAIN, "SHROUD_BOOT_KS");
    }
}
