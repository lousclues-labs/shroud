// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Firewall binary path detection.
//!
//! Detects iptables/ip6tables/nft binaries across distros and caches results.

use std::path::PathBuf;
use std::sync::OnceLock;
use tracing::debug;

static IPTABLES_PATH: OnceLock<PathBuf> = OnceLock::new();
static IP6TABLES_PATH: OnceLock<PathBuf> = OnceLock::new();
static NFT_PATH: OnceLock<PathBuf> = OnceLock::new();

const IPTABLES_CANDIDATES: &[&str] = &[
    "/usr/bin/iptables",
    "/usr/sbin/iptables",
    "/bin/iptables",
    "/sbin/iptables",
];

const IP6TABLES_CANDIDATES: &[&str] = &[
    "/usr/bin/ip6tables",
    "/usr/sbin/ip6tables",
    "/bin/ip6tables",
    "/sbin/ip6tables",
];

const NFT_CANDIDATES: &[&str] = &["/usr/bin/nft", "/usr/sbin/nft", "/bin/nft", "/sbin/nft"];

fn find_binary(candidates: &[&str], name: &str) -> PathBuf {
    for candidate in candidates {
        let path = PathBuf::from(candidate);
        if path.exists() {
            debug!("Found {} at {}", name, candidate);
            return path;
        }
    }

    if let Ok(output) = std::process::Command::new("which").arg(name).output() {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout);
            let path = PathBuf::from(path_str.trim());
            if path.exists() {
                debug!("Found {} via which: {}", name, path.display());
                return path;
            }
        }
    }

    debug!("Could not find {}, defaulting to /usr/sbin/{}", name, name);
    PathBuf::from(format!("/usr/sbin/{}", name))
}

pub fn iptables_path() -> &'static PathBuf {
    IPTABLES_PATH.get_or_init(|| find_binary(IPTABLES_CANDIDATES, "iptables"))
}

pub fn ip6tables_path() -> &'static PathBuf {
    IP6TABLES_PATH.get_or_init(|| find_binary(IP6TABLES_CANDIDATES, "ip6tables"))
}

pub fn nft_path() -> &'static PathBuf {
    NFT_PATH.get_or_init(|| find_binary(NFT_CANDIDATES, "nft"))
}

pub fn iptables() -> &'static str {
    iptables_path().to_str().unwrap_or("/usr/sbin/iptables")
}

pub fn ip6tables() -> &'static str {
    ip6tables_path().to_str().unwrap_or("/usr/sbin/ip6tables")
}

pub fn nft() -> &'static str {
    nft_path().to_str().unwrap_or("/usr/sbin/nft")
}

pub fn log_detected_paths() {
    tracing::debug!("Firewall binary paths:");
    tracing::debug!("  iptables:  {}", iptables());
    tracing::debug!("  ip6tables: {}", ip6tables());
    tracing::debug!("  nft:       {}", nft());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_strings_are_absolute() {
        assert!(iptables().starts_with('/'));
        assert!(ip6tables().starts_with('/'));
        assert!(nft().starts_with('/'));
    }

    #[test]
    fn test_paths_are_absolute() {
        assert!(iptables_path().is_absolute());
        assert!(ip6tables_path().is_absolute());
        assert!(nft_path().is_absolute());
    }

    #[test]
    fn test_iptables_path_contains_iptables() {
        let path = iptables();
        assert!(
            path.contains("iptables"),
            "iptables path should contain 'iptables': {}",
            path
        );
    }

    #[test]
    fn test_ip6tables_path_contains_ip6tables() {
        let path = ip6tables();
        assert!(
            path.contains("ip6tables"),
            "ip6tables path should contain 'ip6tables': {}",
            path
        );
    }

    #[test]
    fn test_nft_path_contains_nft() {
        let path = nft();
        assert!(
            path.contains("nft"),
            "nft path should contain 'nft': {}",
            path
        );
    }

    #[test]
    fn test_paths_not_empty() {
        assert!(!iptables().is_empty());
        assert!(!ip6tables().is_empty());
        assert!(!nft().is_empty());
    }

    #[test]
    fn test_log_detected_paths_does_not_panic() {
        // Just verify it doesn't panic
        log_detected_paths();
    }
}
