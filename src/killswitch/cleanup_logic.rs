// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Kill-switch cleanup helpers — pure functions, easily testable.
//!
//! Builds cleanup command lists and parses iptables output without
//! actually running any commands.
//!
//! Some functions (`chain_exists_in_output`, `find_shroud_rules`,
//! `build_chain_cleanup`) are used by tests and available for future
//! use by `verify.rs`. Suppress dead-code warnings on the module.

#![allow(dead_code)]

/// Chain names managed by Shroud.
pub const SHROUD_CHAINS: &[&str] = &["SHROUD_KILLSWITCH", "SHROUD_BOOT_KS"];

/// Build the iptables arguments to remove a jump rule from a parent chain.
pub fn build_remove_jump(parent: &str, target: &str) -> Vec<String> {
    vec!["-D".into(), parent.into(), "-j".into(), target.into()]
}

/// Build the iptables arguments to flush a chain.
pub fn build_flush_chain(chain: &str) -> Vec<String> {
    vec!["-F".into(), chain.into()]
}

/// Build the iptables arguments to delete (remove) a chain.
pub fn build_delete_chain(chain: &str) -> Vec<String> {
    vec!["-X".into(), chain.into()]
}

/// Build the full ordered cleanup sequence for one chain.
pub fn build_chain_cleanup(chain: &str) -> Vec<Vec<String>> {
    vec![
        build_remove_jump("OUTPUT", chain),
        build_remove_jump("FORWARD", chain),
        build_flush_chain(chain),
        build_delete_chain(chain),
    ]
}

/// Check whether a chain name appears in `iptables -L` output.
pub fn chain_exists_in_output(output: &str, chain: &str) -> bool {
    output
        .lines()
        .any(|line| line.starts_with("Chain ") && line.contains(chain))
}

/// Find all lines in `iptables -S` output that reference any Shroud chain.
pub fn find_shroud_rules(output: &str) -> Vec<String> {
    output
        .lines()
        .filter(|line| SHROUD_CHAINS.iter().any(|c| line.contains(c)))
        .map(|s| s.to_string())
        .collect()
}

/// Ordered cleanup phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupPhase {
    RemoveJumps,
    FlushChains,
    DeleteChains,
}

/// Return the phases in the correct order.
pub fn cleanup_order() -> Vec<CleanupPhase> {
    vec![
        CleanupPhase::RemoveJumps,
        CleanupPhase::FlushChains,
        CleanupPhase::DeleteChains,
    ]
}

/// Build manual-cleanup instruction text for the user.
pub fn manual_cleanup_instructions(iptables_bin: &str, ip6tables_bin: &str) -> String {
    let mut lines = Vec::new();
    lines.push("Manual cleanup commands:".to_string());
    for chain in SHROUD_CHAINS {
        lines.push(format!("  sudo {} -D OUTPUT -j {}", iptables_bin, chain));
        lines.push(format!("  sudo {} -F {}", iptables_bin, chain));
        lines.push(format!("  sudo {} -X {}", iptables_bin, chain));
        lines.push(format!("  sudo {} -D OUTPUT -j {}", ip6tables_bin, chain));
        lines.push(format!("  sudo {} -F {}", ip6tables_bin, chain));
        lines.push(format!("  sudo {} -X {}", ip6tables_bin, chain));
    }
    lines.join("\n")
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shroud_chains_non_empty() {
        assert!(!SHROUD_CHAINS.is_empty());
        assert!(SHROUD_CHAINS.contains(&"SHROUD_KILLSWITCH"));
        assert!(SHROUD_CHAINS.contains(&"SHROUD_BOOT_KS"));
    }

    #[test]
    fn test_build_remove_jump() {
        let args = build_remove_jump("OUTPUT", "SHROUD_KILLSWITCH");
        assert_eq!(args, vec!["-D", "OUTPUT", "-j", "SHROUD_KILLSWITCH"]);
    }

    #[test]
    fn test_build_flush_chain() {
        let args = build_flush_chain("MY_CHAIN");
        assert_eq!(args, vec!["-F", "MY_CHAIN"]);
    }

    #[test]
    fn test_build_delete_chain() {
        let args = build_delete_chain("MY_CHAIN");
        assert_eq!(args, vec!["-X", "MY_CHAIN"]);
    }

    #[test]
    fn test_build_chain_cleanup_order() {
        let cmds = build_chain_cleanup("TEST");
        assert_eq!(cmds.len(), 4);
        // First: remove jumps from OUTPUT and FORWARD
        assert!(cmds[0].contains(&"-D".to_string()));
        assert!(cmds[0].contains(&"OUTPUT".to_string()));
        assert!(cmds[1].contains(&"FORWARD".to_string()));
        // Then flush, then delete
        assert!(cmds[2].contains(&"-F".to_string()));
        assert!(cmds[3].contains(&"-X".to_string()));
    }

    #[test]
    fn test_chain_exists_in_output_found() {
        let output = "\
Chain INPUT (policy ACCEPT)
Chain FORWARD (policy ACCEPT)
Chain OUTPUT (policy ACCEPT)
Chain SHROUD_KILLSWITCH (1 references)
";
        assert!(chain_exists_in_output(output, "SHROUD_KILLSWITCH"));
    }

    #[test]
    fn test_chain_exists_in_output_not_found() {
        let output = "Chain INPUT (policy ACCEPT)\nChain OUTPUT (policy ACCEPT)\n";
        assert!(!chain_exists_in_output(output, "SHROUD_KILLSWITCH"));
    }

    #[test]
    fn test_chain_exists_partial_no_match() {
        let output = "Chain SHROUD_KILLSWITCH (1 references)\n";
        assert!(!chain_exists_in_output(output, "SHROUD_BOOT"));
    }

    #[test]
    fn test_find_shroud_rules() {
        let output = "\
-A OUTPUT -j ACCEPT
-A OUTPUT -j SHROUD_KILLSWITCH
-A INPUT -j ACCEPT
";
        let rules = find_shroud_rules(output);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].contains("SHROUD_KILLSWITCH"));
    }

    #[test]
    fn test_find_shroud_rules_empty() {
        let rules = find_shroud_rules("-A OUTPUT -j ACCEPT\n");
        assert!(rules.is_empty());
    }

    #[test]
    fn test_cleanup_order() {
        let order = cleanup_order();
        assert_eq!(order[0], CleanupPhase::RemoveJumps);
        assert_eq!(order[1], CleanupPhase::FlushChains);
        assert_eq!(order[2], CleanupPhase::DeleteChains);
    }

    #[test]
    fn test_manual_cleanup_instructions() {
        let text = manual_cleanup_instructions("iptables", "ip6tables");
        assert!(text.contains("iptables -D OUTPUT -j SHROUD_KILLSWITCH"));
        assert!(text.contains("ip6tables -D OUTPUT -j SHROUD_KILLSWITCH"));
        assert!(text.contains("SHROUD_BOOT_KS"));
    }

    #[test]
    fn test_manual_cleanup_custom_binaries() {
        let text = manual_cleanup_instructions("/usr/sbin/iptables", "/usr/sbin/ip6tables");
        assert!(text.contains("/usr/sbin/iptables"));
        assert!(text.contains("/usr/sbin/ip6tables"));
    }
}
