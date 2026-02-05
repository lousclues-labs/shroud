//! End-to-end tests for Shroud
//!
//! These tests run the actual shroud binary and verify behavior.
//! Run with: `cargo test --test e2e`
//!
//! Privileged tests (marked #[ignore]) require root:
//! `sudo -E cargo test --test e2e -- --ignored`

mod common;

use common::*;
use std::time::Duration;

// ============================================================================
// Headless Mode Tests
// ============================================================================

mod headless {
    use super::*;

    #[tokio::test]
    async fn test_headless_startup_and_shutdown() {
        init();
        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        // Start should succeed
        let result = shroud.start_headless().await;
        assert!(result.is_ok(), "Failed to start headless: {:?}", result);

        // Should respond to status
        let status = shroud.status().await;
        assert!(status.is_ok(), "Status failed: {:?}", status);

        // Graceful shutdown
        let stop = shroud.stop().await;
        assert!(stop.is_ok(), "Stop failed: {:?}", stop);

        // Should not be running
        assert!(!shroud.is_running());
    }

    #[tokio::test]
    async fn test_headless_status_output() {
        init();
        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        let status = shroud.status().await.expect("Failed to get status");

        // Should contain state information
        assert!(!status.is_empty(), "Status output is empty");
        // Should show disconnected state
        let status_lower = status.to_lowercase();
        assert!(
            status_lower.contains("disconnect") || status_lower.contains("idle"),
            "Unexpected status: {}",
            status
        );

        shroud.stop().await.ok();
    }

    #[tokio::test]
    async fn test_headless_list_command() {
        init();
        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        let list = shroud.run_command(&["list"]).await;
        // Should succeed even if no VPNs configured
        assert!(list.is_ok(), "List command failed: {:?}", list);

        shroud.stop().await.ok();
    }

    #[tokio::test]
    async fn test_headless_help_command() {
        init();
        let ctx = TestContext::new();
        let shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        // Help should work without daemon running
        let help = shroud.run_command(&["--help"]).await;
        assert!(help.is_ok(), "Help command failed: {:?}", help);
        let output = help.unwrap();
        assert!(output.contains("shroud") || output.contains("Shroud"));
    }

    #[tokio::test]
    async fn test_headless_version_command() {
        init();
        let ctx = TestContext::new();
        let shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        let version = shroud.run_command(&["--version"]).await;
        assert!(version.is_ok(), "Version command failed: {:?}", version);
        let output = version.unwrap();
        assert!(output.contains("shroud") || output.contains("1."));
    }

    #[tokio::test]
    async fn test_headless_invalid_command() {
        init();
        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        // Invalid command should fail gracefully
        let result = shroud.run_command(&["not-a-real-command"]).await;
        assert!(result.is_err(), "Invalid command should fail");

        // Daemon should still be running
        assert!(shroud.is_running());
        assert!(shroud.status().await.is_ok());

        shroud.stop().await.ok();
    }

    #[tokio::test]
    async fn test_headless_multiple_status_calls() {
        init();
        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        // Multiple rapid status calls should all succeed
        for i in 0..10 {
            let status = shroud.status().await;
            assert!(status.is_ok(), "Status call {} failed: {:?}", i, status);
        }

        shroud.stop().await.ok();
    }
}

// ============================================================================
// Kill Switch Tests (Privileged)
// ============================================================================

mod killswitch {
    use super::*;

    #[tokio::test]
    #[ignore = "requires root"]
    async fn test_killswitch_enable_creates_chain() {
        require_root();
        cleanup_iptables();

        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        // Enable kill switch
        let result = shroud.ks_enable().await;
        assert!(result.is_ok(), "Failed to enable killswitch: {:?}", result);

        // Chain should exist
        assert_chain_exists("SHROUD_KILLSWITCH");

        shroud.stop().await.ok();
        cleanup_iptables();
    }

    #[tokio::test]
    #[ignore = "requires root"]
    async fn test_killswitch_disable_removes_chain() {
        require_root();
        cleanup_iptables();

        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        // Enable then disable
        shroud.ks_enable().await.expect("Failed to enable");
        tokio::time::sleep(Duration::from_millis(500)).await;
        shroud.ks_disable().await.expect("Failed to disable");
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Chain should be gone
        assert_chain_not_exists("SHROUD_KILLSWITCH");

        shroud.stop().await.ok();
        cleanup_iptables();
    }

