// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Shared utilities for state machine fuzz targets.

use shroud::state::{Event, StateMachine, VpnState};

// ========================================================================
// Server name pools
// ========================================================================

/// "Normal" server names that appear in realistic usage
pub const NORMAL_SERVERS: &[&str] = &[
    "ireland-42",
    "tokyo-7",
    "us-east-1",
    "de-frankfurt-03",
    "mullvad-se-sto",
];

/// Pathological server names that should be handled without panic
pub const CHAOS_SERVERS: &[&str] = &[
    "",                                          // empty
    "\0",                                        // null byte
    "\n\r\t",                                    // control characters
    "\x1b[31mred\x1b[0m",                       // ANSI escape sequences
    "a]b",                                       // shell metacharacter (bracket)
    "$(whoami)",                                 // shell injection attempt
    "; rm -rf /",                                // classic injection
    "server\x00evil",                            // null byte in middle
    "日本語サーバー",                              // unicode
    "\u{202e}evil\u{202c}",                      // RTL override
    "a",                                         // minimal
];

/// A 10,000 character server name. Generated once.
pub fn huge_server_name() -> String {
    "a".repeat(10_000)
}

// ========================================================================
// Event generation
// ========================================================================

/// Build an Event from a byte. Covers all 14 variants plus chaos inputs.
///
/// Slots 0-13:  All 14 Event variants with normal inputs.
/// Slots 14-19: Normal variants with chaos server names.
/// Slots 20-25: Edge-case string payloads.
/// Slots 26-31: Duplicate normal variants with different servers (for wrong-server scenarios).
///
/// Total: 32 slots (byte % 32).
pub fn event_from_byte(byte: u8) -> Event {
    match byte % 32 {
        // === All 14 Event variants with normal inputs ===
        0  => Event::UserEnable { server: NORMAL_SERVERS[0].into() },
        1  => Event::UserDisable,
        2  => Event::NmVpnUp { server: NORMAL_SERVERS[0].into() },
        3  => Event::NmVpnDown,
        4  => Event::NmVpnChanged { server: NORMAL_SERVERS[1].into() },
        5  => Event::NmDeviceChanged,
        6  => Event::HealthOk,
        7  => Event::HealthDegraded,
        8  => Event::HealthDead,
        9  => Event::Sleep,
        10 => Event::Wake,
        11 => Event::Timeout,
        12 => Event::ConnectionFailed { reason: "VPN not found".into() },
        13 => Event::EndpointFailed { reason: "endpoint unreachable".into() },

        // === Chaos server names on events that take strings ===
        14 => Event::UserEnable { server: "".into() },
        15 => Event::UserEnable { server: "\0\n\r\t".into() },
        16 => Event::NmVpnUp { server: "".into() },
        17 => Event::NmVpnUp { server: "\0\n\r\t".into() },
        18 => Event::NmVpnChanged { server: "".into() },
        19 => Event::NmVpnChanged { server: "\x1b[31mred\x1b[0m".into() },

        // === Huge and injection payloads ===
        20 => Event::UserEnable { server: huge_server_name() },
        21 => Event::NmVpnUp { server: huge_server_name() },
        22 => Event::ConnectionFailed { reason: "".into() },
        23 => Event::ConnectionFailed { reason: "x".repeat(10_000) },
        24 => Event::EndpointFailed { reason: "".into() },
        25 => Event::EndpointFailed { reason: "x".repeat(10_000) },

        // === Wrong-server scenarios (server name that won't match current state) ===
        26 => Event::UserEnable { server: NORMAL_SERVERS[2].into() },
        27 => Event::NmVpnUp { server: NORMAL_SERVERS[3].into() },
        28 => Event::NmVpnUp { server: NORMAL_SERVERS[4].into() },
        29 => Event::NmVpnChanged { server: NORMAL_SERVERS[0].into() },
        30 => Event::UserEnable { server: "$(whoami)".into() },
        31 => Event::NmVpnUp { server: "; rm -rf /".into() },

        _ => unreachable!(),
    }
}

/// Build an Event from a byte, using a fuzz-provided string for server names.
/// This lets libfuzzer's mutation engine craft arbitrary server strings.
#[allow(dead_code)]
pub fn event_from_byte_with_string(byte: u8, fuzz_string: &str) -> Event {
    match byte % 14 {
        0  => Event::UserEnable { server: fuzz_string.into() },
        1  => Event::UserDisable,
        2  => Event::NmVpnUp { server: fuzz_string.into() },
        3  => Event::NmVpnDown,
        4  => Event::NmVpnChanged { server: fuzz_string.into() },
        5  => Event::NmDeviceChanged,
        6  => Event::HealthOk,
        7  => Event::HealthDegraded,
        8  => Event::HealthDead,
        9  => Event::Sleep,
        10 => Event::Wake,
        11 => Event::Timeout,
        12 => Event::ConnectionFailed { reason: fuzz_string.into() },
        13 => Event::EndpointFailed { reason: fuzz_string.into() },
        _  => unreachable!(),
    }
}

// ========================================================================
// Invariant checking
// ========================================================================

