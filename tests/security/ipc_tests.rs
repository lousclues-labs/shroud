// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! IPC Security Tests
//!
//! Verifies the Unix socket is properly secured against:
//! - Unauthorized access
//! - Malformed input
//! - Command injection
//! - Resource exhaustion
//!
//! Run with: cargo test --test security_ipc -- --nocapture

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

/// Get the socket path
fn socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("shroud.sock")
    } else {
        PathBuf::from("/tmp").join(format!("shroud-{}.sock", unsafe { libc::getuid() }))
    }
}

/// Helper to check if daemon is running
fn daemon_running() -> bool {
    socket_path().exists()
}

/// Send raw bytes to socket and get response
fn send_raw(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut stream = UnixStream::connect(socket_path())?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    stream.write_all(data)?;
    stream.flush()?;

    let mut response = Vec::new();
    let mut buf = [0u8; 4096];

    match stream.read(&mut buf) {
        Ok(n) if n > 0 => response.extend_from_slice(&buf[..n]),
        _ => {}
    }

    Ok(response)
}

/// Send a line to socket and get response
fn send_line(line: &str) -> Result<String, std::io::Error> {
    let data = format!("{}\n", line);
    let response = send_raw(data.as_bytes())?;
    Ok(String::from_utf8_lossy(&response).to_string())
}

// ============================================================================
// SOCKET PERMISSION TESTS
// ============================================================================

#[test]
#[ignore] // Requires daemon running
fn test_socket_has_secure_permissions() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    let path = socket_path();
    let metadata = std::fs::metadata(&path).expect("Failed to get socket metadata");
    let mode = metadata.permissions().mode();

    // Socket should be 0o600 (owner read/write only) or 0o660 at most
    let perms = mode & 0o777;

    println!("Socket permissions: {:o}", perms);

    assert!(
        perms == 0o600 || perms == 0o660,
        "Socket has insecure permissions: {:o}. Expected 0600 or 0660.",
        perms
    );

    // Verify owner is current user
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let socket_uid = metadata.uid();
        let current_uid = unsafe { libc::getuid() };

        assert_eq!(
            socket_uid, current_uid,
            "Socket owned by UID {} but current user is UID {}",
            socket_uid, current_uid
        );
    }
}

#[test]
#[ignore]
fn test_socket_not_world_accessible() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    let path = socket_path();
    let metadata = std::fs::metadata(&path).expect("Failed to get socket metadata");
    let mode = metadata.permissions().mode();

    // Check no world permissions
    let world_perms = mode & 0o007;
    assert_eq!(
        world_perms,
        0,
        "Socket has world permissions: {:o}. This is a security risk!",
        mode & 0o777
    );

    // Check no group write permission (group read is acceptable)
    let group_write = mode & 0o020;
    assert_eq!(
        group_write,
        0,
        "Socket has group write permission: {:o}. This is a security risk!",
        mode & 0o777
    );
}

// ============================================================================
// MALFORMED INPUT TESTS
// ============================================================================

#[test]
#[ignore]
fn test_empty_message_handled() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    // Send empty line
    let result = send_line("");

    // Should get error response, not crash
    assert!(result.is_ok(), "Daemon crashed on empty message");

    // Verify daemon still running
    thread::sleep(Duration::from_millis(100));
    assert!(daemon_running(), "Daemon died after empty message");
}

#[test]
#[ignore]
fn test_invalid_json_handled() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    let invalid_inputs = vec![
        "not json at all",
        "{invalid json}",
        "{\"type\": }",
        "{{{{",
        "]]]]",
        "null",
        "true",
        "12345",
        "[1,2,3]",
        "{\"type\": \"Connect\"}",               // Missing required field
        "{\"type\": \"Unknown\", \"data\": {}}", // Unknown command type
    ];

    for input in invalid_inputs {
        println!("Testing invalid input: {}", input);

        let result = send_line(input);
        assert!(result.is_ok(), "Daemon crashed on input: {}", input);

        let response = result.unwrap();
        // Should get an error response
        assert!(
            response.contains("error") || response.contains("Error") || response.is_empty(),
            "Expected error response for invalid input '{}', got: {}",
            input,
            response
        );

        // Verify daemon still running
        thread::sleep(Duration::from_millis(50));
        assert!(daemon_running(), "Daemon died after input: {}", input);
    }
}

#[test]
#[ignore]
fn test_binary_garbage_handled() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    // Send random binary data
    let garbage: Vec<u8> = (0..256).map(|i| i as u8).collect();

    let _result = send_raw(&garbage);

    // Should not crash (may return error or close connection)
    // The important thing is daemon survives
    thread::sleep(Duration::from_millis(100));
    assert!(daemon_running(), "Daemon died after binary garbage");
}

#[test]
#[ignore]
fn test_null_bytes_handled() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    // JSON with embedded null bytes
    let payload = b"{\"type\": \"Ping\", \x00\"data\": null}\n";

    let _result = send_raw(payload);

    thread::sleep(Duration::from_millis(100));
    assert!(daemon_running(), "Daemon died after null bytes");
}