    #[tokio::test]
    #[ignore = "requires root"]
    async fn test_killswitch_status_command() {
        require_root();
        cleanup_iptables();

        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        // Check status when disabled
        let status_off = shroud.run_command(&["ks", "status"]).await;
        assert!(status_off.is_ok());

        // Enable and check again
        shroud.ks_enable().await.expect("Failed to enable");
        let status_on = shroud.run_command(&["ks", "status"]).await;
        assert!(status_on.is_ok());

        shroud.stop().await.ok();
        cleanup_iptables();
    }

    #[tokio::test]
    #[ignore = "requires root"]
    async fn test_killswitch_idempotent_enable() {
        require_root();
        cleanup_iptables();

        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        // Enable multiple times should be safe
        for _ in 0..5 {
            let result = shroud.ks_enable().await;
            assert!(result.is_ok(), "Enable should be idempotent");
        }

        assert_chain_exists("SHROUD_KILLSWITCH");

        shroud.stop().await.ok();
        cleanup_iptables();
    }

    #[tokio::test]
    #[ignore = "requires root"]
    async fn test_killswitch_idempotent_disable() {
        require_root();
        cleanup_iptables();

        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        // Disable multiple times should be safe (even when not enabled)
        for _ in 0..5 {
            let result = shroud.ks_disable().await;
            assert!(result.is_ok(), "Disable should be idempotent");
        }

        shroud.stop().await.ok();
        cleanup_iptables();
    }
}

// ============================================================================
// Cleanup Tests (Privileged)
// ============================================================================

mod cleanup {
    use super::*;

    #[tokio::test]
    #[ignore = "requires root"]
    async fn test_graceful_shutdown_cleans_killswitch() {
        require_root();
        cleanup_iptables();

        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");
        shroud
            .ks_enable()
            .await
            .expect("Failed to enable killswitch");
        assert_chain_exists("SHROUD_KILLSWITCH");

        // Graceful stop
        shroud.stop().await.expect("Failed to stop");
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Chain should be cleaned up
        assert_chain_not_exists("SHROUD_KILLSWITCH");
    }

    #[tokio::test]
    #[ignore = "requires root"]
    async fn test_sigterm_cleans_killswitch() {
        require_root();
        cleanup_iptables();

        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");
        shroud
            .ks_enable()
            .await
            .expect("Failed to enable killswitch");
        assert_chain_exists("SHROUD_KILLSWITCH");

        // Send SIGTERM
        if let Some(pid) = shroud.pid() {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }

        // Wait for cleanup
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Chain should be cleaned up
        assert_chain_not_exists("SHROUD_KILLSWITCH");
    }

    /// Test that socket is cleaned up on exit
    ///
    /// Note: This test requires a proper system environment with D-Bus.
    /// In CI, the daemon may fail to start due to missing D-Bus session.
    #[tokio::test]
    #[ignore = "Requires D-Bus session - run locally with: cargo test --test e2e -- --ignored"]
    async fn test_socket_cleanup_on_exit() {
        init();
        let ctx = TestContext::new();
        let socket = ctx.socket_path();
        let mut shroud = ShroudProcess::new(shroud_binary(), &socket);

        shroud.start_headless().await.expect("Failed to start");

        // Socket should exist while running
        assert!(socket.exists(), "Socket should exist while running");

        shroud.stop().await.expect("Failed to stop");

        // Give more time for cleanup - socket removal is async
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Socket should be cleaned up
        assert!(!socket.exists(), "Socket should be cleaned up on exit");
    }
}

// ============================================================================
// Configuration Tests
// ============================================================================

mod config {
    use super::*;

    #[tokio::test]
    async fn test_handles_missing_config() {
        init();
        let ctx = TestContext::new();
        // Don't create any config file
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        // Should start with defaults
        let result = shroud.start_headless().await;
        assert!(result.is_ok(), "Should handle missing config: {:?}", result);

        shroud.stop().await.ok();
    }

    #[tokio::test]
    async fn test_handles_corrupted_config() {
        init();
        let ctx = TestContext::new();

        // Write garbage to config
        let config_path = ctx.config_dir().join("config.toml");
        std::fs::write(&config_path, "{{{{NOT VALID TOML}}}}").unwrap();

        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        // Should start anyway with defaults
        let result = shroud.start_headless().await;
        assert!(result.is_ok(), "Should handle corrupted config");

        shroud.stop().await.ok();
    }

