// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Core state machine implementation
//!
//! Handles state transitions based on events, with logging and retry logic.

use tracing::info;

use crate::state::types::{Event, TransitionReason, VpnState};

/// Configuration for the state machine
#[derive(Debug, Clone)]
pub struct StateMachineConfig {
    /// Maximum number of reconnection attempts before failing
    pub max_retries: u32,
}

impl Default for StateMachineConfig {
    fn default() -> Self {
        Self { max_retries: 10 }
    }
}

/// The core VPN state machine
///
/// Manages state transitions based on events from various sources:
/// - User commands (enable/disable)
/// - NetworkManager events (vpn up/down)
/// - Health check results
/// - System events (sleep/wake)
///
/// ## Retry Counter
///
/// `self.retries` is the **canonical source of truth** for the current retry
/// count. `VpnState::Reconnecting { attempt, .. }` is always derived from
/// `self.retries` during transitions — never the other way around.
/// `max_attempts` in the variant is derived from `self.config.max_retries`.
pub struct StateMachine {
    /// Current state
    pub state: VpnState,
    /// Canonical retry counter for the current reconnection cycle.
    /// `Reconnecting { attempt }` is always set equal to this value.
    pub(super) retries: u32,
    /// Configuration
    config: StateMachineConfig,
}

impl StateMachine {
    /// Create a new state machine with default configuration
    pub fn new() -> Self {
        Self::with_config(StateMachineConfig::default())
    }

    /// Create a new state machine with custom configuration
    pub fn with_config(config: StateMachineConfig) -> Self {
        Self {
            state: VpnState::Disconnected,
            retries: 0,
            config,
        }
    }

    /// Get the current retry count
    #[allow(dead_code)]
    pub fn retries(&self) -> u32 {
        self.retries
    }

    /// Get the maximum retries from config
    pub fn max_retries(&self) -> u32 {
        self.config.max_retries
    }

    /// Handle an event and potentially transition to a new state
    ///
    /// Returns the transition reason if a transition occurred, None otherwise.
    #[must_use = "the transition reason indicates whether state changed and should be handled"]
    pub fn handle_event(&mut self, event: Event) -> Option<TransitionReason> {
        let old_state = self.state.clone();

        // Per-source-state transition rules live in `super::transitions`.
        let (new_state, reason) = self.compute_transition(&event);

        if let Some(new) = new_state {
            self.state = new;

            // Invariant: Reconnecting.attempt must always equal self.retries
            debug_assert!(
                !matches!(&self.state, VpnState::Reconnecting { attempt, .. } if *attempt != self.retries),
                "Retry counter desync: state.attempt={} but self.retries={}",
                if let VpnState::Reconnecting { attempt, .. } = &self.state {
                    *attempt
                } else {
                    0
                },
                self.retries
            );

            self.log_transition(&old_state, &self.state, &reason);
            Some(reason)
        } else {
            None
        }
    }

    /// Force set the state (for external sync scenarios like wake-from-sleep)
    ///
    /// This is the escape hatch used by the ~15 supervisor call sites that
    /// reconcile against NetworkManager rather than driving a formal event.
    /// Forcing `Disconnected` also clears the retry budget: reaching that state
    /// ends any reconnect sequence, so a counter surviving into the next
    /// attempt would silently shorten it.
    pub fn set_state(&mut self, new_state: VpnState, reason: TransitionReason) {
        if matches!(new_state, VpnState::Disconnected) {
            self.retries = 0;
        }
        let old_state = std::mem::replace(&mut self.state, new_state);
        if old_state != self.state {
            self.log_transition(&old_state, &self.state, &reason);
        }
    }

    /// Log a state transition
    fn log_transition(&self, from: &VpnState, to: &VpnState, reason: &TransitionReason) {
        info!(
            "State transition: {} → {} (reason: {})",
            from.name(),
            to.name(),
            reason
        );
    }
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disconnected_to_connecting() {
        let mut sm = StateMachine::new();
        let reason = sm.handle_event(Event::UserEnable {
            server: "test".into(),
        });

        assert!(matches!(sm.state, VpnState::Connecting { .. }));
        assert!(matches!(reason, Some(TransitionReason::UserRequested)));
    }