#[test]
#[ignore]
fn test_unicode_edge_cases_handled() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    let long_name = "🔒".repeat(1000);
    let unicode_cases = vec![
        "{\"type\": \"Connect\", \"data\": {\"name\": \"\u{FEFF}test\"}}".to_string(),
        "{\"type\": \"Connect\", \"data\": {\"name\": \"test\u{202E}evil\"}}".to_string(),
        "{\"type\": \"Connect\", \"data\": {\"name\": \"test\\u0000evil\"}}".to_string(),
        format!(
            "{{\"type\": \"Connect\", \"data\": {{\"name\": \"{}\"}}}}",
            long_name
        ),
    ];

    for input in unicode_cases {
        println!(
            "Testing Unicode edge case: {}...",
            &input[..input.len().min(50)]
        );

        let _result = send_line(&input);

        thread::sleep(Duration::from_millis(50));
        assert!(daemon_running(), "Daemon died after Unicode edge case");
    }
}

// ============================================================================
// OVERSIZED MESSAGE TESTS
// ============================================================================

#[test]
#[ignore]
fn test_oversized_message_rejected() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    // Create a very large message (1MB)
    let large_name = "A".repeat(1024 * 1024);
    let payload = format!(
        "{{\"type\": \"Connect\", \"data\": {{\"name\": \"{}\"}}}}\n",
        large_name
    );

    let _result = send_raw(payload.as_bytes());

    // Should either reject or handle gracefully
    thread::sleep(Duration::from_millis(500));
    assert!(daemon_running(), "Daemon died after oversized message");
}

#[test]
#[ignore]
fn test_many_rapid_connections_handled() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    // Open many connections rapidly
    let mut handles = vec![];

    for i in 0..100 {
        let handle = thread::spawn(move || {
            if let Ok(mut stream) = UnixStream::connect(socket_path()) {
                let _ = stream.write_all(b"{\"type\": \"Ping\"}\n");
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
            }
        });
        handles.push(handle);

        // Small delay to avoid overwhelming
        if i % 10 == 0 {
            thread::sleep(Duration::from_millis(10));
        }
    }

    // Wait for all to complete
    for handle in handles {
        let _ = handle.join();
    }

    // Daemon should still be running
    thread::sleep(Duration::from_millis(500));
    assert!(daemon_running(), "Daemon died after many rapid connections");
}

#[test]
#[ignore]
fn test_slow_loris_handled() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    // Open connection and send data very slowly
    let handle = thread::spawn(|| {
        if let Ok(mut stream) = UnixStream::connect(socket_path()) {
            // Send one byte at a time with delays
            let payload = b"{\"type\": \"Ping\"}\n";
            for byte in payload {
                let _ = stream.write_all(&[*byte]);
                let _ = stream.flush();
                thread::sleep(Duration::from_millis(100));
            }
        }
    });

    // Meanwhile, verify daemon still accepts other connections
    thread::sleep(Duration::from_millis(500));

    let result = send_line("{\"type\": \"Ping\"}");
    assert!(result.is_ok(), "Daemon blocked by slow loris attack");

    let _ = handle.join();
}

// ============================================================================
// COMMAND INJECTION TESTS
// ============================================================================

#[test]
#[ignore]
fn test_vpn_name_command_injection() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    // Attempt command injection via VPN name
    let malicious_names = vec![
        "; rm -rf /",
        "$(whoami)",
        "`whoami`",
        "| cat /etc/passwd",
        "&& cat /etc/shadow",
        "test\"; cat /etc/passwd; echo \"",
        "test' || cat /etc/passwd || echo '",
        "../../../etc/passwd",
        "test\ncat /etc/passwd",
        "test\x00cat /etc/passwd",
    ];

    for name in malicious_names {
        println!("Testing injection: {}", name.escape_debug());

        let payload = serde_json::json!({
            "type": "Connect",
            "data": {"name": name}
        });

        let result = send_line(&payload.to_string());

        // Should get error (VPN not found), not execute command
        if let Ok(response) = result {
            assert!(
                !response.contains("root:") && !response.contains("/bin/bash"),
                "Possible command injection! Response: {}",
                response
            );
        }

        thread::sleep(Duration::from_millis(50));
        assert!(
            daemon_running(),
            "Daemon died after injection attempt: {}",
            name.escape_debug()
        );
    }
}

#[test]
#[ignore]
fn test_path_traversal_in_names() {
    if !daemon_running() {
        println!("SKIP: Daemon not running");
        return;
    }

    let traversal_attempts = vec![
        "../../../etc/passwd",
        "..\\..\\..\\etc\\passwd",
        "....//....//etc/passwd",
        "%2e%2e%2f%2e%2e%2f",
        "..%00/etc/passwd",
        "/etc/passwd",
        "file:///etc/passwd",
    ];

    for path in traversal_attempts {
        println!("Testing path traversal: {}", path);

        let payload = serde_json::json!({
            "type": "Connect",
            "data": {"name": path}
        });

        let result = send_line(&payload.to_string());

        if let Ok(response) = result {
            // Should not contain file contents
            assert!(
                !response.contains("root:x:0:0"),
                "Path traversal succeeded! Response: {}",
                response
            );
        }

        thread::sleep(Duration::from_millis(50));
        assert!(daemon_running(), "Daemon died after traversal: {}", path);
    }
}