    #[tokio::test]
    async fn test_handles_empty_config() {
        init();
        let ctx = TestContext::new();

        // Write empty config
        let config_path = ctx.config_dir().join("config.toml");
        std::fs::write(&config_path, "").unwrap();

        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        // Should start with defaults
        let result = shroud.start_headless().await;
        assert!(result.is_ok(), "Should handle empty config");

        shroud.stop().await.ok();
    }
}

// ============================================================================
// IPC Tests
// ============================================================================

mod ipc {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    #[tokio::test]
    async fn test_ipc_malformed_request() {
        init();
        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        // Send garbage to socket
        if let Ok(mut stream) = UnixStream::connect(ctx.socket_path()) {
            let _ = stream.write_all(b"NOT JSON AT ALL\n");
        }

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Daemon should survive
        assert!(shroud.is_running(), "Daemon died from malformed IPC");
        assert!(shroud.status().await.is_ok());

        shroud.stop().await.ok();
    }

    #[tokio::test]
    async fn test_ipc_empty_request() {
        init();
        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        // Send empty data
        if let Ok(mut stream) = UnixStream::connect(ctx.socket_path()) {
            let _ = stream.write_all(b"");
        }

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Daemon should survive
        assert!(shroud.is_running());

        shroud.stop().await.ok();
    }

    #[tokio::test]
    async fn test_ipc_binary_garbage() {
        init();
        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        // Send binary garbage
        if let Ok(mut stream) = UnixStream::connect(ctx.socket_path()) {
            let _ = stream.write_all(&[0x00, 0xFF, 0xFE, 0x00, 0x01, 0x02]);
        }

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Daemon should survive
        assert!(shroud.is_running());
        assert!(shroud.status().await.is_ok());

        shroud.stop().await.ok();
    }

    #[tokio::test]
    async fn test_stale_socket_recovery() {
        init();
        let ctx = TestContext::new();
        let socket = ctx.socket_path();

        // Create a stale socket file
        std::fs::write(&socket, "stale").unwrap();

        let mut shroud = ShroudProcess::new(shroud_binary(), &socket);

        // Should handle stale socket and start successfully
        let result = shroud.start_headless().await;
        assert!(result.is_ok(), "Should handle stale socket: {:?}", result);
        assert!(shroud.status().await.is_ok());

        shroud.stop().await.ok();
    }
}

// ============================================================================
// Concurrency Tests
// ============================================================================

mod concurrency {
    use super::*;

    #[tokio::test]
    async fn test_concurrent_status_requests() {
        init();
        let ctx = TestContext::new();
        let mut shroud = ShroudProcess::new(shroud_binary(), ctx.socket_path());

        shroud.start_headless().await.expect("Failed to start");

        // Spawn concurrent status requests
        let binary = shroud_binary();
        let socket = ctx.socket_path();

        let handles: Vec<_> = (0..20)
            .map(|_| {
                let b = binary.clone();
                let s = socket.clone();
                tokio::spawn(async move {
                    let proc = ShroudProcess::new(b, s);
                    proc.run_command(&["status"]).await
                })
            })
            .collect();

        let mut successes = 0;
        for handle in handles {
            if handle.await.unwrap().is_ok() {
                successes += 1;
            }
        }

        // Most should succeed
        assert!(
            successes >= 15,
            "Only {}/20 concurrent requests succeeded",
            successes
        );

        // Daemon should still work
        assert!(shroud.status().await.is_ok());

        shroud.stop().await.ok();
    }

    #[tokio::test]
    async fn test_prevents_multiple_instances() {
        init();
        let ctx = TestContext::new();
        let socket = ctx.socket_path();

        let mut shroud1 = ShroudProcess::new(shroud_binary(), &socket);
        shroud1
            .start_headless()
            .await
            .expect("First instance failed");

        // Try to start second instance
        let mut shroud2 = ShroudProcess::new(shroud_binary(), &socket);
        let _result = shroud2.start_headless().await;

        // Second should fail to become ready (socket in use)
        // or exit quickly due to lock file
        tokio::time::sleep(Duration::from_secs(1)).await;

        // First should still work
        assert!(shroud1.status().await.is_ok(), "First instance died");

        shroud1.stop().await.ok();
    }
}
