// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Fuzz target: determinism proof.
//!
//! Two state machines receive identical event sequences. Their states
//! must be identical after every event. Proves pure determinism.

#![no_main]

use libfuzzer_sys::fuzz_target;
use shroud::state::{StateMachineConfig, StateMachine};

#[path = "state_machine_common.rs"]
mod common;
use common::{event_from_byte, check_invariants};

fuzz_target!(|data: &[u8]| {
    let config_a = StateMachineConfig { max_retries: 5 };
    let config_b = StateMachineConfig { max_retries: 5 };
    let mut machine_a = StateMachine::with_config(config_a);
    let mut machine_b = StateMachine::with_config(config_b);

    for &byte in data {
        let event_a = event_from_byte(byte);
        let event_b = event_from_byte(byte);

        let reason_a = machine_a.handle_event(event_a);
        let reason_b = machine_b.handle_event(event_b);

        // States must be identical
        assert_eq!(
            machine_a.state, machine_b.state,
            "DETERMINISM VIOLATION: same event produced different states\n\
             Machine A: {:?}\nMachine B: {:?}",
            machine_a.state, machine_b.state
        );

        // Retry counts must be identical
        assert_eq!(
            machine_a.retries(), machine_b.retries(),
            "DETERMINISM VIOLATION: same event produced different retry counts\n\
             Machine A retries: {}\nMachine B retries: {}",
            machine_a.retries(), machine_b.retries()
        );

        // Transition reasons must match
        // (both are Option, and the inner variants should be the same)
        let reason_a_str = reason_a.as_ref().map(|r| format!("{}", r));
        let reason_b_str = reason_b.as_ref().map(|r| format!("{}", r));
        assert_eq!(
            reason_a_str, reason_b_str,
            "DETERMINISM VIOLATION: same event produced different reasons"
        );

        check_invariants(&machine_a);
    }
});
