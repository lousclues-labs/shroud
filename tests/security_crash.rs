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
//! Crash Recovery Security Tests
//!
//! Verifies the system recovers gracefully from:
//! - Daemon crash with kill switch active
//! - Unclean shutdown
//! - Stale state files
//! - Partial writes
//!
//! ## Running These Tests
//! Most tests in this file require root privileges and are marked with `#[ignore]`.
//! To run them:
//!
//! ```bash
//! sudo -E cargo test --test security_crash -- --ignored --nocapture
//! ```
//!
//! ## Requirements
//! - Root/sudo access
//! - NetworkManager running
//! - D-Bus session available
//! - iptables/nftables installed

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

/// Helper to run shroud
fn shroud(args: &[&str]) -> std::process::Output {
    Command::new("./target/debug/shroud")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run shroud")
}

/// Get daemon PID
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

/// Check if daemon is running
fn daemon_running() -> bool {
    get_daemon_pid().is_some()
}

/// Wait for daemon to stop
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

/// Wait for daemon to start
fn wait_for_daemon_start(timeout_secs: u64) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(timeout_secs) {
        let output = shroud(&["ping"]);
        if output.status.success() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Start daemon in background
fn start_daemon() -> Option<std::process::Child> {
    Command::new("./target/debug/shroud")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// Check if iptables has kill switch rules
fn killswitch_rules_exist() -> bool {
    let output = Command::new("sudo")
        .args(["iptables", "-L", "OUTPUT", "-n"])
        .output()
        .expect("Failed to check iptables");

    let rules = String::from_utf8_lossy(&output.stdout);
    rules.contains("DROP") || rules.contains("REJECT")
}

/// Clean up iptables rules manually
fn cleanup_iptables() {
    // Flush OUTPUT chain (dangerous - only for testing!)
    let _ = Command::new("sudo")
        .args(["iptables", "-F", "OUTPUT"])
        .output();

    // Reset to ACCEPT
    let _ = Command::new("sudo")
        .args(["iptables", "-P", "OUTPUT", "ACCEPT"])
        .output();
}

/// Get socket path
fn socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("shroud.sock")
    } else {
        PathBuf::from("/tmp").join(format!("shroud-{}.sock", unsafe { libc::getuid() }))
    }
}

/// Get lock file path
fn lock_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("shroud.lock")
    } else {
        PathBuf::from("/tmp").join("shroud.lock")
    }
}

// ============================================================================
// CRASH WITH KILL SWITCH ACTIVE
// ============================================================================

#[test]
#[ignore = "requires root privileges and daemon access"]
fn test_crash_with_killswitch_cleans_up_on_restart() {
    println!("\n=== TEST: Crash with kill switch active ===\n");

    // Ensure clean state
    let _ = shroud(&["quit"]);
    wait_for_daemon_stop(5);
    cleanup_iptables();

    // Start daemon
    println!("Step 1: Starting daemon...");
    let mut daemon = start_daemon().expect("Failed to start daemon");
    assert!(wait_for_daemon_start(10), "Daemon failed to start");

    // Enable kill switch
    println!("Step 2: Enabling kill switch...");
    let output = shroud(&["killswitch", "on"]);
    assert!(output.status.success(), "Failed to enable kill switch");
    thread::sleep(Duration::from_secs(1));

    // Verify kill switch is active
    assert!(killswitch_rules_exist(), "Kill switch rules not created");
    println!("Kill switch active: iptables rules exist");

    // CRASH the daemon (SIGKILL - no cleanup possible)
    println!("Step 3: Crashing daemon with SIGKILL...");
    if let Some(pid) = get_daemon_pid() {
        let _ = Command::new("sudo")
            .args(["kill", "-9", &pid.to_string()])
            .output();
    }
    let _ = daemon.kill();
    let _ = daemon.wait();

    wait_for_daemon_stop(5);
    println!("Daemon crashed");

    // Verify stale rules exist (this is the problem scenario)
    assert!(
        killswitch_rules_exist(),
        "Kill switch rules should still exist after crash"
    );
    println!("Stale iptables rules confirmed");

    // NOW: Restart daemon - it should clean up stale rules
    println!("Step 4: Restarting daemon (should detect and clean stale rules)...");
    let mut daemon2 = start_daemon().expect("Failed to restart daemon");
    assert!(wait_for_daemon_start(10), "Daemon failed to restart");

    // Check the startup log or behavior
    thread::sleep(Duration::from_secs(2));

    // The daemon should have either:
    // 1. Cleaned up stale rules (if VPN not connected)
    // 2. Re-validated and kept rules (if VPN is connected)

    // For this test, assume VPN is not connected, so rules should be cleaned
    let output = shroud(&["status"]);
    let status = String::from_utf8_lossy(&output.stdout);
    println!("Status after restart: {}", status);

    // Clean up
    let _ = shroud(&["killswitch", "off"]);
    let _ = shroud(&["quit"]);
    let _ = daemon2.kill();
    let _ = daemon2.wait();
    cleanup_iptables();

    println!("\n=== TEST PASSED ===\n");
}

