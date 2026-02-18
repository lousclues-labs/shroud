// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! System-level test utilities

use std::process::Command;

/// Clean up all shroud-related iptables chains
pub fn cleanup_iptables() {
    let chains = [
        "SHROUD_KILLSWITCH",
        "SHROUD_BOOT_KS",
    ];

    for chain in &chains {
        // IPv4
        let _ = Command::new("sudo")
            .args(["iptables", "-D", "OUTPUT", "-j", chain])
            .output();
        let _ = Command::new("sudo")
            .args(["iptables", "-D", "FORWARD", "-j", chain])
            .output();
        let _ = Command::new("sudo")
            .args(["iptables", "-F", chain])
            .output();
        let _ = Command::new("sudo")
            .args(["iptables", "-X", chain])
            .output();

        // IPv6
        let _ = Command::new("sudo")
            .args(["ip6tables", "-D", "OUTPUT", "-j", chain])
            .output();
        let _ = Command::new("sudo")
            .args(["ip6tables", "-F", chain])
            .output();
        let _ = Command::new("sudo")
            .args(["ip6tables", "-X", chain])
            .output();
    }
}

/// Kill any running shroud processes
pub fn cleanup_shroud_processes() {
    let _ = Command::new("pkill").args(["-f", "shroud"]).output();
    std::thread::sleep(std::time::Duration::from_millis(500));
}

/// Clean up test socket
pub fn cleanup_socket() {
    let socket = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let socket_path = format!("{}/shroud.sock", socket);
    let _ = std::fs::remove_file(socket_path);
}

/// Full cleanup
pub fn full_cleanup() {
    cleanup_shroud_processes();
    cleanup_iptables();
    cleanup_socket();
}

/// Check if running as root
pub fn is_root() -> bool {
    nix::unistd::geteuid().is_root()
}

/// Skip test if not root
pub fn require_root() {
    if !is_root() {
        eprintln!("SKIPPED: requires root");
        std::process::exit(77);
    }
}

/// Check if NetworkManager is running
pub fn nm_is_running() -> bool {
    Command::new("systemctl")
        .args(["is-active", "NetworkManager"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Skip test if NM not running
pub fn require_nm() {
    if !nm_is_running() {
        eprintln!("SKIPPED: requires NetworkManager");
        std::process::exit(77);
    }
}

/// Check if running in CI environment
pub fn is_ci() -> bool {
    std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok()
}

/// Get the test timeout (longer in CI)
pub fn test_timeout() -> std::time::Duration {
    if is_ci() {
        std::time::Duration::from_secs(60)
    } else {
        std::time::Duration::from_secs(30)
    }
}
