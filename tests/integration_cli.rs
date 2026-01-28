// tests/integration_cli.rs
//! CLI integration tests
//!
//! These tests verify CLI commands work correctly.
//! They require the daemon to NOT be running.

#[test]
#[ignore] // Run with: cargo test -- --ignored
fn test_cli_help() {
    // Test --help works
    let status = std::process::Command::new("cargo")
        .args(&["run", "--", "--help"])
        .status()
        .expect("failed to execute process");

    assert!(status.success());
}

#[test]
#[ignore]
fn test_cli_version() {
    // Test --version works
    let status = std::process::Command::new("cargo")
        .args(&["run", "--", "--version"])
        .status()
        .expect("failed to execute process");

    assert!(status.success());
}
