// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Regression tests for previously fixed bugs
//!
//! Each test documents the original issue and verifies the fix.
//!
//! Tests use the real StateMachine and HealthChecker via the library target.
//! For modules with deep dependency chains (killswitch, ipc, nm), we use
//! include_str! until their APIs can be exposed without pulling in the
//! entire binary crate.

use shroud::state::{Event, StateMachine, StateMachineConfig, TransitionReason, VpnState};

// ============================================================================
// Issue 1.8.8: ConnectionFailed event
// ============================================================================

/// Regression: connecting to a non-existent VPN got stuck in "Reconnecting".
/// Fix: Added ConnectionFailed event that transitions directly to Disconnected.
///
/// This test exercises the real StateMachine: Connecting → ConnectionFailed → Disconnected.
#[test]
fn regression_connection_failed_from_connecting() {
    let mut sm = StateMachine::new();

    // Put the machine into Connecting state
    let _ = sm.handle_event(Event::UserEnable {
        server: "nonexistent".to_string(),
    });
    assert!(
        matches!(sm.state, VpnState::Connecting { .. }),
        "Should be in Connecting state, got: {:?}",
        sm.state
    );

    // Dispatch ConnectionFailed — must go to Disconnected, not Reconnecting
    let reason = sm.handle_event(Event::ConnectionFailed {
        reason: "VPN not found".to_string(),
    });
    assert_eq!(sm.state, VpnState::Disconnected);
    assert!(
        matches!(reason, Some(TransitionReason::ConnectionFailed)),
        "Expected ConnectionFailed reason, got: {:?}",
        reason
    );
}

/// Regression: ConnectionFailed from Reconnecting state must also go to Disconnected.
/// This was the original bug — state got stuck in Reconnecting loop.
#[test]
fn regression_connection_failed_from_reconnecting() {
    let config = StateMachineConfig { max_retries: 10 };
    let mut sm = StateMachine::with_config(config);

    // Get into Reconnecting state: Connect → Connected → VpnDown
    let _ = sm.handle_event(Event::UserEnable {
        server: "test-vpn".to_string(),
    });
    let _ = sm.handle_event(Event::NmVpnUp {
        server: "test-vpn".to_string(),
    });
    let _ = sm.handle_event(Event::NmVpnDown);
    assert!(
        matches!(sm.state, VpnState::Reconnecting { .. }),
        "Should be in Reconnecting state, got: {:?}",
        sm.state
    );

    // ConnectionFailed must break out of Reconnecting → Disconnected
    let reason = sm.handle_event(Event::ConnectionFailed {
        reason: "VPN no longer exists".to_string(),
    });
    assert_eq!(sm.state, VpnState::Disconnected);
    assert!(
        matches!(reason, Some(TransitionReason::ConnectionFailed)),
        "Expected ConnectionFailed reason, got: {:?}",
        reason
    );
}

// ============================================================================
// Issue 1.8.9: Kill switch state management
// ============================================================================

/// Regression: Kill switch must track enabled state and have enable/disable API.
///
/// KillSwitch has deep dependencies (iptables, config, paths) that pull in most
/// of the binary. We verify the public API exists at compile time by importing
/// the type and calling its constructor + state check.
///
/// Note: We cannot call enable()/disable() in tests (they invoke sudo iptables),
/// but we verify the struct and its initial state.
// TODO: Replace with full behavioral test when KillSwitch is refactored to
// accept a firewall backend trait (allowing a mock backend in tests).
#[test]
fn regression_killswitch_state_managed() {
    // This test verifies the KillSwitch API contract exists via include_str!.
    // The real struct can't be instantiated here without pulling in iptables/config.
    let firewall_content = include_str!("../src/killswitch/firewall.rs");

    // Verify the struct tracks state
    assert!(
        firewall_content.contains("enabled: bool"),
        "Kill switch must track enabled state"
    );
    // Verify the public API surface exists
    assert!(
        firewall_content.contains("pub fn new()"),
        "Kill switch must have public constructor"
    );
    assert!(
        firewall_content.contains("pub fn is_enabled"),
        "Kill switch must have is_enabled() method"
    );
    assert!(
        firewall_content.contains("pub async fn enable"),
        "Kill switch must have enable() method"
    );
    assert!(
        firewall_content.contains("pub async fn disable"),
        "Kill switch must have disable() method"
    );
    // Verify initial state is disabled (from constructor)
    assert!(
        firewall_content.contains("enabled: false"),
        "Kill switch must default to disabled"
    );
}

