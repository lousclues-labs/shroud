// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Common utilities for security tests

#![allow(dead_code)]

use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Run a shroud CLI command
pub fn shroud(args: &[&str]) -> std::process::Output {
    Command::new("./target/debug/shroud")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run shroud")
}

/// Get daemon PID
pub fn get_daemon_pid() -> Option<u32> {
    let output = Command::new("pgrep")
        .args(["-f", "target/debug/shroud"])
        .output()
        .ok()?;

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .lines()
        .next()?
        .parse()
        .ok()
}

/// Check if daemon is running
pub fn daemon_running() -> bool {
    shroud(&["ping"]).status.success()
}

/// Start daemon in background
pub fn start_daemon() -> Option<std::process::Child> {
    Command::new("./target/debug/shroud")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// Wait for daemon to start
pub fn wait_for_daemon(timeout_secs: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(timeout_secs) {
        if daemon_running() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Wait for daemon to stop
pub fn wait_for_daemon_stop(timeout_secs: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(timeout_secs) {
        if !daemon_running() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Check if iptables has kill switch rules
pub fn killswitch_rules_exist() -> bool {
    let output = Command::new("sudo")
        .args(["iptables", "-L", "OUTPUT", "-n"])
        .output()
        .expect("Failed to check iptables");

    let rules = String::from_utf8_lossy(&output.stdout);
    rules.contains("DROP") || rules.contains("REJECT") || rules.contains("shroud")
}

/// Clean up iptables rules manually
pub fn cleanup_iptables() {
    let _ = Command::new("sudo")
        .args(["iptables", "-F", "OUTPUT"])
        .output();
    let _ = Command::new("sudo")
        .args(["iptables", "-P", "OUTPUT", "ACCEPT"])
        .output();
}

/// Get socket path
pub fn socket_path() -> std::path::PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        std::path::PathBuf::from(runtime_dir).join("shroud.sock")
    } else {
        std::path::PathBuf::from("/tmp").join(format!("shroud-{}.sock", unsafe { libc::getuid() }))
    }
}
