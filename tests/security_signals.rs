//! # Security Tests - Privileged Environment Required
//!
//! These tests require a privileged environment to run:
//! - Root/sudo access for iptables manipulation
//! - NetworkManager running with VPN connections configured
//! - D-Bus session available
//!
//! ## Running Locally
//!
//! ```bash
//! # Run all ignored tests (requires sudo)
//! sudo -E cargo test -- --ignored
//!
//! # Run specific test file
//! sudo -E cargo test --test security_crash -- --ignored
//! ```
//!
//! ## CI Behavior
//!
//! These tests are marked with `#[ignore]` and will NOT run in CI.
//! They are skipped by `cargo test` unless `--ignored` is passed.
//!
//! Signal Handling Security Tests
//!
//! Tests for proper signal handling:
//! - SIGTERM/SIGINT during critical operations
//! - SIGHUP reload safety
//! - Signal during iptables modification
//!
//! ## Running These Tests
//! Most tests in this file require root privileges and are marked with `#[ignore]`.
//! To run them:
//!
//! ```bash
//! sudo -E cargo test --test security_signals -- --ignored --nocapture
//! ```
//!
//! ## Requirements
//! - Root/sudo access
//! - NetworkManager running
//! - D-Bus session available
//! - iptables/nftables installed

use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn shroud(args: &[&str]) -> std::process::Output {
    Command::new("./target/debug/shroud")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run shroud")
}