    #[test]
    fn test_connecting_to_connected() {
        let mut sm = StateMachine::new();
        let _ = sm.handle_event(Event::UserEnable {
            server: "test".into(),
        });
        let _ = sm.handle_event(Event::NmVpnUp {
            server: "test".into(),
        });

        assert!(matches!(sm.state, VpnState::Connected { .. }));
        assert_eq!(sm.retries, 0);
    }

    #[test]
    fn test_connected_to_degraded() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Connected {
            server: "test".into(),
        };
        let _ = sm.handle_event(Event::HealthDegraded);

        assert!(matches!(sm.state, VpnState::Degraded { .. }));
    }

    #[test]
    fn test_degraded_to_reconnecting() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Degraded {
            server: "test".into(),
        };
        let _ = sm.handle_event(Event::HealthDead);

        assert!(matches!(sm.state, VpnState::Reconnecting { .. }));
    }

    #[test]
    fn test_user_disable_from_any_state() {
        let mut sm = StateMachine::new();

        // From Connected
        sm.state = VpnState::Connected {
            server: "test".into(),
        };
        let _ = sm.handle_event(Event::UserDisable);
        assert!(matches!(sm.state, VpnState::Disconnected));

        // From Reconnecting
        sm.state = VpnState::Reconnecting {
            server: "test".into(),
            attempt: 3,
            max_attempts: 10,
        };
        let _ = sm.handle_event(Event::UserDisable);
        assert!(matches!(sm.state, VpnState::Disconnected));

        // From Failed
        sm.state = VpnState::Failed {
            server: "test".into(),
            reason: "test".into(),
        };
        let _ = sm.handle_event(Event::UserDisable);
        assert!(matches!(sm.state, VpnState::Disconnected));
    }

    #[test]
    fn test_retry_exhaustion() {
        let config = StateMachineConfig { max_retries: 3 };
        let mut sm = StateMachine::with_config(config);
        sm.state = VpnState::Connecting {
            server: "test".into(),
        };

        // First timeout -> Reconnecting
        let _ = sm.handle_event(Event::Timeout);
        assert!(matches!(
            sm.state,
            VpnState::Reconnecting { attempt: 1, .. }
        ));

        // Second timeout -> still Reconnecting
        let _ = sm.handle_event(Event::Timeout);
        assert!(matches!(
            sm.state,
            VpnState::Reconnecting { attempt: 2, .. }
        ));

        // Third timeout -> Failed
        let _ = sm.handle_event(Event::Timeout);
        assert!(matches!(sm.state, VpnState::Failed { .. }));
    }

    // ---- Extended state transition tests ----

    #[test]
    fn test_external_connection_detected() {
        let mut sm = StateMachine::new();
        // When disconnected, NmVpnUp means an external connection was detected
        let reason = sm.handle_event(Event::NmVpnUp {
            server: "external-vpn".into(),
        });

        assert!(matches!(sm.state, VpnState::Connected { ref server } if server == "external-vpn"));
        assert!(matches!(reason, Some(TransitionReason::ExternalChange)));
        assert_eq!(sm.retries, 0);
    }

    #[test]
    fn test_connected_vpn_changed() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Connected {
            server: "vpn1".into(),
        };

        let reason = sm.handle_event(Event::NmVpnChanged {
            server: "vpn2".into(),
        });

        assert!(matches!(sm.state, VpnState::Connected { ref server } if server == "vpn2"));
        assert!(matches!(reason, Some(TransitionReason::ExternalChange)));
    }

    #[test]
    fn test_connected_vpn_down_triggers_reconnect() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Connected {
            server: "vpn1".into(),
        };

        let reason = sm.handle_event(Event::NmVpnDown);

        assert!(matches!(
            sm.state,
            VpnState::Reconnecting {
                ref server,
                attempt: 1,
                ..
            } if server == "vpn1"
        ));
        assert!(matches!(reason, Some(TransitionReason::VpnLost)));
    }

    #[test]
    fn test_connected_health_ok_no_transition() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Connected {
            server: "vpn1".into(),
        };

        let reason = sm.handle_event(Event::HealthOk);
        assert!(reason.is_none());
        assert!(matches!(sm.state, VpnState::Connected { .. }));
    }

    #[test]
    fn test_degraded_health_ok_recovery() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Degraded {
            server: "vpn1".into(),
        };

        let reason = sm.handle_event(Event::HealthOk);

        assert!(matches!(sm.state, VpnState::Connected { ref server } if server == "vpn1"));
        assert!(matches!(reason, Some(TransitionReason::VpnReestablished)));
    }

    #[test]
    fn test_degraded_vpn_down() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Degraded {
            server: "vpn1".into(),
        };

        let reason = sm.handle_event(Event::NmVpnDown);

        assert!(matches!(sm.state, VpnState::Reconnecting { .. }));
        assert!(matches!(reason, Some(TransitionReason::VpnLost)));
    }

    #[test]
    fn test_reconnecting_success() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Reconnecting {
            server: "vpn1".into(),
            attempt: 3,
            max_attempts: 10,
        };
        sm.retries = 3;

        let reason = sm.handle_event(Event::NmVpnUp {
            server: "vpn1".into(),
        });

        assert!(matches!(sm.state, VpnState::Connected { .. }));
        assert!(matches!(reason, Some(TransitionReason::VpnReestablished)));
        assert_eq!(sm.retries, 0);
    }

    #[test]
    fn test_reconnecting_connection_failed() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Reconnecting {
            server: "vpn1".into(),
            attempt: 2,
            max_attempts: 10,
        };
        sm.retries = 2;

        let reason = sm.handle_event(Event::ConnectionFailed {
            reason: "VPN not found".into(),
        });

        assert!(matches!(sm.state, VpnState::Disconnected));
        assert!(matches!(reason, Some(TransitionReason::ConnectionFailed)));
    }

    #[test]
    fn test_reconnecting_timeout_increments() {
        let config = StateMachineConfig { max_retries: 5 };
        let mut sm = StateMachine::with_config(config);
        sm.state = VpnState::Reconnecting {
            server: "vpn1".into(),
            attempt: 1,
            max_attempts: 5,
        };
        sm.retries = 1;

        let reason = sm.handle_event(Event::Timeout);

        assert!(matches!(
            sm.state,
            VpnState::Reconnecting { attempt: 2, .. }
        ));
        assert!(matches!(reason, Some(TransitionReason::Retrying)));
    }

    #[test]
    fn test_reconnecting_timeout_exhausted() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Reconnecting {
            server: "vpn1".into(),
            attempt: 9,
            max_attempts: 10,
        };
        sm.retries = 9;

        let reason = sm.handle_event(Event::Timeout);

        assert!(matches!(sm.state, VpnState::Failed { .. }));
        assert!(matches!(reason, Some(TransitionReason::RetriesExhausted)));
    }

    #[test]
    fn test_failed_user_enable_restarts() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Failed {
            server: "vpn1".into(),
            reason: "timeout".into(),
        };
        sm.retries = 5;

        let reason = sm.handle_event(Event::UserEnable {
            server: "vpn1".into(),
        });

        assert!(matches!(sm.state, VpnState::Connecting { .. }));
        assert!(matches!(reason, Some(TransitionReason::UserRequested)));
        assert_eq!(sm.retries, 0);
    }

    #[test]
    fn test_failed_external_recovery() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Failed {
            server: "vpn1".into(),
            reason: "timeout".into(),
        };

        let reason = sm.handle_event(Event::NmVpnUp {
            server: "vpn2".into(),
        });

        assert!(matches!(sm.state, VpnState::Connected { ref server } if server == "vpn2"));
        assert!(matches!(reason, Some(TransitionReason::ExternalChange)));
    }

    #[test]
    fn test_connecting_connection_failed() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Connecting {
            server: "bad-vpn".into(),
        };

        let reason = sm.handle_event(Event::ConnectionFailed {
            reason: "Invalid config".into(),
        });

        assert!(matches!(sm.state, VpnState::Disconnected));
        assert!(matches!(reason, Some(TransitionReason::ConnectionFailed)));
        assert_eq!(sm.retries, 0);
    }

    #[test]
    fn test_connecting_nm_vpn_down() {
        let config = StateMachineConfig { max_retries: 5 };
        let mut sm = StateMachine::with_config(config);
        sm.state = VpnState::Connecting {
            server: "vpn1".into(),
        };

        let reason = sm.handle_event(Event::NmVpnDown);

        assert!(matches!(
            sm.state,
            VpnState::Reconnecting { attempt: 1, .. }
        ));
        assert!(matches!(reason, Some(TransitionReason::Retrying)));
    }

    #[test]
    fn test_wake_event_returns_none() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Connected {
            server: "vpn1".into(),
        };

        let reason = sm.handle_event(Event::Wake);

        // Wake doesn't change state, just signals resync
        assert!(reason.is_none());
        assert!(matches!(sm.state, VpnState::Connected { .. }));
    }

    #[test]
    fn test_sleep_event_no_transition() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Connected {
            server: "vpn1".into(),
        };

        let reason = sm.handle_event(Event::Sleep);
        assert!(reason.is_none());
    }

    #[test]
    fn test_user_disable_resets_retries() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Reconnecting {
            server: "vpn1".into(),
            attempt: 5,
            max_attempts: 10,
        };
        sm.retries = 5;

        let _ = sm.handle_event(Event::UserDisable);

        assert!(matches!(sm.state, VpnState::Disconnected));
        assert_eq!(sm.retries, 0);
    }

    #[test]
    fn test_set_state_logs_transition() {
        let mut sm = StateMachine::new();
        sm.set_state(
            VpnState::Connected {
                server: "vpn1".into(),
            },
            TransitionReason::ExternalChange,
        );
        assert!(matches!(sm.state, VpnState::Connected { .. }));
    }

    #[test]
    fn test_set_state_same_state_no_log() {
        let mut sm = StateMachine::new();
        // Setting to same state should not log
        sm.set_state(VpnState::Disconnected, TransitionReason::Unknown);
        assert!(matches!(sm.state, VpnState::Disconnected));
    }

    #[test]
    fn test_default_config_values() {
        let config = StateMachineConfig::default();
        assert_eq!(config.max_retries, 10);
    }

    #[test]
    fn test_default_impl() {
        let sm = StateMachine::default();
        assert!(matches!(sm.state, VpnState::Disconnected));
        assert_eq!(sm.retries, 0);
    }

    #[test]
    fn test_max_retries_accessor() {
        let config = StateMachineConfig { max_retries: 42 };
        let sm = StateMachine::with_config(config);
        assert_eq!(sm.max_retries(), 42);
    }

    #[test]
    fn test_retries_accessor() {
        let mut sm = StateMachine::new();
        assert_eq!(sm.retries(), 0);

        sm.state = VpnState::Connecting {
            server: "vpn".into(),
        };
        let _ = sm.handle_event(Event::Timeout);
        assert_eq!(sm.retries(), 1);
    }

    #[test]
    fn test_unhandled_events_return_none() {
        let mut sm = StateMachine::new();
        // Disconnected + NmDeviceChanged = no transition
        let result = sm.handle_event(Event::NmDeviceChanged);
        assert!(result.is_none());

        // Disconnected + HealthOk = no transition
        let result = sm.handle_event(Event::HealthOk);
        assert!(result.is_none());

        // Disconnected + HealthDegraded = no transition
        let result = sm.handle_event(Event::HealthDegraded);
        assert!(result.is_none());
    }

    #[test]
    fn test_full_lifecycle() {
        let config = StateMachineConfig { max_retries: 3 };
        let mut sm = StateMachine::with_config(config);

        // Disconnected -> Connecting
        let _ = sm.handle_event(Event::UserEnable {
            server: "vpn".into(),
        });
        assert!(matches!(sm.state, VpnState::Connecting { .. }));

        // Connecting -> Connected
        let _ = sm.handle_event(Event::NmVpnUp {
            server: "vpn".into(),
        });
        assert!(matches!(sm.state, VpnState::Connected { .. }));

        // Connected -> Degraded
        let _ = sm.handle_event(Event::HealthDegraded);
        assert!(matches!(sm.state, VpnState::Degraded { .. }));

        // Degraded -> Reconnecting
        let _ = sm.handle_event(Event::HealthDead);
        assert!(matches!(sm.state, VpnState::Reconnecting { .. }));

        // Reconnecting -> Connected
        let _ = sm.handle_event(Event::NmVpnUp {
            server: "vpn".into(),
        });
        assert!(matches!(sm.state, VpnState::Connected { .. }));

        // Connected -> Disconnected
        let _ = sm.handle_event(Event::UserDisable);
        assert!(matches!(sm.state, VpnState::Disconnected));
    }
}

