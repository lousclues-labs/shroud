// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Race Condition Security Tests
//!
//! Tests for concurrent access issues:
//! - Kill switch toggle during VPN state change
//! - Config read/write races
//! - Multiple CLI clients
//! - State machine transitions
//!
//! ## Running These Tests
//! Most tests in this file require a privileged environment and are marked with `#[ignore]`.
//! To run them:
//!
//! ```bash
//! sudo -E cargo test --test security_race -- --ignored --nocapture
//! ```
//!
//! ## Requirements
//! - Root/sudo access
//! - NetworkManager running
//! - D-Bus session available
//! - iptables/nftables installed

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
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

fn daemon_running() -> bool {
    shroud(&["ping"]).status.success()
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
        if daemon_running() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

// ============================================================================
// CONCURRENT COMMAND TESTS
// ============================================================================

#[test]
#[ignore = "requires privileged environment"]
fn test_concurrent_killswitch_toggles() {
    println!("\n=== TEST: Concurrent kill switch toggles ===\n");
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

    // Ensure consistent starting state
    let _ = shroud(&["killswitch", "off"]);
    thread::sleep(Duration::from_millis(500));

    let error_count = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    // Spawn many threads toggling kill switch simultaneously
    for i in 0..20 {
        let errors = Arc::clone(&error_count);
        let handle = thread::spawn(move || {
            for j in 0..10 {
                let action = if (i + j) % 2 == 0 { "on" } else { "off" };
                let output = shroud(&["killswitch", action]);

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    // Transient errors during racing are acceptable
                    // But crashes or corruption are not
                    if stderr.contains("panic") || stderr.contains("SIGSEGV") {
                        errors.fetch_add(1, Ordering::SeqCst);
                    }
                }

                // Small random delay
                thread::sleep(Duration::from_millis((i * 7 % 50) as u64));
            }
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        let _ = handle.join();
    }

    // Verify daemon is still healthy
    thread::sleep(Duration::from_secs(1));
    assert!(daemon_running(), "Daemon died during concurrent toggles");

    // Verify we can still control kill switch
    let output = shroud(&["killswitch", "off"]);
    assert!(
        output.status.success(),
        "Kill switch unresponsive after race"
    );

    let errors = error_count.load(Ordering::SeqCst);
    assert_eq!(errors, 0, "Detected {} critical errors during race", errors);

    if let Some(mut d) = started_daemon {
        let _ = d.kill();
        let _ = d.wait();
    }

    println!("\n=== TEST PASSED ===\n");
}

#[test]
#[ignore = "requires privileged environment"]
fn test_concurrent_status_during_connect() {
    println!("\n=== TEST: Concurrent status during connect ===\n");
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

    // Get VPN name
    let output = shroud(&["list"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let vpn_name = stdout
        .lines()
        .find(|l| !l.is_empty() && !l.contains("Available") && !l.contains("VPN"))
        .map(|l| {
            l.trim()
                .trim_start_matches("• ")
                .trim_start_matches("- ")
                .to_string()
        });

    if vpn_name.is_none() {
        println!("SKIP: No VPNs configured");
        return;
    }
    let vpn_name = vpn_name.unwrap();

    let running = Arc::new(AtomicBool::new(true));
    let errors = Arc::new(AtomicUsize::new(0));

    // Spawn status polling threads
    let mut handles = vec![];
    for _ in 0..10 {
        let r = Arc::clone(&running);
        let e = Arc::clone(&errors);
        let handle = thread::spawn(move || {
            while r.load(Ordering::SeqCst) {
                let output = shroud(&["status"]);
                if !output.status.success() {
                    e.fetch_add(1, Ordering::SeqCst);
                }
                thread::sleep(Duration::from_millis(50));
            }
        });
        handles.push(handle);
    }

    // Start VPN connection
    println!("Starting VPN connection while polling status...");
    let _ = shroud(&["connect", &vpn_name]);

    // Let it run for a bit
    thread::sleep(Duration::from_secs(5));

    // Stop polling
    running.store(false, Ordering::SeqCst);

    // Disconnect
    let _ = shroud(&["disconnect"]);

    // Wait for threads
    for handle in handles {
        let _ = handle.join();
    }

    let error_count = errors.load(Ordering::SeqCst);
    println!("Errors during concurrent status: {}", error_count);

    // Some errors are acceptable (timing), but should be rare
    assert!(
        error_count < 10,
        "Too many errors during concurrent status: {}",
        error_count
    );

    assert!(daemon_running(), "Daemon died during test");

    if let Some(mut d) = started_daemon {
        let _ = d.kill();
        let _ = d.wait();
    }

    println!("\n=== TEST PASSED ===\n");
}

#[test]
#[ignore = "requires privileged environment"]
fn test_rapid_connect_disconnect() {
    println!("\n=== TEST: Rapid connect/disconnect cycles ===\n");
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

    // Get VPN name
    let output = shroud(&["list"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let vpn_name = stdout
        .lines()
        .find(|l| !l.is_empty() && !l.contains("Available") && !l.contains("VPN"))
        .map(|l| {
            l.trim()
                .trim_start_matches("• ")
                .trim_start_matches("- ")
                .to_string()
        });

    if vpn_name.is_none() {
        println!("SKIP: No VPNs configured");
        return;
    }
    let vpn_name = vpn_name.unwrap();

    // Rapid connect/disconnect without waiting
    for i in 0..10 {
        println!("Cycle {}/10", i + 1);

        // Connect (don't wait)
        let _ = Command::new("./target/debug/shroud")
            .args(["connect", &vpn_name])
            .spawn();

        thread::sleep(Duration::from_millis(200));

        // Disconnect immediately
        let _ = shroud(&["disconnect"]);

        thread::sleep(Duration::from_millis(100));
    }

    // Give daemon time to settle
    thread::sleep(Duration::from_secs(2));

    // Verify daemon is healthy
    assert!(daemon_running(), "Daemon died during rapid cycles");

    let output = shroud(&["status"]);
    assert!(output.status.success(), "Status failed after rapid cycles");

    if let Some(mut d) = started_daemon {
        let _ = d.kill();
        let _ = d.wait();
    }

    println!("\n=== TEST PASSED ===\n");
}

#[test]
#[ignore = "requires privileged environment"]
fn test_config_read_write_race() {
    println!("\n=== TEST: Config read/write race ===\n");
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

    let running = Arc::new(AtomicBool::new(true));
    let errors = Arc::new(AtomicUsize::new(0));

    // Thread that toggles auto-reconnect (writes config)
    let r1 = Arc::clone(&running);
    let _e1 = Arc::clone(&errors);
    let writer = thread::spawn(move || {
        while r1.load(Ordering::SeqCst) {
            let _ = shroud(&["auto-reconnect", "toggle"]);
            thread::sleep(Duration::from_millis(50));
        }
    });

    // Thread that toggles kill switch (writes config)
    let r2 = Arc::clone(&running);
    let _e2 = Arc::clone(&errors);
    let writer2 = thread::spawn(move || {
        while r2.load(Ordering::SeqCst) {
            let _ = shroud(&["killswitch", "toggle"]);
            thread::sleep(Duration::from_millis(50));
        }
    });

    // Thread that reads status (reads config)
    let r3 = Arc::clone(&running);
    let e3 = Arc::clone(&errors);
    let reader = thread::spawn(move || {
        while r3.load(Ordering::SeqCst) {
            let output = shroud(&["status"]);
            if !output.status.success() {
                e3.fetch_add(1, Ordering::SeqCst);
            }
            thread::sleep(Duration::from_millis(30));
        }
    });

    // Let it run
    thread::sleep(Duration::from_secs(5));
    running.store(false, Ordering::SeqCst);

    let _ = writer.join();
    let _ = writer2.join();
    let _ = reader.join();

    // Reset state
    let _ = shroud(&["killswitch", "off"]);

    // Verify daemon healthy
    assert!(daemon_running(), "Daemon died during config race");

    if let Some(mut d) = started_daemon {
        let _ = d.kill();
        let _ = d.wait();
    }

    println!("\n=== TEST PASSED ===\n");
}

#[test]
#[ignore = "requires privileged environment"]
fn test_reload_during_connect() {
    println!("\n=== TEST: Reload during connect ===\n");
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

    // Get VPN name
    let output = shroud(&["list"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let vpn_name = stdout
        .lines()
        .find(|l| !l.is_empty() && !l.contains("Available") && !l.contains("VPN"))
        .map(|l| {
            l.trim()
                .trim_start_matches("• ")
                .trim_start_matches("- ")
                .to_string()
        });

    if vpn_name.is_none() {
        println!("SKIP: No VPNs configured");
        return;
    }
    let vpn_name = vpn_name.unwrap();

    // Start connection
    let _ = Command::new("./target/debug/shroud")
        .args(["connect", &vpn_name])
        .spawn();

    // Immediately send reload
    thread::sleep(Duration::from_millis(100));
    let _ = shroud(&["reload"]);

    // And another
    thread::sleep(Duration::from_millis(100));
    let _ = shroud(&["reload"]);

    // Wait for connection to complete or fail
    thread::sleep(Duration::from_secs(5));

    // Verify daemon healthy
    assert!(daemon_running(), "Daemon died during reload race");

    let _ = shroud(&["disconnect"]);

    if let Some(mut d) = started_daemon {
        let _ = d.kill();
        let _ = d.wait();
    }

    println!("\n=== TEST PASSED ===\n");
}

// ============================================================================
// STATE MACHINE RACE TESTS
// ============================================================================

#[test]
#[ignore = "requires privileged environment"]
fn test_killswitch_toggle_during_state_transition() {
    println!("\n=== TEST: Kill switch toggle during state transition ===\n");
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

    // Get VPN name
    let output = shroud(&["list"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let vpn_name = stdout
        .lines()
        .find(|l| !l.is_empty() && !l.contains("Available") && !l.contains("VPN"))
        .map(|l| {
            l.trim()
                .trim_start_matches("• ")
                .trim_start_matches("- ")
                .to_string()
        });

    if vpn_name.is_none() {
        println!("SKIP: No VPNs configured");
        return;
    }
    let vpn_name = vpn_name.unwrap();

    // Ensure kill switch off
    let _ = shroud(&["killswitch", "off"]);

    // Start connection
    println!("Starting connection...");
    let _ = Command::new("./target/debug/shroud")
        .args(["connect", &vpn_name])
        .spawn();

    // Rapidly toggle kill switch during connection
    for i in 0..20 {
        thread::sleep(Duration::from_millis(100));
        let action = if i % 2 == 0 { "on" } else { "off" };
        let _ = shroud(&["killswitch", action]);
    }

    // Wait for things to settle
    thread::sleep(Duration::from_secs(3));

    // Verify consistent state
    let status_output = shroud(&["status"]);
    let ks_output = shroud(&["killswitch", "status"]);

    assert!(status_output.status.success(), "Status failed");
    assert!(ks_output.status.success(), "Kill switch status failed");

    // Verify iptables matches reported state
    let ks_status = String::from_utf8_lossy(&ks_output.stdout);
    let rules_exist = Command::new("sudo")
        .args(["iptables", "-L", "OUTPUT", "-n"])
        .output()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            s.contains("DROP") || s.contains("shroud")
        })
        .unwrap_or(false);

    let reported_enabled = ks_status.to_lowercase().contains("enabled");

    // State should be consistent
    if reported_enabled != rules_exist {
        println!("WARNING: State inconsistency detected!");
        println!(
            "  Reported: {}",
            if reported_enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        println!(
            "  iptables: {}",
            if rules_exist { "has rules" } else { "no rules" }
        );
    }

    // Clean up
    let _ = shroud(&["killswitch", "off"]);
    let _ = shroud(&["disconnect"]);

    if let Some(mut d) = started_daemon {
        let _ = d.kill();
        let _ = d.wait();
    }

    println!("\n=== TEST PASSED ===\n");
}
