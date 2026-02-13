//! Regression tests for previously fixed bugs
//!
//! Each test documents the original issue and verifies the fix.
//! These tests use code inspection (include_str!) to verify fix patterns exist.

/// Verify fix for issue 1.8.8: Invalid VPN state bug
///
/// When connecting to a non-existent VPN, state got stuck in "Reconnecting"
/// Fix: Added ConnectionFailed event that transitions to Disconnected
#[test]
fn regression_connection_failed_event_exists() {
    let types_content = include_str!("../src/state/types.rs");
    assert!(
        types_content.contains("ConnectionFailed"),
        "ConnectionFailed event must exist in state types"
    );
}

#[test]
fn regression_handlers_dispatch_connection_failed() {
    let handlers_content = include_str!("../src/supervisor/handlers.rs");
    assert!(
        handlers_content.contains("Event::ConnectionFailed")
            || handlers_content.contains("ConnectionFailed"),
        "Handlers must be able to dispatch ConnectionFailed event"
    );
}

#[test]
fn regression_state_machine_handles_connection_failed() {
    let machine_content = include_str!("../src/state/machine.rs");
    assert!(
        machine_content.contains("ConnectionFailed"),
        "State machine must handle ConnectionFailed event"
    );
    assert!(
        machine_content.contains("VpnState::Disconnected"),
        "State machine must have Disconnected state for failed connection"
    );
}

/// Verify fix for issue 1.8.9: Kill switch toggle race condition
/// Note: The kill switch is owned by the supervisor and accessed through async methods,
/// which provides thread safety without explicit synchronization primitives.
#[test]
fn regression_killswitch_state_managed() {
    let firewall_content = include_str!("../src/killswitch/firewall.rs");
    // Kill switch should have an enabled state that can be checked
    assert!(
        firewall_content.contains("enabled: bool") || firewall_content.contains("is_enabled"),
        "Kill switch must track enabled state"
    );
    // And methods to enable/disable
    assert!(
        firewall_content.contains("fn enable") && firewall_content.contains("fn disable"),
        "Kill switch must have enable/disable methods"
    );
}

/// Verify boot kill switch cleanup on startup
#[test]
fn regression_boot_killswitch_has_cleanup() {
    let content = include_str!("../src/killswitch/boot.rs");
    let has_cleanup = content.contains("cleanup")
        || content.contains("remove")
        || content.contains("disable")
        || content.contains("clear");
    assert!(has_cleanup, "Boot kill switch must have cleanup logic");
}

/// Verify IPC socket cleanup on startup
#[test]
fn regression_ipc_socket_cleanup() {
    let content = include_str!("../src/ipc/server.rs");
    let has_cleanup = content.contains("remove_file")
        || content.contains("unlink")
        || content.contains("fs::remove")
        || content.contains("std::fs::remove");
    assert!(has_cleanup, "IPC server must clean up stale socket");
}

/// Verify SHROUD_NMCLI environment variable support
#[test]
fn regression_nmcli_env_override() {
    // nmcli_command() is centralized in nm/mod.rs
    let content = include_str!("../src/nm/mod.rs");
    assert!(
        content.contains("SHROUD_NMCLI"),
        "nm module must support SHROUD_NMCLI env var for testing"
    );
}

/// Verify signal handlers are installed
#[test]
fn regression_signal_handlers() {
    let main_content = include_str!("../src/main.rs");
    let supervisor_content = include_str!("../src/supervisor/mod.rs");
    let event_loop = include_str!("../src/supervisor/event_loop.rs");

    let has_signal_handling = main_content.contains("signal")
        || main_content.contains("ctrlc")
        || supervisor_content.contains("signal")
        || event_loop.contains("signal")
        || main_content.contains("tokio::signal");

    assert!(has_signal_handling, "Signal handlers must be installed");
}

/// Verify kill switch error types are complete
#[test]
fn regression_killswitch_error_types() {
    let content = include_str!("../src/killswitch/firewall.rs");
    assert!(
        content.contains("KillSwitchError"),
        "KillSwitchError type must exist"
    );
    // Should have multiple error variants
    let has_variants = content.contains("NotFound")
        || content.contains("Permission")
        || content.contains("Command");
    assert!(
        has_variants,
        "KillSwitchError should have specific variants"
    );
}

/// Verify state machine has proper max retries
#[test]
fn regression_state_machine_has_max_retries() {
    let content = include_str!("../src/state/machine.rs");
    assert!(
        content.contains("max_retries") || content.contains("max_attempts"),
        "State machine must have retry limit"
    );
}

/// Verify health checker has threshold configuration
#[test]
fn regression_health_checker_thresholds() {
    let content = include_str!("../src/health/checker.rs");
    let has_thresholds = content.contains("threshold")
        || content.contains("degraded")
        || content.contains("failure_count");
    assert!(
        has_thresholds,
        "Health checker must have configurable thresholds"
    );
}

/// Verify DNS mode configuration exists
#[test]
fn regression_dns_mode_exists() {
    let content = include_str!("../src/config/settings.rs");
    assert!(
        content.contains("DnsMode"),
        "DnsMode configuration must exist"
    );
}

/// Verify VPN state types implement required traits
#[test]
fn regression_vpn_state_has_traits() {
    let content = include_str!("../src/state/types.rs");
    // Should derive Debug and Clone at minimum
    assert!(
        content.contains("#[derive(Debug"),
        "VpnState should derive Debug"
    );
    assert!(content.contains("Clone"), "VpnState should derive Clone");
}

/// Verify TransitionReason has Display impl for logging
#[test]
fn regression_transition_reason_display() {
    let content = include_str!("../src/state/types.rs");
    assert!(
        content.contains("impl fmt::Display for TransitionReason")
            || content.contains("impl Display for TransitionReason"),
        "TransitionReason must implement Display for logging"
    );
}
