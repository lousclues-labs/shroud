//! Kill switch cleanup functionality
//!
//! Principle III: Leave No Trace
//! Cleanup is non-negotiable.
//!
//! Principle II: Fail Loud, Recover Quiet
//! Cleanup should be silent on success, loud on failure.

use log::{debug, error, info, warn};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;

use crate::killswitch::paths::{ip6tables, iptables, nft};

/// Default timeout for cleanup operations
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors that can occur during cleanup
#[derive(Error, Debug)]
pub enum CleanupError {
    #[error("Cleanup timed out after {0:?} - password prompt may be blocking")]
    Timeout(Duration),

    #[error("Cleanup command failed: {0}")]
    CommandFailed(String),
}

/// Result of a cleanup attempt
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupResult {
    /// Rules were cleaned up successfully
    Cleaned,
    /// No rules existed, nothing to clean
    NothingToClean,
    /// Cleanup failed but we logged instructions
    Failed(String),
}

/// Check if SHROUD_KILLSWITCH chain exists in iptables
pub fn rules_exist() -> Result<bool, CleanupError> {
    // Use sudo -n to check without password prompt
    let output = Command::new("sudo")
        .args(["-n", iptables(), "-L", "SHROUD_KILLSWITCH", "-n"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match output {
        Ok(status) => Ok(status.success()),
        Err(e) => {
            debug!("Could not check iptables rules: {}", e);
            Ok(false)
        }
    }
}

/// Check if IPv6 SHROUD_KILLSWITCH chain exists
pub fn rules_exist_ipv6() -> Result<bool, CleanupError> {
    // Use sudo -n to check without password prompt
    let output = Command::new("sudo")
        .args(["-n", ip6tables(), "-L", "SHROUD_KILLSWITCH", "-n"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match output {
        Ok(status) => Ok(status.success()),
        Err(e) => {
            debug!("Could not check ip6tables rules: {}", e);
            Ok(false)
        }
    }
}

fn run_cleanup_command() -> Result<(), CleanupError> {
    // Use sudo -n to avoid password prompts that would cause hangs
    let commands: Vec<Vec<&str>> = vec![
        vec!["-n", iptables(), "-D", "OUTPUT", "-j", "SHROUD_KILLSWITCH"],
        vec!["-n", iptables(), "-F", "SHROUD_KILLSWITCH"],
        vec!["-n", iptables(), "-X", "SHROUD_KILLSWITCH"],
        vec!["-n", ip6tables(), "-D", "OUTPUT", "-j", "SHROUD_KILLSWITCH"],
        vec!["-n", ip6tables(), "-F", "SHROUD_KILLSWITCH"],
        vec!["-n", ip6tables(), "-X", "SHROUD_KILLSWITCH"],
    ];

    for command in commands {
        let _ = Command::new("sudo")
            .args(&command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    let _ = Command::new("sudo")
        .args(["-n", nft(), "delete", "table", "inet", "shroud_killswitch"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    Ok(())
}

/// Execute cleanup with a timeout.
///
/// This prevents blocking forever if a password prompt appears.
pub fn cleanup_with_timeout(timeout: Duration) -> Result<CleanupResult, CleanupError> {
    let ipv4_exists = rules_exist().unwrap_or(false);
    let ipv6_exists = rules_exist_ipv6().unwrap_or(false);

    if !ipv4_exists && !ipv6_exists {
        debug!("No kill switch rules to clean up");
        return Ok(CleanupResult::NothingToClean);
    }

    info!("Cleaning up kill switch rules...");

    let start = Instant::now();

    match run_cleanup_command() {
        Ok(()) => {
            if start.elapsed() > timeout {
                warn!("Cleanup timed out after {:?}", timeout);
                return Err(CleanupError::Timeout(timeout));
            }
            if rules_exist().unwrap_or(false) || rules_exist_ipv6().unwrap_or(false) {
                return Err(CleanupError::CommandFailed(
                    "Kill switch rules still present after cleanup".to_string(),
                ));
            }
            info!("Kill switch rules cleaned up successfully");
            Ok(CleanupResult::Cleaned)
        }
        Err(err) => {
            if start.elapsed() > timeout {
                warn!("Cleanup timed out after {:?}", timeout);
                return Err(CleanupError::Timeout(timeout));
            }
            Err(err)
        }
    }
}

/// Perform cleanup, logging clear instructions on failure.
pub fn cleanup_with_fallback() -> CleanupResult {
    match cleanup_with_timeout(CLEANUP_TIMEOUT) {
        Ok(result) => result,
        Err(e) => {
            error!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            error!("KILL SWITCH CLEANUP FAILED");
            error!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            error!("");
            error!("Error: {}", e);
            error!("");
            error!("Your firewall rules may still be blocking network traffic.");
            error!("To manually clean up, run:");
            error!("");
            error!("  sudo {} -D OUTPUT -j SHROUD_KILLSWITCH", iptables());
            error!("  sudo {} -F SHROUD_KILLSWITCH", iptables());
            error!("  sudo {} -X SHROUD_KILLSWITCH", iptables());
            error!("  sudo {} -D OUTPUT -j SHROUD_KILLSWITCH", ip6tables());
            error!("  sudo {} -F SHROUD_KILLSWITCH", ip6tables());
            error!("  sudo {} -X SHROUD_KILLSWITCH", ip6tables());
            error!("");
            error!("To avoid this in the future, install the sudoers rule:");
            error!("  ./setup.sh --install-sudoers");
            error!("");
            error!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

            CleanupResult::Failed(e.to_string())
        }
    }
}

/// Clean up stale rules from a previous crash.
pub fn cleanup_stale_on_startup() {
    let ipv4_exists = rules_exist().unwrap_or(false);
    let ipv6_exists = rules_exist_ipv6().unwrap_or(false);

    if !ipv4_exists && !ipv6_exists {
        return;
    }

    warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    warn!("STALE KILL SWITCH RULES DETECTED");
    warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    warn!("");
    warn!("Found firewall rules from a previous Shroud instance.");
    warn!("Attempting cleanup...");
    warn!("");

    match cleanup_with_timeout(CLEANUP_TIMEOUT) {
        Ok(CleanupResult::Cleaned) => {
            info!("Stale rules cleaned up successfully");
        }
        Ok(CleanupResult::NothingToClean) => {
            debug!("No stale rules to clean");
        }
        Ok(CleanupResult::Failed(msg)) | Err(CleanupError::CommandFailed(msg)) => {
            error!("Failed to clean stale rules: {}", msg);
            log_manual_cleanup_instructions();
        }
        Err(CleanupError::Timeout(_)) => {
            warn!("Cleanup timed out. You may need to enter your password.");
            log_manual_cleanup_instructions();
        }
    }
}

fn log_manual_cleanup_instructions() {
    error!("");
    error!("Manual cleanup commands:");
    error!("  sudo {} -D OUTPUT -j SHROUD_KILLSWITCH", iptables());
    error!("  sudo {} -F SHROUD_KILLSWITCH", iptables());
    error!("  sudo {} -X SHROUD_KILLSWITCH", iptables());
    error!("  sudo ip6tables -D OUTPUT -j SHROUD_KILLSWITCH");
    error!("  sudo ip6tables -F SHROUD_KILLSWITCH");
    error!("  sudo ip6tables -X SHROUD_KILLSWITCH");
    error!("");
}

/// Clean up all kill switch rules (iptables, ip6tables, nft, boot chain).
///
/// Used during shutdown to ensure no rules are left behind.
pub fn cleanup_all() -> Result<(), CleanupError> {
    // Clean main kill switch
    let _ = cleanup_with_timeout(CLEANUP_TIMEOUT);

    // Clean boot kill switch chain
    // Use sudo -n to avoid password prompts that would cause hangs
    let boot_commands: Vec<Vec<&str>> = vec![
        vec!["-n", iptables(), "-D", "OUTPUT", "-j", "SHROUD_BOOT_KS"],
        vec!["-n", iptables(), "-F", "SHROUD_BOOT_KS"],
        vec!["-n", iptables(), "-X", "SHROUD_BOOT_KS"],
        vec!["-n", ip6tables(), "-D", "OUTPUT", "-j", "SHROUD_BOOT_KS"],
        vec!["-n", ip6tables(), "-F", "SHROUD_BOOT_KS"],
        vec!["-n", ip6tables(), "-X", "SHROUD_BOOT_KS"],
    ];

    for command in boot_commands {
        let _ = Command::new("sudo")
            .args(&command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cleanup_result_variants() {
        let cleaned = CleanupResult::Cleaned;
        let nothing = CleanupResult::NothingToClean;
        let failed = CleanupResult::Failed("test".to_string());

        assert_eq!(cleaned, CleanupResult::Cleaned);
        assert_eq!(nothing, CleanupResult::NothingToClean);
        assert_ne!(cleaned, nothing);

        if let CleanupResult::Failed(msg) = failed {
            assert_eq!(msg, "test");
        } else {
            panic!("Expected Failed variant");
        }
    }
}