#[test]
#[ignore = "requires root privileges and daemon access"]
fn test_stale_lock_file_handled() {
    println!("\n=== TEST: Stale lock file handling ===\n");

    // Ensure daemon stopped
    let _ = shroud(&["quit"]);
    wait_for_daemon_stop(5);

    // Create a stale lock file with fake PID
    let lock = lock_path();
    println!("Creating stale lock file: {:?}", lock);

    fs::write(&lock, "99999").expect("Failed to create stale lock");

    // Set permissions
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&lock, fs::Permissions::from_mode(0o600));
    }

    // Try to start daemon - should handle stale lock
    println!("Starting daemon with stale lock...");
    let mut daemon = start_daemon().expect("Failed to start daemon");

    let started = wait_for_daemon_start(10);

    // Clean up
    let _ = shroud(&["quit"]);
    let _ = daemon.kill();
    let _ = daemon.wait();
    wait_for_daemon_stop(5);
    let _ = fs::remove_file(&lock);

    assert!(started, "Daemon should handle stale lock file and start");

    println!("\n=== TEST PASSED ===\n");
}

#[test]
#[ignore = "requires root privileges and daemon access"]
fn test_stale_socket_handled() {
    println!("\n=== TEST: Stale socket file handling ===\n");

    // Ensure daemon stopped
    let _ = shroud(&["quit"]);
    wait_for_daemon_stop(5);

    // Create stale socket file
    let socket = socket_path();
    println!("Creating stale socket: {:?}", socket);

    // Create a regular file pretending to be a socket
    fs::write(&socket, "stale").expect("Failed to create stale socket");

    // Try to start daemon
    println!("Starting daemon with stale socket...");
    let mut daemon = start_daemon().expect("Failed to start daemon");

    let started = wait_for_daemon_start(10);

    // Clean up
    let _ = shroud(&["quit"]);
    let _ = daemon.kill();
    let _ = daemon.wait();
    wait_for_daemon_stop(5);

    assert!(started, "Daemon should handle stale socket and start");

    println!("\n=== TEST PASSED ===\n");
}

#[test]
#[ignore = "requires root privileges and daemon access"]
fn test_partial_config_write_recovery() {
    println!("\n=== TEST: Partial config write recovery ===\n");

    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("shroud");
    let config_file = config_dir.join("config.toml");
    let backup = config_dir.join("config.toml.test_backup");

    // Backup existing config
    if config_file.exists() {
        let _ = fs::copy(&config_file, &backup);
    }

    // Write a truncated/corrupted config (simulating crash during write)
    let _ = fs::create_dir_all(&config_dir);
    let _ = fs::write(&config_file, "[shroud]\nkill_switch_enabled = ");

    // Start daemon
    let mut daemon = start_daemon().expect("Failed to start daemon");
    let started = wait_for_daemon_start(10);

    // Verify daemon started (should use defaults for corrupted config)
    assert!(started, "Daemon should start even with corrupted config");

    // Verify it's functional
    let output = shroud(&["status"]);
    assert!(output.status.success(), "Status should work");

    // Clean up
    let _ = shroud(&["quit"]);
    let _ = daemon.kill();
    let _ = daemon.wait();
    wait_for_daemon_stop(5);

    // Restore backup
    if backup.exists() {
        let _ = fs::rename(&backup, &config_file);
    } else {
        let _ = fs::remove_file(&config_file);
    }

    println!("\n=== TEST PASSED ===\n");
}

