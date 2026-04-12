// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Test assertions for VPNShroud

use std::process::Command;

/// Assert iptables chain exists
pub fn assert_chain_exists(chain: &str) {
    let output = Command::new("sudo")
        .args(["iptables", "-L", chain, "-n"])
        .output()
        .expect("Failed to run iptables");

    assert!(
        output.status.success(),
        "iptables chain '{}' does not exist",
        chain
    );
}

/// Assert iptables chain does NOT exist
pub fn assert_chain_not_exists(chain: &str) {
    let output = Command::new("sudo")
        .args(["iptables", "-L", chain, "-n"])
        .output()
        .expect("Failed to run iptables");

    assert!(
        !output.status.success(),
        "iptables chain '{}' should not exist",
        chain
    );
}

/// Assert iptables chain contains a rule matching pattern
pub fn assert_chain_contains(chain: &str, pattern: &str) {
    let output = Command::new("sudo")
        .args(["iptables", "-L", chain, "-n"])
        .output()
        .expect("Failed to run iptables");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(pattern),
        "Chain '{}' does not contain '{}'.\nActual:\n{}",
        chain,
        pattern,
        stdout
    );
}

/// Assert a network interface exists
pub fn assert_interface_exists(iface: &str) {
    let output = Command::new("ip")
        .args(["link", "show", iface])
        .output()
        .expect("Failed to run ip");

    assert!(
        output.status.success(),
        "Interface '{}' does not exist",
        iface
    );
}

/// Assert a network interface does NOT exist
pub fn assert_interface_not_exists(iface: &str) {
    let output = Command::new("ip")
        .args(["link", "show", iface])
        .output()
        .expect("Failed to run ip");

    assert!(
        !output.status.success(),
        "Interface '{}' should not exist",
        iface
    );
}

/// Assert IP forwarding is enabled
pub fn assert_forwarding_enabled() {
    let content = std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
        .expect("Failed to read ip_forward");
    assert_eq!(content.trim(), "1", "IP forwarding is not enabled");
}

/// Assert string contains pattern
pub fn assert_contains(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "Expected to contain '{}', got:\n{}",
        needle,
        haystack
    );
}

/// Assert string does NOT contain pattern
pub fn assert_not_contains(haystack: &str, needle: &str) {
    assert!(
        !haystack.contains(needle),
        "Should not contain '{}', got:\n{}",
        needle,
        haystack
    );
}

/// Assert command exits successfully
pub fn assert_command_succeeds(cmd: &str, args: &[&str]) {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .expect("Failed to run command");

    assert!(
        output.status.success(),
        "Command '{} {}' failed: {}",
        cmd,
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Assert command fails
pub fn assert_command_fails(cmd: &str, args: &[&str]) {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .expect("Failed to run command");

    assert!(
        !output.status.success(),
        "Command '{} {}' should have failed",
        cmd,
        args.join(" ")
    );
}

/// Assert file exists
pub fn assert_file_exists(path: &std::path::Path) {
    assert!(path.exists(), "File does not exist: {:?}", path);
}

/// Assert file does NOT exist
pub fn assert_file_not_exists(path: &std::path::Path) {
    assert!(!path.exists(), "File should not exist: {:?}", path);
}

/// Assert file permissions
#[cfg(unix)]
pub fn assert_file_permissions(path: &std::path::Path, expected_mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path).expect("Failed to get file metadata");
    let mode = metadata.permissions().mode() & 0o777;
    assert_eq!(
        mode, expected_mode,
        "File {:?} has mode {:o}, expected {:o}",
        path, mode, expected_mode
    );
}
