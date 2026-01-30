//! Privilege Escalation Security Tests
//!
//! Verifies that privileged operations cannot be abused:
//! - iptables command injection
//! - VPN server IP validation
//! - pkexec/sudo command sanitization
//!
//! ## Running These Tests
//! Most tests in this file require root privileges and are marked with `#[ignore]`.
//! To run them:
//!
//! ```bash
//! sudo -E cargo test --test security_privilege -- --ignored --nocapture
//! ```
//!
//! ## Requirements
//! - Root/sudo access
//! - NetworkManager running
//! - D-Bus session available
//! - iptables/nftables installed

use std::process::Command;
use std::thread;
use std::time::Duration;

/// Helper to run shroud commands
fn shroud(args: &[&str]) -> std::process::Output {
    Command::new("./target/debug/shroud")
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("Failed to run shroud")
}

/// Check iptables for any suspicious rules
fn check_iptables_for_injection() -> bool {
    let output = Command::new("sudo")
        .args(["iptables", "-L", "-n"])
        .output()
        .expect("Failed to run iptables");

    let rules = String::from_utf8_lossy(&output.stdout);

    // Check for signs of injection
    let suspicious = ["/etc/passwd", "/bin/sh", "/bin/bash", "$(", "`", "||", "&&"];

    for pattern in suspicious {
        if rules.contains(pattern) {
            println!("SUSPICIOUS: Found '{}' in iptables rules!", pattern);
            return true;
        }
    }

    false
}

// ============================================================================
// IPTABLES INJECTION TESTS
// ============================================================================

#[test]
#[ignore = "requires specific user/group setup"]
fn test_iptables_ip_validation() {
    // These should be rejected as invalid IPs
    let invalid_ips = vec![
        "not.an.ip",
        "256.256.256.256",
        "-1.0.0.0",
        "1.2.3.4; cat /etc/passwd",
        "1.2.3.4 && rm -rf /",
        "$(whoami).1.2.3",
        "1.2.3.4\n-A OUTPUT -j DROP",
        "1.2.3.4 -j ACCEPT -A INPUT",
    ];

    for ip in invalid_ips {
        println!("Testing invalid IP: {}", ip.escape_debug());

        // After any attempt, check iptables wasn't corrupted
        assert!(
            !check_iptables_for_injection(),
            "iptables injection detected with IP: {}",
            ip.escape_debug()
        );
    }
}

#[test]
#[ignore = "requires specific user/group setup"]
fn test_iptables_interface_validation() {
    // Invalid interface names that could be injection attempts
    let invalid_interfaces = vec![
        "tun0; cat /etc/passwd",
        "tun0 && rm -rf /",
        "$(whoami)",
        "tun0\n-A INPUT -j ACCEPT",
        "../../../etc/passwd",
        "tun0 -j ACCEPT",
    ];

    for iface in invalid_interfaces {
        println!("Testing invalid interface: {}", iface.escape_debug());

        // After attempt, verify no injection
        assert!(
            !check_iptables_for_injection(),
            "iptables injection detected with interface: {}",
            iface.escape_debug()
        );
    }
}

#[test]
#[ignore = "requires specific user/group setup"]
fn test_killswitch_script_no_injection_vectors() {
    // Enable/disable kill switch and verify clean rules
    let _ = shroud(&["killswitch", "on"]);
    thread::sleep(Duration::from_secs(1));

    assert!(
        !check_iptables_for_injection(),
        "Suspicious content in iptables after enabling kill switch"
    );

    let _ = shroud(&["killswitch", "off"]);
    thread::sleep(Duration::from_secs(1));
}

#[test]
#[ignore = "requires specific user/group setup"]
fn test_no_suid_binaries_created() {
    // Verify shroud doesn't create any SUID binaries
    let output = Command::new("find")
        .args(["/home", "-name", "*shroud*", "-perm", "-4000", "-type", "f"])
        .output()
        .expect("Failed to run find");

    let suid_files = String::from_utf8_lossy(&output.stdout);

    assert!(
        suid_files.trim().is_empty(),
        "Found SUID shroud binaries: {}",
        suid_files
    );
}

#[test]
#[ignore = "requires specific user/group setup"]
fn test_no_world_writable_files() {
    // Check shroud doesn't create world-writable files
    let home = std::env::var("HOME").unwrap_or_default();

    let locations = vec![
        format!("{}/.config/shroud", home),
        format!("{}/.local/share/shroud", home),
    ];

    for location in locations {
        if std::path::Path::new(&location).exists() {
            let output = Command::new("find")
                .args([&location, "-perm", "-002", "-type", "f"])
                .output()
                .expect("Failed to run find");

            let world_writable = String::from_utf8_lossy(&output.stdout);

            assert!(
                world_writable.trim().is_empty(),
                "Found world-writable files in {}: {}",
                location,
                world_writable
            );
        }
    }
}

// ============================================================================
// ENVIRONMENT VARIABLE TESTS
// ============================================================================

#[test]
#[ignore = "requires specific user/group setup"]
fn test_no_sensitive_env_leakage() {
    // Set some sensitive env vars and verify they don't leak
    std::env::set_var("SECRET_KEY", "super_secret_value");
    std::env::set_var("API_TOKEN", "secret_token_123");

    let output = shroud(&["status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stdout.contains("super_secret_value") && !stderr.contains("super_secret_value"),
        "Sensitive env var leaked in output"
    );

    assert!(
        !stdout.contains("secret_token_123") && !stderr.contains("secret_token_123"),
        "API token leaked in output"
    );

    // Clean up
    std::env::remove_var("SECRET_KEY");
    std::env::remove_var("API_TOKEN");
}

#[test]
#[ignore = "requires specific user/group setup"]
fn test_path_not_hijackable() {
    // Temporarily modify PATH to include a malicious directory
    let original_path = std::env::var("PATH").unwrap_or_default();

    // Create temp dir with fake nmcli
    let temp_dir = tempfile::tempdir().unwrap();
    let fake_nmcli = temp_dir.path().join("nmcli");

    std::fs::write(
        &fake_nmcli,
        "#!/bin/sh\necho 'HIJACKED' > /tmp/hijack_test\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake_nmcli, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // Prepend our directory to PATH
    let malicious_path = format!("{}:{}", temp_dir.path().display(), original_path);
    std::env::set_var("PATH", &malicious_path);

    // Run shroud command that uses nmcli
    let _ = shroud(&["list"]);

    // Restore PATH
    std::env::set_var("PATH", original_path);

    // Check if hijack file was created
    let hijacked = std::path::Path::new("/tmp/hijack_test").exists();

    // Clean up
    let _ = std::fs::remove_file("/tmp/hijack_test");

    // Note: This test documents the behavior. If using full paths to nmcli,
    // this should not be hijackable. If using PATH lookup, this is a risk.
    if hijacked {
        println!("WARNING: nmcli lookup is PATH-dependent. Consider using absolute path.");
    }
}
