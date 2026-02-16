// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! Kill switch cleanup functionality
//!
//! Principle III: Leave No Trace
//! Cleanup is non-negotiable.
//!
//! Principle II: Fail Loud, Recover Quiet
//! Cleanup should be silent on success, loud on failure.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, error, info, warn};

use crate::killswitch::cleanup_logic::{self, SHROUD_CHAINS};
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
    let mut failures: Vec<String> = Vec::new();
    let bins: &[&str] = &[iptables(), ip6tables()];

    // Phase 1: Remove ALL duplicate jump rules for every Shroud chain
    // (race conditions can create many)
    for chain in SHROUD_CHAINS {
        let jump_args = cleanup_logic::build_remove_jump("OUTPUT", chain);
        for bin in bins {
            for _ in 0..100 {
                // Safety limit
                let mut full_args = vec!["-n".to_string(), bin.to_string()];
                full_args.extend(jump_args.clone());
                let status = Command::new("sudo")
                    .args(&full_args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                if !matches!(status, Ok(s) if s.success()) {
                    break;
                }
            }
        }
    }

    // Phase 2 & 3: Flush then delete each chain
    for chain in SHROUD_CHAINS {
        let flush_args = cleanup_logic::build_flush_chain(chain);
        let delete_args = cleanup_logic::build_delete_chain(chain);

        for args in [&flush_args, &delete_args] {
            for bin in bins {
                let mut full_args = vec!["-n".to_string(), bin.to_string()];
                full_args.extend(args.clone());
                match Command::new("sudo")
                    .args(&full_args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                {
                    Ok(s) if !s.success() => {
                        // -X/-F fails when chain doesn't exist — idempotent cleanup.
                        debug!("cleanup command failed: sudo {}", full_args.join(" "));
                    }
                    Err(e) => {
                        failures.push(format!("sudo {} {}: {}", bin, args.join(" "), e));
                    }
                    _ => {}
                }
            }
        }
    }

    // Also try nftables cleanup
    if let Err(e) = Command::new("sudo")
        .args(["-n", nft(), "delete", "table", "inet", "shroud_killswitch"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        failures.push(format!("nft delete table: {}", e));
    }

    if !failures.is_empty() {
        return Err(CleanupError::CommandFailed(failures.join("; ")));
    }

    Ok(())
}

/// Execute cleanup and verify completion within a time budget.
///
/// Runs `run_cleanup_command()` synchronously (blocking on `sudo -n`), then
/// checks whether the elapsed time exceeded `timeout`. The timeout is **not**
/// enforced as a deadline — it is a post-hoc duration check. `sudo -n` prevents
/// interactive password prompts; this function detects cases where the commands
/// took unexpectedly long (e.g., kernel module load, slow nft flush).
///
/// **NOTE:** Uses synchronous commands — safe for CLI and startup use but must
/// not be called from the daemon event loop. Use `KillSwitch::disable()` instead.
///
/// # Errors
///
/// Returns [`CleanupError::Timeout`] if elapsed time exceeds `timeout` after commands complete.
///
/// Returns [`CleanupError::CommandFailed`] if rules remain after cleanup commands complete.
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
            for line in cleanup_logic::manual_cleanup_instructions(iptables(), ip6tables()).lines()
            {
                error!("{}", line);
            }
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
    let boot_exists = boot_chain_exists().unwrap_or(false);

    if !ipv4_exists && !ipv6_exists && !boot_exists {
        return;
    }

    warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    warn!("STALE KILL SWITCH RULES DETECTED");
    warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    warn!("");
    warn!("Found firewall rules from a previous Shroud instance.");
    warn!("Attempting cleanup...");
    warn!("");

    // Use cleanup_all which handles both kill switch and boot chain
    match cleanup_all() {
        Ok(()) => {
            info!("Stale rules cleaned up successfully");
        }
        Err(e) => {
            error!("Failed to clean stale rules: {}", e);
            log_manual_cleanup_instructions();
        }
    }
}

/// Check if boot kill switch chain exists
fn boot_chain_exists() -> Result<bool, CleanupError> {
    let output = Command::new("sudo")
        .args(["-n", iptables(), "-L", "SHROUD_BOOT_KS", "-n"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match output {
        Ok(status) => Ok(status.success()),
        Err(e) => {
            debug!("Could not check boot chain: {}", e);
            Ok(false)
        }
    }
}

fn log_manual_cleanup_instructions() {
    for line in cleanup_logic::manual_cleanup_instructions(iptables(), ip6tables()).lines() {
        error!("{}", line);
    }
}

/// Clean up all kill switch rules (iptables, ip6tables, nft, boot chain).
///
/// Uses `cleanup_logic::SHROUD_CHAINS` as the single source of truth for
/// chain names — ensures both `SHROUD_KILLSWITCH` and `SHROUD_BOOT_KS`
/// are cleaned. Principle III: Leave No Trace.
///
/// # Errors
///
/// Returns [`CleanupError::CommandFailed`] if any cleanup step fails and
/// firewall rules remain after the attempt.
pub fn cleanup_all() -> Result<(), CleanupError> {
    let mut errors: Vec<String> = Vec::new();

    // run_cleanup_command() handles all chains in SHROUD_CHAINS
    // (SHROUD_KILLSWITCH + SHROUD_BOOT_KS) for both iptables and ip6tables
    if let Err(e) = run_cleanup_command() {
        errors.push(format!("cleanup commands: {}", e));
    }

    // Verify nothing is left behind
    let ipv4_remain = rules_exist().unwrap_or(false);
    let ipv6_remain = rules_exist_ipv6().unwrap_or(false);
    let boot_remain = boot_chain_exists().unwrap_or(false);

    if ipv4_remain || ipv6_remain || boot_remain {
        let remaining: Vec<&str> = [
            if ipv4_remain { Some("iptables") } else { None },
            if ipv6_remain { Some("ip6tables") } else { None },
            if boot_remain {
                Some("boot chain")
            } else {
                None
            },
        ]
        .into_iter()
        .flatten()
        .collect();
        let msg = format!(
            "Rules still present after cleanup: {}",
            remaining.join(", ")
        );
        error!("{}", msg);
        return Err(CleanupError::CommandFailed(msg));
    }

    if !errors.is_empty() {
        // Cleanup commands reported errors but rules are gone — log but succeed
        warn!(
            "Cleanup had transient errors (rules removed): {}",
            errors.join("; ")
        );
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