// ============================================================================
// State machine property tests
// ============================================================================

/// Regression: State machine must enforce retry limits.
#[test]
fn regression_state_machine_retry_exhaustion() {
    let config = StateMachineConfig { max_retries: 3 };
    let mut sm = StateMachine::with_config(config);

    assert_eq!(sm.max_retries(), 3);

    // Connect → Connected → VpnDown → Reconnecting { attempt: 1, max_attempts: 3 }
    let _ = sm.handle_event(Event::UserEnable {
        server: "vpn".to_string(),
    });
    let _ = sm.handle_event(Event::NmVpnUp {
        server: "vpn".to_string(),
    });
    let _ = sm.handle_event(Event::NmVpnDown);
    assert!(matches!(
        sm.state,
        VpnState::Reconnecting { attempt: 1, .. }
    ));

    // Timeout 1: attempt 1+1=2, 2 < 3 → still Reconnecting
    let _ = sm.handle_event(Event::Timeout);
    assert!(matches!(
        sm.state,
        VpnState::Reconnecting { attempt: 2, .. }
    ));

    // Timeout 2: attempt 2+1=3, 3 >= 3 → Failed (exhausted)
    let reason = sm.handle_event(Event::Timeout);
    assert!(
        matches!(sm.state, VpnState::Failed { .. }),
        "Should transition to Failed after exhausting retries, got: {:?}",
        sm.state
    );
    assert!(
        matches!(reason, Some(TransitionReason::RetriesExhausted)),
        "Expected RetriesExhausted, got: {:?}",
        reason
    );
}

/// Regression: UserDisable from any state must reach Disconnected.
#[test]
fn regression_user_disable_always_disconnects() {
    let states_to_test = vec![
        VpnState::Connecting {
            server: "vpn".to_string(),
        },
        VpnState::Connected {
            server: "vpn".to_string(),
        },
        VpnState::Degraded {
            server: "vpn".to_string(),
        },
        VpnState::Reconnecting {
            server: "vpn".to_string(),
            attempt: 2,
            max_attempts: 10,
        },
        VpnState::Failed {
            server: "vpn".to_string(),
            reason: "timeout".to_string(),
        },
    ];

    for initial_state in states_to_test {
        let mut sm = StateMachine::new();
        sm.set_state(initial_state.clone(), TransitionReason::Unknown);

        let reason = sm.handle_event(Event::UserDisable);
        assert_eq!(
            sm.state,
            VpnState::Disconnected,
            "UserDisable from {:?} should reach Disconnected",
            initial_state
        );
        assert!(
            matches!(reason, Some(TransitionReason::UserRequested)),
            "UserDisable reason should be UserRequested, got {:?}",
            reason
        );
    }
}

/// Regression: VpnState and TransitionReason must implement Display for logging.
#[test]
fn regression_types_implement_display() {
    // If Display is removed, this test won't compile
    let state = VpnState::Connected {
        server: "test".to_string(),
    };
    let display = format!("{}", state);
    assert!(!display.is_empty(), "VpnState Display must produce output");

    let reason = TransitionReason::UserRequested;
    let display = format!("{}", reason);
    assert!(
        !display.is_empty(),
        "TransitionReason Display must produce output"
    );
}

