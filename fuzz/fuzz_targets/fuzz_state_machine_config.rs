// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Fuzz target: config extremes.
//!
//! First 4 bytes of input -> max_retries (u32 little-endian).
//! Remaining bytes -> random event sequence.
//!
//! Tests the state machine with max_retries values from 0 to u32::MAX.
//! Proves: configuration values cannot break state machine invariants.

#![no_main]

use libfuzzer_sys::fuzz_target;
use shroud::state::{StateMachineConfig, StateMachine, VpnState};

#[path = "state_machine_common.rs"]
mod common;
use common::{event_from_byte, check_invariants};

fuzz_target!(|data: &[u8]| {
    if data.len() < 5 { return; } // Need at least 4 bytes for config + 1 for events

    // First 4 bytes -> max_retries
    let max_retries = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);

    let config = StateMachineConfig { max_retries };
    let mut machine = StateMachine::with_config(config);

    // Verify config was applied
    assert_eq!(machine.max_retries(), max_retries);

    for &byte in &data[4..] {
        let event = event_from_byte(byte);
        let _reason = machine.handle_event(event);
        check_invariants(&machine);
    }

    // Special check for max_retries == 0:
    // Any timeout from Connecting should go directly to Failed
    if max_retries == 0 {
        machine = StateMachine::with_config(StateMachineConfig { max_retries: 0 });
        let _ = machine.handle_event(shroud::state::Event::UserEnable {
            server: "test".into(),
        });
        // Now in Connecting. A Timeout should exhaust retries immediately.
        let _ = machine.handle_event(shroud::state::Event::Timeout);
        // With max_retries=0, retries (1) >= max_retries (0), so should be Failed
        assert!(
            matches!(machine.state, VpnState::Failed { .. }),
            "CONFIG VIOLATION: max_retries=0, Timeout from Connecting did not reach Failed.\n\
             Current state: {:?}", machine.state
        );
        check_invariants(&machine);
    }

    check_invariants(&machine);
});