#[cfg(test)]
mod retry_budget_tests {
    use super::*;
    use crate::state::{Event, TransitionReason, VpnState};

    /// The reported failure mode: a reconnect sequence climbs partway, the
    /// supervisor force-sets `Disconnected` (health check dead, VPN lost,
    /// external change...), then the user clicks Connect. The new attempt must
    /// start from a full retry budget, not inherit the old counter.
    #[test]
    fn test_manual_reconnect_after_forced_disconnect_starts_fresh() {
        let mut sm = StateMachine::with_config(StateMachineConfig { max_retries: 10 });
        sm.retries = 8;

        // Mirrors handlers/health.rs HealthCheckDead and handlers/nm.rs VpnLost.
        sm.set_state(VpnState::Disconnected, TransitionReason::HealthCheckDead);
        assert_eq!(
            sm.retries, 0,
            "forcing Disconnected must end the retry sequence"
        );

        let _ = sm.handle_event(Event::UserEnable {
            server: "vpn-a".into(),
        });
        assert_eq!(
            sm.retries, 0,
            "a user-initiated connect starts a new budget"
        );
    }

    /// Belt and braces: even if some future path reaches `Disconnected` with a
    /// stale counter, `UserEnable` itself must clear it.
    #[test]
    fn test_user_enable_from_disconnected_resets_retries() {
        let mut sm = StateMachine::with_config(StateMachineConfig { max_retries: 10 });
        sm.state = VpnState::Disconnected;
        sm.retries = 7;

        let _ = sm.handle_event(Event::UserEnable {
            server: "vpn-a".into(),
        });

        assert_eq!(sm.retries, 0);
        assert!(matches!(sm.state, VpnState::Connecting { .. }));
    }