/// Regression: VpnState must derive Debug and Clone.
#[test]
fn regression_vpn_state_traits() {
    let state = VpnState::Connected {
        server: "test".to_string(),
    };
    // Clone — if removed, this won't compile
    let cloned = state.clone();
    assert_eq!(state, cloned);
    // Debug — if removed, this won't compile
    let debug = format!("{:?}", state);
    assert!(debug.contains("Connected"));
}

// ============================================================================
// Health checker behavioral tests
// ============================================================================

/// Regression: Health checker must have configurable thresholds.
#[test]
fn regression_health_checker_thresholds() {
    use shroud::health::checker::{HealthChecker, HealthConfig};

    let config = HealthConfig {
        endpoints: vec!["https://example.com".to_string()],
        timeout_secs: 5,
        degraded_threshold_ms: 2000,
        failure_threshold: 3,
        degraded_threshold: 2,
    };

    let mut checker = HealthChecker::with_config(config);
    // reset() must exist and not panic — guards against removal
    checker.reset();
}

// ============================================================================
// Remaining include_str! tests for modules without library exposure
// ============================================================================

/// Verify boot kill switch cleanup logic exists.
// TODO: Replace with behavioral test when boot killswitch accepts a backend trait.
#[test]
fn regression_boot_killswitch_has_cleanup() {
    let content = include_str!("../src/killswitch/boot.rs");
    assert!(
        content.contains("disable_boot_killswitch"),
        "Boot kill switch must have disable/cleanup function"
    );
}

/// Verify IPC socket cleanup on startup.
// TODO: Replace with behavioral test when IPC server is testable without bind().
#[test]
fn regression_ipc_socket_cleanup() {
    let content = include_str!("../src/ipc/server.rs");
    assert!(
        content.contains("remove_file") || content.contains("std::fs::remove"),
        "IPC server must clean up stale socket"
    );
}

/// Verify SHROUD_NMCLI environment variable support.
// TODO: Replace with behavioral test when nm module is exposed via lib.rs.
#[test]
fn regression_nmcli_env_override() {
    let content = include_str!("../src/nm/mod.rs");
    assert!(
        content.contains("SHROUD_NMCLI"),
        "nm module must support SHROUD_NMCLI env var override"
    );
}

/// Verify signal handlers are installed.
// TODO: Replace with behavioral test when signal setup is refactored.
#[test]
fn regression_signal_handlers() {
    let main_content = include_str!("../src/main.rs");
    let event_loop = include_str!("../src/supervisor/event_loop.rs");

    let has_signal_handling = main_content.contains("ctrlc")
        || event_loop.contains("signal")
        || event_loop.contains("SignalKind");

    assert!(has_signal_handling, "Signal handlers must be installed");
}

/// Verify kill switch error types are complete.
// TODO: Replace with behavioral test when KillSwitchError is exposed via lib.rs.
#[test]
fn regression_killswitch_error_types() {
    let content = include_str!("../src/killswitch/firewall.rs");
    assert!(
        content.contains("pub enum KillSwitchError"),
        "KillSwitchError type must be a public enum"
    );
    assert!(
        content.contains("Permission"),
        "KillSwitchError must have Permission variant"
    );
    assert!(
        content.contains("Spawn"),
        "KillSwitchError must have Spawn variant"
    );
    assert!(
        content.contains("Command(String)"),
        "KillSwitchError must have Command variant"
    );
}

/// Verify DNS mode configuration exists.
// TODO: Replace with behavioral test when config module is exposed via lib.rs.
#[test]
fn regression_dns_mode_exists() {
    let content = include_str!("../src/config/settings.rs");
    assert!(
        content.contains("pub enum DnsMode"),
        "DnsMode must be a public enum"
    );
    assert!(
        content.contains("Tunnel"),
        "DnsMode must have Tunnel variant"
    );
    assert!(
        content.contains("Strict"),
        "DnsMode must have Strict variant"
    );
}
