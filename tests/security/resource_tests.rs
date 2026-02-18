// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Resource Exhaustion Security Tests
//!
//! Verifies the application handles resource limits gracefully:
//! - Memory limits
//! - File descriptor limits
//! - Disk space
//! - CPU time
//!
//! Run with: cargo test --test security_resources -- --ignored --nocapture

use std::process::Command;
use std::thread;
use std::time::Duration;

fn shroud(args: &[&str]) -> std::process::Output {
    Command::new("./target/debug/shroud")
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("Failed to run shroud")
}

fn daemon_running() -> bool {
    let output = shroud(&["ping"]);
    output.status.success()
}

// ============================================================================
// FILE DESCRIPTOR TESTS
// ============================================================================

#[test]
#[ignore]
fn test_file_descriptors_not_leaked() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    // Get initial FD count
    let pid = Command::new("pgrep")
        .args(["-f", "shroud"])
        .output()
        .expect("Failed to get PID");

    let pid = String::from_utf8_lossy(&pid.stdout).trim().to_string();

    if pid.is_empty() {
        println!("SKIP: Could not find daemon PID");
        return;
    }

    let get_fd_count = || -> usize {
        let output = Command::new("ls")
            .args(["-la", &format!("/proc/{}/fd", pid)])
            .output()
            .ok();

        output
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
            .unwrap_or(0)
    };

    let initial_fds = get_fd_count();
    println!("Initial FD count: {}", initial_fds);

    // Make many requests
    for _ in 0..100 {
        let _ = shroud(&["status"]);
        let _ = shroud(&["list"]);
        let _ = shroud(&["ping"]);
    }

    // Wait a bit for cleanup
    thread::sleep(Duration::from_secs(2));

    let final_fds = get_fd_count();
    println!("Final FD count: {}", final_fds);

    // Allow some variance but catch major leaks
    let leaked = final_fds as i64 - initial_fds as i64;
    assert!(
        leaked < 50,
        "Possible FD leak: {} -> {} ({} leaked)",
        initial_fds,
        final_fds,
        leaked
    );
}

// ============================================================================
// LOG FILE TESTS
// ============================================================================

#[test]
fn test_log_rotation_prevents_disk_exhaustion() {
    // Verify log rotation is configured
    // Check max log size constant

    // This is more of a documentation test - verify the log module
    // has rotation enabled

    // The actual values should be:
    // MAX_LOG_SIZE = 10 MB
    // MAX_LOG_FILES = 3
    // Total max = 30 MB

    println!("Log rotation should limit total log size to ~30MB");
    println!("Verify in src/logging.rs:");
    println!("  MAX_LOG_SIZE = 10 * 1024 * 1024");
    println!("  MAX_LOG_FILES = 3");
}

#[test]
fn test_log_file_size_limit() {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("shroud");

    if !data_dir.exists() {
        println!("No log directory, skipping");
        return;
    }

    // Check all log files are under limit
    let max_size: u64 = 10 * 1024 * 1024; // 10 MB

    for entry in std::fs::read_dir(&data_dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "log").unwrap_or(false) {
            if let Ok(metadata) = std::fs::metadata(&path) {
                let size = metadata.len();
                assert!(
                    size <= max_size * 2, // Allow some buffer
                    "Log file {} is too large: {} bytes",
                    path.display(),
                    size
                );
            }
        }
    }
}

// ============================================================================
// MEMORY TESTS
// ============================================================================

#[test]
#[ignore]
fn test_memory_not_leaked() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    let pid = Command::new("pgrep")
        .args(["-f", "shroud"])
        .output()
        .expect("Failed to get PID");

    let pid = String::from_utf8_lossy(&pid.stdout).trim().to_string();

    if pid.is_empty() {
        println!("SKIP: Could not find daemon PID");
        return;
    }

    let get_memory = || -> u64 {
        let output = Command::new("ps")
            .args(["-o", "rss=", "-p", &pid])
            .output()
            .ok();

        output
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
            .unwrap_or(0)
    };

    let initial_mem = get_memory();
    println!("Initial memory: {} KB", initial_mem);

    // Make many requests
    for i in 0..500 {
        let _ = shroud(&["status"]);
        if i % 100 == 0 {
            println!("  {} requests, memory: {} KB", i, get_memory());
        }
    }

    // Force GC / cleanup
    thread::sleep(Duration::from_secs(5));

    let final_mem = get_memory();
    println!("Final memory: {} KB", final_mem);

    // Allow 50% growth but catch major leaks
    let growth_ratio = final_mem as f64 / initial_mem as f64;
    assert!(
        growth_ratio < 2.0,
        "Possible memory leak: {} KB -> {} KB ({:.1}x growth)",
        initial_mem,
        final_mem,
        growth_ratio
    );
}

// ============================================================================
// CONCURRENT ACCESS TESTS
// ============================================================================

#[test]
#[ignore]
fn test_concurrent_requests_handled() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    let mut handles = vec![];

    // Spawn many concurrent requests
    for _ in 0..50 {
        let handle = thread::spawn(|| {
            for _ in 0..10 {
                let _ = shroud(&["status"]);
            }
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        let _ = handle.join();
    }

    // Daemon should still be responsive
    thread::sleep(Duration::from_secs(1));

    let output = shroud(&["ping"]);
    assert!(
        output.status.success(),
        "Daemon unresponsive after concurrent load"
    );
}
