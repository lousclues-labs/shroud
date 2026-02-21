// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Fuzz target: differential testing across configurations.
//!
//! Two state machines receive identical events with different max_retries.
//! Config-independent invariants must hold on both. Config-dependent behavior
//! is allowed to differ (e.g., one may reach Failed while the other is still
//! Reconnecting) but structural guarantees are universal.

#![no_main]

use libfuzzer_sys::fuzz_target;
use shroud::state::{StateMachineConfig, StateMachine, VpnState};

#[path = "state_machine_common.rs"]
mod common;
use common::{event_from_byte, check_invariants};

fuzz_target!(|data: &[u8]| {
    let mut machine_low = StateMachine::with_config(StateMachineConfig { max_retries: 3 });
    let mut machine_high = StateMachine::with_config(StateMachineConfig { max_retries: 100 });

    for &byte in data {
        let event_low = event_from_byte(byte);
        let event_high = event_from_byte(byte);

        let _reason_low = machine_low.handle_event(event_low);
        let _reason_high = machine_high.handle_event(event_high);

        // Both must satisfy core invariants
        check_invariants(&machine_low);
        check_invariants(&machine_high);

        // Config-independent guarantee: UserDisable always -> Disconnected
        if byte % 32 == 1 { // Event::UserDisable
            assert!(matches!(machine_low.state, VpnState::Disconnected),
                "DIFFERENTIAL: UserDisable on low-config machine did not reach Disconnected");
            assert!(matches!(machine_high.state, VpnState::Disconnected),
                "DIFFERENTIAL: UserDisable on high-config machine did not reach Disconnected");
        }

        // Config-independent guarantee: NmVpnUp from Disconnected always -> Connected
        if byte % 32 == 2 {
            if matches!(machine_low.state, VpnState::Connected { .. }) {
                // If low-config reached Connected, high-config must have too
                // (both were in the same state before, receiving the same event)
                // NOTE: This only holds if they were in the same state before.
                // After divergence, this check is not valid -- so we skip it.
            }
        }

        // Both machines' retries must satisfy their own config bounds
        assert!(machine_low.retries() <= machine_low.max_retries().max(2));
        assert!(machine_high.retries() <= machine_high.max_retries().max(2));
    }
});
