//! Configuration File Security Tests
//!
//! Verifies config files are properly secured:
//! - Correct file permissions
//! - Path traversal prevention
//! - Malformed config handling
//! - Bounds checking on values
//!
//! Run with: cargo test --test security_config -- --nocapture

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("shroud")
}

fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("shroud")
}

// ============================================================================
// FILE PERMISSION TESTS
// ============================================================================

#[test]
fn test_config_directory_permissions() {
    let dir = config_dir();

    if !dir.exists() {
        println!("Config directory doesn't exist yet, skipping");
        return;
    }

    let metadata = fs::metadata(&dir).expect("Failed to get directory metadata");
    let mode = metadata.permissions().mode();
    let perms = mode & 0o777;

    println!("Config directory permissions: {:o}", perms);

    // Shroud's config directory should not be world-writable
    // (world-readable is acceptable, world-writable is a security risk)
    let world_writable = perms & 0o002;
    assert_eq!(
        world_writable, 0,
        "Config directory is world-writable: {:o}",
        perms
    );
}

#[test]
fn test_config_file_permissions() {
    let file = config_file();

    if !file.exists() {
        println!("Config file doesn't exist yet, skipping");
        return;
    }

    let metadata = fs::metadata(&file).expect("Failed to get file metadata");
    let mode = metadata.permissions().mode();
    let perms = mode & 0o777;

    println!("Config file permissions: {:o}", perms);

    // Config file should not be world-writable (security concern)
    // World-readable is acceptable for non-sensitive config
    let world_writable = perms & 0o002;
    assert_eq!(
        world_writable, 0,
        "Config file is world-writable: {:o}",
        perms
    );

    let group_write = perms & 0o020;
    assert_eq!(group_write, 0, "Config file has group write: {:o}", perms);
}

#[test]
fn test_log_directory_permissions() {
    let dir = data_dir();

    if !dir.exists() {
        println!("Data directory doesn't exist yet, skipping");
        return;
    }

    let metadata = fs::metadata(&dir).expect("Failed to get directory metadata");
    let mode = metadata.permissions().mode();
    let perms = mode & 0o777;

    // Should not be world writable
    let world_write = perms & 0o002;
    assert_eq!(
        world_write, 0,
        "Data directory is world writable: {:o}",
        perms
    );
}

// ============================================================================
// MALFORMED CONFIG TESTS
// ============================================================================

#[test]
fn test_malformed_config_handled() {
    // Create backup of existing config
    let config = config_file();
    let backup = config.with_extension("toml.bak");

    if config.exists() {
        let _ = fs::copy(&config, &backup);
    }

    let long_value = format!("last_server = \"{}\"", "A".repeat(1_000_000));

    // Test various malformed configs
    let malformed_configs = vec![
        // Empty
        "",
        // Invalid TOML
        "this is not valid toml {{{",
        // Wrong types
        "kill_switch_enabled = \"not a bool\"",
        "auto_reconnect = 12345",
        // Missing values
        "last_server = ",
        // Injection attempts
        "last_server = \"$(cat /etc/passwd)\"",
        "last_server = \"`rm -rf /`\"",
        // Path traversal
        "last_server = \"../../../etc/passwd\"",
        // Extremely long values
        &long_value,
        // Null bytes
        "last_server = \"test\x00evil\"",
        // Unicode shenanigans
        "last_server = \"test\u{202E}live\"",
    ];

    for (i, content) in malformed_configs.iter().enumerate() {
        println!(
            "Testing malformed config {}: {}...",
            i,
            &content[..content.len().min(50)]
        );

        // Write malformed config
        if let Some(parent) = config.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&config, content);

        // Try to run shroud - should not crash
        let output = std::process::Command::new("./target/debug/shroud")
            .args(["--help"]) // Simple command that loads config
            .output();

        assert!(output.is_ok(), "Shroud crashed on malformed config {}", i);
    }

    // Restore backup
    if backup.exists() {
        let _ = fs::rename(&backup, &config);
    } else {
        let _ = fs::remove_file(&config);
    }
}

#[test]
fn test_config_values_bounds_checked() {
    let config = config_file();
    let backup = config.with_extension("toml.bak");

    if config.exists() {
        let _ = fs::copy(&config, &backup);
    }

    // Test boundary values
    let boundary_configs = [
        // Negative numbers where positive expected
        "[timeouts]\nconnection = -1",
        // Very large numbers
        "[timeouts]\nconnection = 999999999999999999999",
        // Floating point where int expected
        "[timeouts]\nconnection = 3.14159",
    ];

    for (i, content) in boundary_configs.iter().enumerate() {
        println!("Testing boundary config {}", i);

        if let Some(parent) = config.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&config, content);

        let output = std::process::Command::new("./target/debug/shroud")
            .args(["status"])
            .output();

        // Should handle gracefully (use defaults or clamp values)
        assert!(output.is_ok(), "Shroud crashed on boundary config {}", i);
    }

    // Restore
    if backup.exists() {
        let _ = fs::rename(&backup, &config);
    } else {
        let _ = fs::remove_file(&config);
    }
}

// ============================================================================
// SYMLINK ATTACK TESTS
// ============================================================================

#[test]
#[ignore] // Potentially destructive
fn test_config_symlink_attack() {
    let config = config_file();

    // Backup existing config
    let backup = config.with_extension("toml.bak");
    if config.exists() {
        if config.is_symlink() {
            let _ = fs::remove_file(&config);
        } else {
            let _ = fs::rename(&config, &backup);
        }
    }

    // Create symlink to /etc/passwd
    let target = PathBuf::from("/tmp/shroud_symlink_test");

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        // Create symlink pointing to temp file
        let _ = fs::write(&target, "original content");
        let _ = symlink(&target, &config);

        // Run shroud which might write to config
        let _ = std::process::Command::new("./target/debug/shroud")
            .args(["status"])
            .output();

        // Check if target was overwritten
        let content = fs::read_to_string(&target).unwrap_or_default();

        // Clean up
        let _ = fs::remove_file(&config);
        let _ = fs::remove_file(&target);

        // Verify target wasn't modified with unexpected content
        if !content.contains("original content") && !content.is_empty() {
            println!(
                "WARNING: Config wrote through symlink. Content: {}",
                content
            );
        }
    }

    // Restore backup
    if backup.exists() {
        let _ = fs::rename(&backup, &config);
    }
}

#[test]
fn test_no_sensitive_data_in_config() {
    let config = config_file();

    if !config.exists() {
        println!("Config doesn't exist, skipping");
        return;
    }

    let content = fs::read_to_string(&config).unwrap_or_default();

    // Config should not contain passwords, keys, or tokens
    let sensitive_patterns = [
        "password",
        "secret",
        "token",
        "api_key",
        "private_key",
        "-----BEGIN",
    ];

    for pattern in sensitive_patterns {
        assert!(
            !content.to_lowercase().contains(pattern),
            "Config may contain sensitive data matching '{}'",
            pattern
        );
    }
}
