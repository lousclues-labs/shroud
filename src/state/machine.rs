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
pub struct StateMachine {
    /// Current state
    pub state: VpnState,
    /// Number of retry attempts in current reconnection cycle
    retries: u32,
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
        let mut reason = TransitionReason::Unknown;

        let new_state = match (&self.state, &event) {
            // --- Disconnected ---
            (VpnState::Disconnected, Event::UserEnable { server }) => {
                reason = TransitionReason::UserRequested;
                Some(VpnState::Connecting {
                    server: server.clone(),
                })
            }
            (VpnState::Disconnected, Event::NmVpnUp { server }) => {
                // External connection detected
                self.retries = 0;
                reason = TransitionReason::ExternalChange;
                Some(VpnState::Connected {
                    server: server.clone(),
                })
            }

            // --- Connecting ---
            (VpnState::Connecting { .. }, Event::NmVpnUp { server }) => {
                self.retries = 0;
                reason = TransitionReason::VpnEstablished;
                Some(VpnState::Connected {
                    server: server.clone(),
                })
            }
            (VpnState::Connecting { .. }, Event::ConnectionFailed { reason: _ }) => {
                // Definitive failure - VPN doesn't exist, invalid config, etc.
                // Go directly to Disconnected, not Reconnecting
                self.retries = 0;
                reason = TransitionReason::ConnectionFailed;
                Some(VpnState::Disconnected)
            }
            (VpnState::Connecting { server }, Event::Timeout) => {
                self.retries += 1;
                if self.retries >= self.config.max_retries {
                    reason = TransitionReason::RetriesExhausted;
                    Some(VpnState::Failed {
                        server: server.clone(),
                        reason: "Connection timeout".to_string(),
                    })
                } else {
                    reason = TransitionReason::Retrying;
                    Some(VpnState::Reconnecting {
                        server: server.clone(),
                        attempt: self.retries,
                        max_attempts: self.config.max_retries,
                    })
                }
            }
            (VpnState::Connecting { server }, Event::NmVpnDown) => {
                self.retries += 1;
                if self.retries >= self.config.max_retries {
                    reason = TransitionReason::RetriesExhausted;
                    Some(VpnState::Failed {
                        server: server.clone(),
                        reason: "Connection failed".to_string(),
                    })
                } else {
                    reason = TransitionReason::Retrying;
                    Some(VpnState::Reconnecting {
                        server: server.clone(),
                        attempt: self.retries,
                        max_attempts: self.config.max_retries,
                    })
                }
            }

            // --- Connected ---
            (VpnState::Connected { server }, Event::HealthDegraded) => {
                reason = TransitionReason::HealthCheckFailed;
                Some(VpnState::Degraded {
                    server: server.clone(),
                })
            }
            (VpnState::Connected { server }, Event::NmVpnDown) => {
                reason = TransitionReason::VpnLost;
                Some(VpnState::Reconnecting {
                    server: server.clone(),
                    attempt: 1,
                    max_attempts: self.config.max_retries,
                })
            }
            (VpnState::Connected { .. }, Event::NmVpnChanged { server }) => {
                // External switch to different VPN
                self.retries = 0;
                reason = TransitionReason::ExternalChange;
                Some(VpnState::Connected {
                    server: server.clone(),
                })
            }
            (VpnState::Connected { .. }, Event::HealthOk) => {
                // Already connected and healthy, no transition
                None
            }

            // --- Degraded ---
            (VpnState::Degraded { server }, Event::HealthDead) => {
                reason = TransitionReason::HealthCheckDead;
                Some(VpnState::Reconnecting {
                    server: server.clone(),
                    attempt: 1,
                    max_attempts: self.config.max_retries,
                })
            }
            (VpnState::Degraded { server }, Event::HealthOk) => {
                // Recovered from degraded state
                reason = TransitionReason::VpnReestablished;
                Some(VpnState::Connected {
                    server: server.clone(),
                })
            }
            (VpnState::Degraded { server }, Event::NmVpnDown) => {
                reason = TransitionReason::VpnLost;
                Some(VpnState::Reconnecting {
                    server: server.clone(),
                    attempt: 1,
                    max_attempts: self.config.max_retries,
                })
            }

            // --- Reconnecting ---
            (VpnState::Reconnecting { .. }, Event::NmVpnUp { server }) => {
                self.retries = 0;
                reason = TransitionReason::VpnReestablished;
                Some(VpnState::Connected {
                    server: server.clone(),
                })
            }
            (VpnState::Reconnecting { .. }, Event::ConnectionFailed { reason: _ }) => {
                // Definitive failure - VPN doesn't exist, invalid config, etc.
                // Go directly to Disconnected
                self.retries = 0;
                reason = TransitionReason::ConnectionFailed;
                Some(VpnState::Disconnected)
            }
            (
                VpnState::Reconnecting {
                    server,
                    attempt,
                    max_attempts,
                },
                Event::Timeout,
            ) => {
                self.retries = *attempt + 1;
                if self.retries >= *max_attempts {
                    reason = TransitionReason::RetriesExhausted;
                    Some(VpnState::Failed {
                        server: server.clone(),
                        reason: format!("Max reconnection attempts ({}) exceeded", max_attempts),
                    })
                } else {
                    reason = TransitionReason::Retrying;
                    Some(VpnState::Reconnecting {
                        server: server.clone(),
                        attempt: self.retries,
                        max_attempts: *max_attempts,
                    })
                }
            }

            // --- Failed ---
            (VpnState::Failed { .. }, Event::UserEnable { server }) => {
                self.retries = 0;
                reason = TransitionReason::UserRequested;
                Some(VpnState::Connecting {
                    server: server.clone(),
                })
            }
            (VpnState::Failed { .. }, Event::NmVpnUp { server }) => {
                // External recovery
                self.retries = 0;
                reason = TransitionReason::ExternalChange;
                Some(VpnState::Connected {
                    server: server.clone(),
                })
            }

            // --- Global: UserDisable from any state ---
            (_, Event::UserDisable) => {
                self.retries = 0;
                reason = TransitionReason::UserRequested;
                Some(VpnState::Disconnected)
            }

            // --- Wake event: force resync ---
            (_, Event::Wake) => {
                reason = TransitionReason::WakeResync;
                // Don't change state here, just signal that a resync is needed
                // The supervisor will query NM and update accordingly
                None
            }

            // Default: no transition
            _ => None,
        };

        if let Some(new) = new_state {
            self.state = new;
            self.log_transition(&old_state, &self.state, &reason);
            Some(reason)
        } else {
            None
        }
    }

    /// Force set the state (for external sync scenarios like wake-from-sleep)
    pub fn set_state(&mut self, new_state: VpnState, reason: TransitionReason) {
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
