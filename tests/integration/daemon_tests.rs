// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Integration tests for daemon lifecycle.
//!
//! These tests are ignored by default and require a suitable environment.

use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn shroud(args: &[&str]) -> std::process::Output {
    Command::new("./target/debug/shroud")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run shroud")
}

fn daemon_running() -> bool {
    let output = shroud(&["ping"]);
    output.status.success()
}

fn wait_for_daemon(timeout_secs: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(timeout_secs) {
        if daemon_running() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn wait_for_daemon_stop(timeout_secs: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(timeout_secs) {
        if !daemon_running() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
#[ignore]
fn test_daemon_lifecycle() {
    let _ = shroud(&["quit"]);
    wait_for_daemon_stop(5);

    let mut daemon = Command::new("./target/debug/shroud")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to start daemon");

    assert!(wait_for_daemon(10), "Daemon failed to start");

    let output = shroud(&["ping"]);
    assert!(output.status.success(), "Ping failed");

    let output = shroud(&["status"]);
    assert!(output.status.success(), "Status failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Status:"), "Status output missing");

    let output = shroud(&["list"]);
    assert!(output.status.success(), "List failed");

    let output = shroud(&["killswitch", "status"]);
    assert!(output.status.success(), "Killswitch status failed");

    let output = shroud(&["auto-reconnect", "status"]);
    assert!(output.status.success(), "Auto-reconnect status failed");

    let output = shroud(&["reload"]);
    assert!(output.status.success(), "Reload failed");

    let output = shroud(&["restart"]);
    assert!(output.status.success(), "Restart failed");

    thread::sleep(Duration::from_secs(2));
    assert!(wait_for_daemon(10), "Daemon failed to restart");

    let output = shroud(&["quit"]);
    assert!(output.status.success(), "Quit failed");

    assert!(wait_for_daemon_stop(5), "Daemon failed to stop");

    let _ = daemon.kill();
    let _ = daemon.wait();
}

#[test]
#[ignore]
fn test_autostart_commands() {
    let output = shroud(&["autostart", "status"]);
    assert!(output.status.success());

    let output = shroud(&["autostart", "on"]);
    assert!(output.status.success());

    let output = shroud(&["autostart", "status"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("enabled"));

    let output = shroud(&["autostart", "off"]);
    assert!(output.status.success());

    let output = shroud(&["autostart", "status"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("disabled"));
}

#[test]
#[ignore]
fn test_cleanup_command() {
    let output = shroud(&["cleanup"]);
    assert!(output.status.success());
}

#[test]
#[ignore]
fn test_version_check() {
    let output = shroud(&["version"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("shroud"));

    let _ = shroud(&["version", "--check"]);
}

#[test]
#[ignore]
fn test_help_commands() {
    let output = shroud(&["help"]);
    assert!(output.status.success());

    let output = shroud(&["help", "connect"]);
    assert!(output.status.success());

    let output = shroud(&["help", "killswitch"]);
    assert!(output.status.success());

    let output = shroud(&["help", "autostart"]);
    assert!(output.status.success());
}

#[test]
#[ignore]
fn test_json_output() {
    if !daemon_running() {
        let _ = Command::new("./target/debug/shroud")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        wait_for_daemon(10);
    }

    let output = shroud(&["status", "--json"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "Status --json should output valid JSON");
}

#[test]
#[ignore]
fn test_quiet_mode() {
    if !daemon_running() {
        let _ = Command::new("./target/debug/shroud")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        wait_for_daemon(10);
    }

    let output = shroud(&["-q", "status"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty() || stdout.trim().is_empty());
}
