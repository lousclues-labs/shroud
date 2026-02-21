// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Fuzz target: lifecycle recovery proof.
//!
//! Phase 1: Random chaos events (fuzzer-controlled).
//! Phase 2: Force UserDisable. Machine MUST reach Disconnected.
//! Phase 3: Force UserEnable + NmVpnUp. Machine MUST reach Connected.
//! Phase 4: More random chaos events.
//! Phase 5: Force UserDisable again. Machine MUST reach Disconnected again.
//!
//! Proves: UserDisable is always an escape hatch. The connect sequence
//! always works from Disconnected. No chaos phase can break recovery.

#![no_main]

use libfuzzer_sys::fuzz_target;
use shroud::state::{Event, StateMachineConfig, StateMachine, VpnState};

#[path = "state_machine_common.rs"]
mod common;
use common::{event_from_byte, check_invariants};

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 { return; }

    let config = StateMachineConfig { max_retries: 5 };
    let mut machine = StateMachine::with_config(config);

    // Split input: first half is chaos phase 1, second half is chaos phase 2
    let midpoint = data.len() / 2;

    // === Phase 1: Chaos ===
    for &byte in &data[..midpoint] {
        let event = event_from_byte(byte);
        let _reason = machine.handle_event(event);
        check_invariants(&machine);
    }

    // === Phase 2: UserDisable — MUST reach Disconnected ===
    let _reason = machine.handle_event(Event::UserDisable);
    assert!(
        matches!(machine.state, VpnState::Disconnected),
        "ESCAPE HATCH VIOLATION: UserDisable did not reach Disconnected.\n\
         Current state: {:?}",
        machine.state
    );
    assert_eq!(machine.retries(), 0,
        "ESCAPE HATCH VIOLATION: UserDisable left retries at {}", machine.retries());
    check_invariants(&machine);

    // === Phase 3: Recovery — UserEnable + NmVpnUp MUST reach Connected ===
    let _reason = machine.handle_event(Event::UserEnable {
        server: "recovery-server".into(),
    });
    assert!(
        matches!(machine.state, VpnState::Connecting { .. }),
        "RECOVERY VIOLATION: UserEnable from Disconnected did not reach Connecting.\n\
         Current state: {:?}",
        machine.state
    );
    check_invariants(&machine);

    let _reason = machine.handle_event(Event::NmVpnUp {
        server: "recovery-server".into(),
    });
    assert!(
        matches!(machine.state, VpnState::Connected { .. }),
        "RECOVERY VIOLATION: NmVpnUp from Connecting did not reach Connected.\n\
         Current state: {:?}",
        machine.state
    );
    assert_eq!(machine.retries(), 0);
    check_invariants(&machine);

    // === Phase 4: More chaos ===
    for &byte in &data[midpoint..] {
        let event = event_from_byte(byte);
        let _reason = machine.handle_event(event);
        check_invariants(&machine);
    }

    // === Phase 5: Final UserDisable — MUST reach Disconnected again ===
    let _reason = machine.handle_event(Event::UserDisable);
    assert!(
        matches!(machine.state, VpnState::Disconnected),
        "FINAL ESCAPE HATCH VIOLATION: UserDisable did not reach Disconnected.\n\
         Current state: {:?}",
        machine.state
    );
    assert_eq!(machine.retries(), 0);
    check_invariants(&machine);
});
