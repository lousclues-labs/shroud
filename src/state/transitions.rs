// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! State-transition rules, factored out by source state.
//!
//! Each `from_*` helper handles transitions out of a single source state,
//! returning `(Option<VpnState>, TransitionReason)`. The
//! [`StateMachine::compute_transition`] entry point dispatches on the current
//! state and falls through to global rules (`UserDisable`, `Wake`) when no
//! state-specific arm matches.
//!
//! This file exists solely to keep [`super::machine`] focused on the
//! event-loop, logging, and invariants while the per-state logic lives here.

use crate::state::types::{Event, TransitionReason, VpnState};

use super::machine::StateMachine;

#[allow(clippy::wrong_self_convention)]
impl StateMachine {
    /// Compute the next state for the given event.
    ///
    /// Returns `(Some(next_state), reason)` on a transition, or
    /// `(None, TransitionReason::Unknown)` if no rule matched.
    pub(super) fn compute_transition(
        &mut self,
        event: &Event,
    ) -> (Option<VpnState>, TransitionReason) {
        // Per-state rules
        let from_state = match &self.state {
            VpnState::Disconnected => self.from_disconnected(event),
            VpnState::Connecting { server } => {
                let server = server.clone();
                self.from_connecting(&server, event)
            }
            VpnState::Connected { server } => {
                let server = server.clone();
                self.from_connected(&server, event)
            }
            VpnState::Degraded { server } => {
                let server = server.clone();
                self.from_degraded(&server, event)
            }
            VpnState::Reconnecting { server, .. } => {
                let server = server.clone();
                self.from_reconnecting(&server, event)
            }
            VpnState::Failed { .. } => self.from_failed(event),
        };

        if from_state.0.is_some() {
            return from_state;
        }

        // Global fall-through rules (apply regardless of source state)
        match event {
            Event::UserDisable => {
                self.retries = 0;
                (
                    Some(VpnState::Disconnected),
                    TransitionReason::UserRequested,
                )
            }
            Event::Wake => {
                // Signal that a resync is needed; supervisor handles it.
                // No state change here.
                (None, TransitionReason::WakeResync)
            }
            _ => (None, TransitionReason::Unknown),
        }
    }

    fn from_disconnected(&mut self, event: &Event) -> (Option<VpnState>, TransitionReason) {
        match event {
            Event::UserEnable { server } => {
                // A fresh user-initiated connect starts a new retry budget.
                // Without this, a counter left over from an earlier reconnect
                // loop carries into the new attempt and exhausts it early —
                // e.g. failing at attempt 8 then reconnecting manually gave
                // 2 attempts instead of 10.
                self.retries = 0;
                (
                    Some(VpnState::Connecting {
                        server: server.clone(),
                    }),
                    TransitionReason::UserRequested,
                )
            }
            Event::NmVpnUp { server } => {
                // External connection detected
                self.retries = 0;
                (
                    Some(VpnState::Connected {
                        server: server.clone(),
                    }),
                    TransitionReason::ExternalChange,
                )
            }
            _ => (None, TransitionReason::Unknown),
        }
    }

    fn from_connecting(
        &mut self,
        server: &str,
        event: &Event,
    ) -> (Option<VpnState>, TransitionReason) {
        match event {
            Event::NmVpnUp { server: new_server } => {
                self.retries = 0;
                (
                    Some(VpnState::Connected {
                        server: new_server.clone(),
                    }),
                    TransitionReason::VpnEstablished,
                )
            }
            Event::ConnectionFailed { reason: _ } => {
                // Definitive failure - VPN doesn't exist, invalid config, etc.
                // Go directly to Disconnected, not Reconnecting
                self.retries = 0;
                (
                    Some(VpnState::Disconnected),
                    TransitionReason::ConnectionFailed,
                )
            }
            Event::Timeout => {
                self.retries += 1;
                if self.retries >= self.max_retries() {
                    (
                        Some(VpnState::Failed {
                            server: server.to_string(),
                            reason: "Connection timeout".to_string(),
                        }),
                        TransitionReason::RetriesExhausted,
                    )
                } else {
                    (
                        Some(VpnState::Reconnecting {
                            server: server.to_string(),
                            attempt: self.retries,
                            max_attempts: self.max_retries(),
                        }),
                        TransitionReason::Retrying,
                    )
                }
            }
            Event::NmVpnDown => {
                self.retries += 1;
                if self.retries >= self.max_retries() {
                    (
                        Some(VpnState::Failed {
                            server: server.to_string(),
                            reason: "Connection failed".to_string(),
                        }),
                        TransitionReason::RetriesExhausted,
                    )
                } else {
                    (
                        Some(VpnState::Reconnecting {
                            server: server.to_string(),
                            attempt: self.retries,
                            max_attempts: self.max_retries(),
                        }),
                        TransitionReason::Retrying,
                    )
                }
            }
            _ => (None, TransitionReason::Unknown),
        }
    }