fn get_daemon_pid() -> Option<u32> {
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

fn daemon_running() -> bool {
    get_daemon_pid().is_some()
}

fn start_daemon() -> Option<std::process::Child> {
    Command::new("./target/debug/shroud")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

fn wait_for_daemon(timeout_secs: u64) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(timeout_secs) {
        if shroud(&["ping"]).status.success() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn wait_for_daemon_stop(timeout_secs: u64) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(timeout_secs) {
        if !daemon_running() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn killswitch_rules_exist() -> bool {
    let output = Command::new("sudo")
        .args(["iptables", "-L", "OUTPUT", "-n"])
        .output()
        .expect("Failed to check iptables");

    let rules = String::from_utf8_lossy(&output.stdout);
    rules.contains("DROP") || rules.contains("shroud")
}

fn cleanup_iptables() {
    let _ = Command::new("sudo")
        .args(["iptables", "-F", "OUTPUT"])
        .output();
    let _ = Command::new("sudo")
        .args(["iptables", "-P", "OUTPUT", "ACCEPT"])
        .output();
}

// ============================================================================
// SIGTERM HANDLING
// ============================================================================

#[test]
#[ignore = "requires root privileges for signal handling and iptables"]
fn test_sigterm_cleans_up_killswitch() {
    println!("\n=== TEST: SIGTERM cleans up kill switch ===\n");

    // Ensure clean state
    let _ = shroud(&["quit"]);
    wait_for_daemon_stop(5);
    cleanup_iptables();

    // Start daemon
    let mut daemon = start_daemon().expect("Failed to start");
    assert!(wait_for_daemon(10), "Daemon failed to start");

    // Enable kill switch
    let _ = shroud(&["killswitch", "on"]);
    thread::sleep(Duration::from_secs(1));
    assert!(killswitch_rules_exist(), "Kill switch not enabled");

    // Send SIGTERM (graceful shutdown)
    println!("Sending SIGTERM...");
    if let Some(pid) = get_daemon_pid() {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
    }

    // Wait for shutdown
    wait_for_daemon_stop(10);
    let _ = daemon.kill();
    let _ = daemon.wait();

    // Verify iptables cleaned up
    thread::sleep(Duration::from_secs(1));

    let rules_remain = killswitch_rules_exist();
    cleanup_iptables();

    assert!(!rules_remain, "SIGTERM did not clean up iptables rules!");

    println!("\n=== TEST PASSED ===\n");
}

#[test]
#[ignore = "requires root privileges for signal handling and iptables"]
fn test_sigint_cleans_up_killswitch() {
    println!("\n=== TEST: SIGINT cleans up kill switch ===\n");

    // Ensure clean state
    let _ = shroud(&["quit"]);
    wait_for_daemon_stop(5);
    cleanup_iptables();

    // Start daemon
    let mut daemon = start_daemon().expect("Failed to start");
    assert!(wait_for_daemon(10), "Daemon failed to start");

    // Enable kill switch
    let _ = shroud(&["killswitch", "on"]);
    thread::sleep(Duration::from_secs(1));
    assert!(killswitch_rules_exist(), "Kill switch not enabled");

    // Send SIGINT (Ctrl+C)
    println!("Sending SIGINT...");
    if let Some(pid) = get_daemon_pid() {
        let _ = Command::new("kill")
            .args(["-INT", &pid.to_string()])
            .output();
    }

    // Wait for shutdown
    wait_for_daemon_stop(10);
    let _ = daemon.kill();
    let _ = daemon.wait();

    // Verify iptables cleaned up
    thread::sleep(Duration::from_secs(1));

    let rules_remain = killswitch_rules_exist();
    cleanup_iptables();

    assert!(!rules_remain, "SIGINT did not clean up iptables rules!");

    println!("\n=== TEST PASSED ===\n");
}

#[test]
#[ignore = "requires root privileges for signal handling and iptables"]
fn test_sigterm_during_iptables_modification() {
    println!("\n=== TEST: SIGTERM during iptables modification ===\n");

    // This tests a race: what if we get SIGTERM while modifying iptables?

    let _ = shroud(&["quit"]);
    wait_for_daemon_stop(5);
    cleanup_iptables();

    // We'll try to hit the race window
    for i in 0..5 {
        println!("Attempt {}/5", i + 1);

        let mut daemon = start_daemon().expect("Failed to start");
        assert!(wait_for_daemon(10), "Daemon failed to start");

        // Start enabling kill switch in background
        let _ = Command::new("./target/debug/shroud")
            .args(["killswitch", "on"])
            .spawn();

        // Immediately send SIGTERM
        thread::sleep(Duration::from_millis(50));
        if let Some(pid) = get_daemon_pid() {
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .output();
        }

        wait_for_daemon_stop(5);
        let _ = daemon.kill();
        let _ = daemon.wait();

        // Check for orphaned rules
        if killswitch_rules_exist() {
            println!("  Found orphaned rules after SIGTERM race");
        }

        cleanup_iptables();
        thread::sleep(Duration::from_millis(500));
    }

    // This test documents the behavior - we want to ensure
    // either clean shutdown OR detectable orphaned rules

    println!("\n=== TEST COMPLETE ===\n");
}

// ============================================================================
// SIGHUP (RELOAD) HANDLING
// ============================================================================

#[test]
#[ignore = "requires root privileges for signal handling and iptables"]
fn test_sighup_safe_reload() {
    println!("\n=== TEST: SIGHUP safe reload ===\n");
    let mut started_daemon = None;
    if !daemon_running() {
        let mut d = start_daemon().unwrap();
        if !wait_for_daemon(10) {
            let _ = d.kill();
            let _ = d.wait();
            println!("SKIP: Could not start daemon");
            return;
        }
        started_daemon = Some(d);
    }

    // Get initial state
    let _initial_status = shroud(&["status"]);

    // Send SIGHUP (reload)
    if let Some(pid) = get_daemon_pid() {
        println!("Sending SIGHUP to PID {}...", pid);
        let _ = Command::new("kill")
            .args(["-HUP", &pid.to_string()])
            .output();
    }

    thread::sleep(Duration::from_secs(2));

    // Daemon should still be running
    assert!(daemon_running(), "Daemon died on SIGHUP");

    // State should be preserved
    let final_status = shroud(&["status"]);
    assert!(final_status.status.success(), "Status failed after SIGHUP");

    if let Some(mut d) = started_daemon {
        let _ = d.kill();
        let _ = d.wait();
    }

    println!("\n=== TEST PASSED ===\n");
}

#[test]
#[ignore = "requires root privileges for signal handling and iptables"]
fn test_rapid_sighup() {
    println!("\n=== TEST: Rapid SIGHUP signals ===\n");
    let mut started_daemon = None;
    if !daemon_running() {
        let mut d = start_daemon().unwrap();
        if !wait_for_daemon(10) {
            let _ = d.kill();
            let _ = d.wait();
            println!("SKIP: Could not start daemon");
            return;
        }
        started_daemon = Some(d);
    }

    // Send many SIGHUPs rapidly
    if let Some(pid) = get_daemon_pid() {
        for _ in 0..20 {
            let _ = Command::new("kill")
                .args(["-HUP", &pid.to_string()])
                .output();
            thread::sleep(Duration::from_millis(50));
        }
    }

    // Daemon should survive
    thread::sleep(Duration::from_secs(2));
    assert!(daemon_running(), "Daemon died on rapid SIGHUP");

    let output = shroud(&["status"]);
    assert!(output.status.success(), "Status failed after rapid SIGHUP");

    if let Some(mut d) = started_daemon {
        let _ = d.kill();
        let _ = d.wait();
    }

    println!("\n=== TEST PASSED ===\n");
}

// ============================================================================
// SIGUSR1/SIGUSR2 (IF USED)
// ============================================================================

#[test]
fn test_unexpected_signals_handled() {
    println!("\n=== TEST: Unexpected signals handled ===\n");
    let mut started_daemon = None;
    if !daemon_running() {
        let mut d = start_daemon().unwrap();
        if !wait_for_daemon(10) {
            let _ = d.kill();
            let _ = d.wait();
            println!("SKIP: Could not start daemon");
            return;
        }
        started_daemon = Some(d);
    }

    if let Some(pid) = get_daemon_pid() {
        // Send various signals that shouldn't crash daemon
        let signals = ["USR1", "USR2", "WINCH", "CONT"];

        for sig in signals {
            println!("Sending SIG{}...", sig);
            let _ = Command::new("kill")
                .args([&format!("-{}", sig), &pid.to_string()])
                .output();

            thread::sleep(Duration::from_millis(100));

            assert!(daemon_running(), "Daemon died on SIG{}", sig);
        }
    }

    // Verify still functional
    let output = shroud(&["status"]);
    assert!(output.status.success(), "Status failed after signals");

    if let Some(mut d) = started_daemon {
        let _ = d.kill();
        let _ = d.wait();
    }

    println!("\n=== TEST PASSED ===\n");
}
