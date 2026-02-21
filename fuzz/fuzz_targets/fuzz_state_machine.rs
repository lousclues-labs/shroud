// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Fuzz target: chaos cannon.
//!
//! Throws millions of random event sequences at the state machine.
//! Checks all invariants after every event. Simulates rapid-fire
//! duplicate events at ~4% probability.

#![no_main]

use libfuzzer_sys::fuzz_target;
use shroud::state::{StateMachineConfig, StateMachine};

#[path = "state_machine_common.rs"]
mod common;
use common::{event_from_byte, check_invariants};

fuzz_target!(|data: &[u8]| {
    let config = StateMachineConfig { max_retries: 5 };
    let mut machine = StateMachine::with_config(config);

    for chunk in data.chunks(2) {
        let event_byte = chunk[0];
        let timing_byte = chunk.get(1).copied().unwrap_or(0);

        // ~4% chance of rapid-fire: same event 3x
        let iterations: u8 = if timing_byte < 10 { 3 } else { 1 };

        let event = event_from_byte(event_byte);

        for _ in 0..iterations {
            let _reason = machine.handle_event(event.clone());
            check_invariants(&machine);
        }
    }

    check_invariants(&machine);
});