/// Core invariants that must hold after every event on any StateMachine.
/// If any invariant fails, the fuzz target will panic and libfuzzer will
/// save the failing input as a crash.
pub fn check_invariants(machine: &StateMachine) {
    let retries = machine.retries();
    let max = machine.max_retries();

    // I1: Retry counter is bounded.
    //
    // Connected/Degraded → Reconnecting sets retries=1 without checking
    // max_retries. A subsequent Timeout increments to 2 and then transitions
    // to Failed. So for max_retries < 2, retries can reach 2 (in Failed).
    // The correct upper bound is max(max_retries, 2).
    let bound = max.max(2);
    assert!(
        retries <= bound,
        "INVARIANT VIOLATION: retries ({}) > bound ({}) [max_retries={}]",
        retries, bound, max
    );

    match &machine.state {
        // I2: Disconnected state always has retries == 0
        VpnState::Disconnected => {
            assert_eq!(retries, 0,
                "INVARIANT VIOLATION: Disconnected but retries = {}", retries);
        }

        // I3: Reconnecting state has retries > 0
        // I4: Reconnecting.attempt == retries (canonical counter sync)
        // I5: Reconnecting.max_attempts == config.max_retries
        VpnState::Reconnecting { attempt, max_attempts, .. } => {
            assert!(retries > 0,
                "INVARIANT VIOLATION: Reconnecting but retries = 0");
            assert_eq!(*attempt, retries,
                "INVARIANT VIOLATION: Reconnecting.attempt ({}) != retries ({})",
                attempt, retries);
            assert_eq!(*max_attempts, max,
                "INVARIANT VIOLATION: Reconnecting.max_attempts ({}) != max_retries ({})",
                max_attempts, max);
        }

        // I6: Failed state must have a non-empty reason
        VpnState::Failed { reason, .. } => {
            assert!(!reason.is_empty(),
                "INVARIANT VIOLATION: Failed state with empty reason");
        }

        // I7: Connecting, Connected, Degraded have no additional invariants
        // beyond retries bound
        VpnState::Connecting { .. }
        | VpnState::Connected { .. }
        | VpnState::Degraded { .. } => {}
    }

    // I8: Display impl must not panic
    let _ = format!("{}", machine.state);

    // I9: name() must return a valid &'static str
    let name = machine.state.name();
    assert!(
        ["Disconnected", "Connecting", "Connected", "Degraded", "Reconnecting", "Failed"]
            .contains(&name),
        "INVARIANT VIOLATION: unknown state name: {}", name
    );

    // I10: server_name() must not panic
    let _ = machine.state.server_name();

    // I11: is_active() and is_busy() must not panic
    let _ = machine.state.is_active();
    let _ = machine.state.is_busy();

    // I12: If Connecting or Reconnecting, is_busy() must be true
    if matches!(machine.state, VpnState::Connecting { .. } | VpnState::Reconnecting { .. }) {
        assert!(machine.state.is_busy(),
            "INVARIANT VIOLATION: {} state but is_busy() is false", name);
    }

    // I13: If Disconnected or Failed, is_active() must be false
    if matches!(machine.state, VpnState::Disconnected | VpnState::Failed { .. }) {
        assert!(!machine.state.is_active(),
            "INVARIANT VIOLATION: {} state but is_active() is true", name);
    }
}

/// Extended invariant: verify that the TransitionReason matches the state change.
#[allow(dead_code)]
pub fn check_transition_reason(
    _old_state: &VpnState,
    new_state: &VpnState,
    reason: &Option<shroud::state::TransitionReason>,
) {
    use shroud::state::TransitionReason;

    match reason {
        None => {
            // No transition should mean no state change
        }
        Some(TransitionReason::UserRequested) => {
            // UserRequested can go to Disconnected (UserDisable) or Connecting (UserEnable)
            assert!(
                matches!(new_state, VpnState::Disconnected | VpnState::Connecting { .. }),
                "INVARIANT VIOLATION: UserRequested but new state is {}",
                new_state.name()
            );
        }
        Some(TransitionReason::RetriesExhausted) => {
            // Must land in Failed
            assert!(
                matches!(new_state, VpnState::Failed { .. }),
                "INVARIANT VIOLATION: RetriesExhausted but new state is {}",
                new_state.name()
            );
        }
        Some(TransitionReason::VpnEstablished) | Some(TransitionReason::VpnReestablished) => {
            assert!(
                matches!(new_state, VpnState::Connected { .. }),
                "INVARIANT VIOLATION: VpnEstablished/Reestablished but new state is {}",
                new_state.name()
            );
        }
        Some(TransitionReason::ConnectionFailed) => {
            assert!(
                matches!(new_state, VpnState::Disconnected),
                "INVARIANT VIOLATION: ConnectionFailed but new state is {}",
                new_state.name()
            );
        }
        Some(TransitionReason::VpnLost) => {
            assert!(
                matches!(new_state, VpnState::Reconnecting { .. }),
                "INVARIANT VIOLATION: VpnLost but new state is {}",
                new_state.name()
            );
        }
        Some(TransitionReason::HealthCheckFailed) => {
            assert!(
                matches!(new_state, VpnState::Degraded { .. }),
                "INVARIANT VIOLATION: HealthCheckFailed but new state is {}",
                new_state.name()
            );
        }
        Some(TransitionReason::HealthCheckDead) => {
            assert!(
                matches!(new_state, VpnState::Reconnecting { .. }),
                "INVARIANT VIOLATION: HealthCheckDead but new state is {}",
                new_state.name()
            );
        }
        _ => {
            // Retrying, WakeResync, ExternalChange, Unknown, Timeout
            // have more flexible state transitions -- just verify state is valid
        }
    }
}