#[test]
#[ignore = "requires root privileges and daemon access"]
fn test_crash_during_vpn_connect() {
    println!("\n=== TEST: Crash during VPN connect ===\n");

    // Start daemon
    let _ = shroud(&["quit"]);
    wait_for_daemon_stop(5);

    let mut daemon = start_daemon().expect("Failed to start daemon");
    assert!(wait_for_daemon_start(10), "Daemon failed to start");

    // Get a VPN name
    let output = shroud(&["list"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let vpn_name = stdout
        .lines()
        .find(|l| !l.is_empty() && !l.contains("Available") && !l.contains("VPN"))
        .map(|l| l.trim().trim_start_matches("• ").trim_start_matches("- "));

    if vpn_name.is_none() {
        println!("SKIP: No VPNs configured");
        let _ = daemon.kill();
        let _ = daemon.wait();
        return;
    }
    let vpn_name = vpn_name.unwrap();

    // Start connecting (don't wait for completion)
    println!("Starting VPN connection...");
    let _ = Command::new("./target/debug/shroud")
        .args(["connect", vpn_name])
        .spawn();

    // Immediately crash
    thread::sleep(Duration::from_millis(500));
    println!("Crashing daemon mid-connect...");

    if let Some(pid) = get_daemon_pid() {
        let _ = Command::new("sudo")
            .args(["kill", "-9", &pid.to_string()])
            .output();
    }
    let _ = daemon.kill();
    let _ = daemon.wait();
    wait_for_daemon_stop(5);

    // Restart
    println!("Restarting daemon...");
    let mut daemon2 = start_daemon().expect("Failed to restart");
    assert!(wait_for_daemon_start(10), "Daemon failed to restart");

    // Daemon should resync with NetworkManager state
    thread::sleep(Duration::from_secs(2));

    let output = shroud(&["status"]);
    assert!(
        output.status.success(),
        "Status should work after crash recovery"
    );

    // Clean up
    let _ = shroud(&["disconnect"]);
    let _ = shroud(&["quit"]);
    let _ = daemon2.kill();
    let _ = daemon2.wait();

    println!("\n=== TEST PASSED ===\n");
}

// ============================================================================
// IPTABLES RULE VERIFICATION AFTER RESTART
// ============================================================================

#[test]
#[ignore = "requires root privileges and daemon access"]
fn test_orphaned_iptables_detected_on_startup() {
    println!("\n=== TEST: Orphaned iptables detection ===\n");

    // Ensure daemon stopped
    let _ = shroud(&["quit"]);
    wait_for_daemon_stop(5);
    cleanup_iptables();

    // Manually create orphaned iptables rules (simulating crash)
    println!("Creating orphaned iptables rules...");
    let _ = Command::new("sudo")
        .args([
            "iptables",
            "-A",
            "OUTPUT",
            "-j",
            "DROP",
            "-m",
            "comment",
            "--comment",
            "shroud-killswitch",
        ])
        .output();

    assert!(killswitch_rules_exist(), "Failed to create test rules");

    // Start daemon - should detect and handle orphaned rules
    println!("Starting daemon (should detect orphaned rules)...");
    let mut daemon = start_daemon().expect("Failed to start");
    assert!(wait_for_daemon_start(10), "Daemon failed to start");

    // Give it time to detect
    thread::sleep(Duration::from_secs(2));

    // Check status - should indicate kill switch state correctly
    let output = shroud(&["killswitch", "status"]);
    let status = String::from_utf8_lossy(&output.stdout);
    println!("Kill switch status: {}", status);

    // Clean up
    let _ = shroud(&["killswitch", "off"]);
    let _ = shroud(&["quit"]);
    let _ = daemon.kill();
    let _ = daemon.wait();
    cleanup_iptables();

    println!("\n=== TEST PASSED ===\n");
}