    fn from_connected(
        &mut self,
        server: &str,
        event: &Event,
    ) -> (Option<VpnState>, TransitionReason) {
        match event {
            Event::HealthDegraded => (
                Some(VpnState::Degraded {
                    server: server.to_string(),
                }),
                TransitionReason::HealthCheckFailed,
            ),
            Event::NmVpnDown => {
                self.retries = 1;
                (
                    Some(VpnState::Reconnecting {
                        server: server.to_string(),
                        attempt: self.retries,
                        max_attempts: self.max_retries(),
                    }),
                    TransitionReason::VpnLost,
                )
            }
            Event::NmVpnChanged { server: new_server } => {
                // External switch to different VPN
                self.retries = 0;
                (
                    Some(VpnState::Connected {
                        server: new_server.clone(),
                    }),
                    TransitionReason::ExternalChange,
                )
            }
            Event::HealthOk => {
                // Already connected and healthy, no transition
                (None, TransitionReason::Unknown)
            }
            _ => (None, TransitionReason::Unknown),
        }
    }

    fn from_degraded(
        &mut self,
        server: &str,
        event: &Event,
    ) -> (Option<VpnState>, TransitionReason) {
        match event {
            Event::HealthDead => {
                self.retries = 1;
                (
                    Some(VpnState::Reconnecting {
                        server: server.to_string(),
                        attempt: self.retries,
                        max_attempts: self.max_retries(),
                    }),
                    TransitionReason::HealthCheckDead,
                )
            }
            Event::HealthOk => {
                // Recovered from degraded state
                (
                    Some(VpnState::Connected {
                        server: server.to_string(),
                    }),
                    TransitionReason::VpnReestablished,
                )
            }
            Event::NmVpnDown => {
                self.retries = 1;
                (
                    Some(VpnState::Reconnecting {
                        server: server.to_string(),
                        attempt: self.retries,
                        max_attempts: self.max_retries(),
                    }),
                    TransitionReason::VpnLost,
                )
            }
            _ => (None, TransitionReason::Unknown),
        }
    }

    fn from_reconnecting(
        &mut self,
        server: &str,
        event: &Event,
    ) -> (Option<VpnState>, TransitionReason) {
        match event {
            Event::NmVpnUp { server: new_server } => {
                self.retries = 0;
                (
                    Some(VpnState::Connected {
                        server: new_server.clone(),
                    }),
                    TransitionReason::VpnReestablished,
                )
            }
            Event::ConnectionFailed { reason: _ } => {
                // Definitive failure - VPN doesn't exist, invalid config, etc.
                // Go directly to Disconnected
                self.retries = 0;
                (
                    Some(VpnState::Disconnected),
                    TransitionReason::ConnectionFailed,
                )
            }
            Event::Timeout => {
                self.retries += 1;
                if self.retries >= self.max_retries() {
                    let max = self.max_retries();
                    (
                        Some(VpnState::Failed {
                            server: server.to_string(),
                            reason: format!("Max reconnection attempts ({}) exceeded", max),
                        }),
                        TransitionReason::RetriesExhausted,
                    )
                } else {
                    (
                        Some(VpnState::Reconnecting {
                            server: server.to_string(),
                            attempt: self.retries,
                            max_attempts: self.max_retries(),
                        }),
                        TransitionReason::Retrying,
                    )
                }
            }
            _ => (None, TransitionReason::Unknown),
        }
    }

    fn from_failed(&mut self, event: &Event) -> (Option<VpnState>, TransitionReason) {
        match event {
            Event::UserEnable { server } => {
                self.retries = 0;
                (
                    Some(VpnState::Connecting {
                        server: server.clone(),
                    }),
                    TransitionReason::UserRequested,
                )
            }
            Event::NmVpnUp { server } => {
                // External recovery
                self.retries = 0;
                (
                    Some(VpnState::Connected {
                        server: server.clone(),
                    }),
                    TransitionReason::ExternalChange,
                )
            }
            _ => (None, TransitionReason::Unknown),
        }
    }
}
