// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Integration tests for input validation
//!
//! Verifies that invalid inputs are rejected end-to-end.
//!
//! Run with: cargo test --test validation_integration -- --nocapture

use std::process::{Command, Stdio};

fn shroud(args: &[&str]) -> std::process::Output {
    Command::new("./target/debug/shroud")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run shroud")
}

#[test]
fn test_invalid_timeout_rejected() {
    let output = shroud(&["--timeout", "0", "status"]);
    assert!(!output.status.success(), "Zero timeout should fail");

    let output = shroud(&["--timeout", "-1", "status"]);
    assert!(!output.status.success(), "Negative timeout should fail");

    let output = shroud(&["--timeout", "99999999", "status"]);
    assert!(!output.status.success(), "Huge timeout should fail");

    let output = shroud(&["--timeout", "abc", "status"]);
    assert!(!output.status.success(), "Non-numeric timeout should fail");
}

#[test]
fn test_valid_timeout_accepted() {
    let output = shroud(&["--timeout", "5", "--help"]);
    assert!(output.status.success(), "Valid timeout should succeed");

    let output = shroud(&["--timeout", "3600", "--help"]);
    assert!(output.status.success(), "Max timeout should succeed");
}

#[test]
fn test_invalid_log_level_rejected() {
    let output = shroud(&["--log-level", "invalid", "--help"]);
    assert!(!output.status.success(), "Invalid log level should fail");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("must be one of"),
        "Should show valid options"
    );
}

#[test]
fn test_valid_log_level_accepted() {
    for level in &["error", "warn", "info", "debug", "trace", "DEBUG", "Info"] {
        let output = shroud(&["--log-level", level, "--help"]);
        assert!(
            output.status.success(),
            "Log level '{}' should be accepted",
            level
        );
    }
}

#[test]
fn test_empty_vpn_name_rejected() {
    let output = shroud(&["connect", ""]);
    assert!(!output.status.success(), "Empty VPN name should fail");
}

#[test]
fn test_error_messages_helpful() {
    let output = shroud(&["--timeout", "0", "status"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("at least") || stderr.contains("1"),
        "Should mention minimum value"
    );

    let output = shroud(&["--log-level", "bad", "status"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error") && stderr.contains("warn"),
        "Should list valid options"
    );
}
