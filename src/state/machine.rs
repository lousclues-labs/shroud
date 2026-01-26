//! Core state machine implementation
//!
//! Handles state transitions based on events, with logging and retry logic.

use log::info;

use crate::state::types::{Event, TransitionReason, VpnState};

/// Configuration for the state machine
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct StateMachineConfig {
    /// Maximum number of reconnection attempts before failing
    pub max_retries: u32,
    /// Base delay for exponential backoff in seconds
    pub base_delay_secs: u64,
    /// Maximum delay cap for backoff in seconds
    pub max_delay_secs: u64,
}

impl Default for StateMachineConfig {
    fn default() -> Self {
        Self {
            max_retries: 10,
            base_delay_secs: 2,
            max_delay_secs: 30,
        }
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

    /// Calculate the backoff delay for the current retry attempt
    #[allow(dead_code)]
    pub fn backoff_delay_secs(&self) -> u64 {
        std::cmp::min(
            self.config.base_delay_secs * (self.retries as u64 + 1),
            self.config.max_delay_secs,
        )
    }

    /// Handle an event and potentially transition to a new state
    ///
    /// Returns the transition reason if a transition occurred, None otherwise.
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
        sm.handle_event(Event::UserEnable {
            server: "test".into(),
        });
        sm.handle_event(Event::NmVpnUp {
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
        sm.handle_event(Event::HealthDegraded);

        assert!(matches!(sm.state, VpnState::Degraded { .. }));
    }

    #[test]
    fn test_degraded_to_reconnecting() {
        let mut sm = StateMachine::new();
        sm.state = VpnState::Degraded {
            server: "test".into(),
        };
        sm.handle_event(Event::HealthDead);

        assert!(matches!(sm.state, VpnState::Reconnecting { .. }));
    }

    #[test]
    fn test_user_disable_from_any_state() {
        let mut sm = StateMachine::new();

        // From Connected
        sm.state = VpnState::Connected {
            server: "test".into(),
        };
        sm.handle_event(Event::UserDisable);
        assert!(matches!(sm.state, VpnState::Disconnected));

        // From Reconnecting
        sm.state = VpnState::Reconnecting {
            server: "test".into(),
            attempt: 3,
            max_attempts: 10,
        };
        sm.handle_event(Event::UserDisable);
        assert!(matches!(sm.state, VpnState::Disconnected));

        // From Failed
        sm.state = VpnState::Failed {
            server: "test".into(),
            reason: "test".into(),
        };
        sm.handle_event(Event::UserDisable);
        assert!(matches!(sm.state, VpnState::Disconnected));
    }

    #[test]
    fn test_retry_exhaustion() {
        let config = StateMachineConfig {
            max_retries: 3,
            ..Default::default()
        };
        let mut sm = StateMachine::with_config(config);
        sm.state = VpnState::Connecting {
            server: "test".into(),
        };

        // First timeout -> Reconnecting
        sm.handle_event(Event::Timeout);
        assert!(matches!(
            sm.state,
            VpnState::Reconnecting { attempt: 1, .. }
        ));

        // Second timeout -> still Reconnecting
        sm.handle_event(Event::Timeout);
        assert!(matches!(
            sm.state,
            VpnState::Reconnecting { attempt: 2, .. }
        ));

        // Third timeout -> Failed
        sm.handle_event(Event::Timeout);
        assert!(matches!(sm.state, VpnState::Failed { .. }));
    }

    #[test]
    fn test_backoff_delay() {
        let config = StateMachineConfig {
            max_retries: 10,
            base_delay_secs: 2,
            max_delay_secs: 30,
        };
        let mut sm = StateMachine::with_config(config);

        assert_eq!(sm.backoff_delay_secs(), 2); // 2 * 1
        sm.retries = 1;
        assert_eq!(sm.backoff_delay_secs(), 4); // 2 * 2
        sm.retries = 5;
        assert_eq!(sm.backoff_delay_secs(), 12); // 2 * 6
        sm.retries = 20;
        assert_eq!(sm.backoff_delay_secs(), 30); // capped at max
    }
}