    /// The full budget must actually be available after the reset.
    ///
    /// Asserted by equivalence rather than a hardcoded count: a machine that
    /// carried a stale counter must behave *identically* to a freshly created
    /// one driven through the same sequence.
    #[test]
    fn test_full_retry_budget_available_after_reset() {
        fn attempts_until_failed(sm: &mut StateMachine, max: u32) -> u32 {
            let _ = sm.handle_event(Event::UserEnable {
                server: "vpn-a".into(),
            });
            // The drop itself counts as the first reconnect attempt.
            let _ = sm.handle_event(Event::NmVpnDown);

            let mut timeouts = 0;
            while !matches!(sm.state, VpnState::Failed { .. }) && timeouts < max * 3 {
                let _ = sm.handle_event(Event::Timeout);
                timeouts += 1;
            }
            assert!(
                matches!(sm.state, VpnState::Failed { .. }),
                "expected the machine to exhaust its budget, ended in {:?}",
                sm.state
            );
            timeouts
        }

        let max = 5;

        let mut fresh = StateMachine::with_config(StateMachineConfig { max_retries: max });
        let baseline = attempts_until_failed(&mut fresh, max);

        // Same machine, but arriving via a forced disconnect with a stale counter.
        let mut recycled = StateMachine::with_config(StateMachineConfig { max_retries: max });
        recycled.retries = max - 1;
        recycled.set_state(VpnState::Disconnected, TransitionReason::VpnLost);
        let after_reset = attempts_until_failed(&mut recycled, max);

        assert_eq!(
            after_reset, baseline,
            "a manual reconnect after a forced disconnect must get the same \
             budget as a fresh connect (got {after_reset}, expected {baseline})"
        );
    }

    /// Forcing a non-Disconnected state must leave an in-progress reconnect
    /// sequence intact.
    #[test]
    fn test_set_state_to_other_states_preserves_retries() {
        let mut sm = StateMachine::with_config(StateMachineConfig { max_retries: 10 });
        sm.retries = 3;

        sm.set_state(
            VpnState::Connecting {
                server: "vpn-a".into(),
            },
            TransitionReason::UserRequested,
        );

        assert_eq!(sm.retries, 3);
    }
}
