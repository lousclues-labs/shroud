//! State machine types and enums
//!
//! Defines the core state model for the VPN manager, following the formal
//! state machine design: Disconnected → Connecting → Connected → Degraded → Reconnecting → Failed

use std::fmt;

/// VPN connection state
///
/// This is the application's view of the VPN state, not NetworkManager's internal state.
/// The state machine handles transitions based on events from NM, health checks, and user commands.
#[derive(Debug, Clone, PartialEq)]
pub enum VpnState {
    /// No active VPN connection
    Disconnected,

    /// Currently establishing connection to a server
    Connecting { server: String },

    /// Successfully connected and verified healthy
    Connected { server: String },

    /// Connected but health checks failing (tunnel may be dead)
    /// This is a transitional state before Reconnecting
    Degraded { server: String },

    /// Connection dropped, attempting to reconnect with backoff
    Reconnecting {
        server: String,
        attempt: u32,
        max_attempts: u32,
    },

    /// Connection failed after exhausting retries
    Failed { server: String, reason: String },
}

impl VpnState {
    /// Get the server name if in a state with a server
    pub fn server_name(&self) -> Option<&str> {
        match self {
            VpnState::Connected { server }
            | VpnState::Connecting { server }
            | VpnState::Degraded { server }
            | VpnState::Reconnecting { server, .. }
            | VpnState::Failed { server, .. } => Some(server),
            VpnState::Disconnected => None,
        }
    }

    /// Check if this state represents an active or pending connection
    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            VpnState::Connected { .. }
                | VpnState::Connecting { .. }
                | VpnState::Degraded { .. }
                | VpnState::Reconnecting { .. }
        )
    }

    /// Check if this state represents a transitional state (busy)
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            VpnState::Connecting { .. } | VpnState::Reconnecting { .. }
        )
    }

    /// Get a short name for the state (for logging)
    pub fn name(&self) -> &'static str {
        match self {
            VpnState::Disconnected => "Disconnected",
            VpnState::Connecting { .. } => "Connecting",
            VpnState::Connected { .. } => "Connected",
            VpnState::Degraded { .. } => "Degraded",
            VpnState::Reconnecting { .. } => "Reconnecting",
            VpnState::Failed { .. } => "Failed",
        }
    }
}

impl fmt::Display for VpnState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VpnState::Disconnected => write!(f, "Disconnected"),
            VpnState::Connecting { server } => write!(f, "Connecting to {}", server),
            VpnState::Connected { server } => write!(f, "Connected to {}", server),
            VpnState::Degraded { server } => write!(f, "Degraded: {}", server),
            VpnState::Reconnecting {
                server,
                attempt,
                max_attempts,
            } => {
                write!(
                    f,
                    "Reconnecting to {} ({}/{})",
                    server, attempt, max_attempts
                )
            }
            VpnState::Failed { server, reason } => {
                write!(f, "Failed: {} - {}", server, reason)
            }
        }
    }
}

/// Events that can trigger state transitions
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Event {
    // User-initiated events
    /// User requested VPN connection
    UserEnable { server: String },
    /// User requested disconnection
    UserDisable,

    // NetworkManager events
    /// NM reports VPN is now active
    NmVpnUp { server: String },
    /// NM reports VPN went down
    NmVpnDown,
    /// NM reports a different VPN is active (external switch)
    NmVpnChanged { server: String },
    /// NM device changed (wifi roam, ethernet plug/unplug)
    NmDeviceChanged,

    // Health check events
    /// Health check passed
    HealthOk,
    /// Health check shows degraded connectivity
    HealthDegraded,
    /// Health check failed completely (tunnel is dead)
    HealthDead,

    // System events
    /// System is going to sleep
    Sleep,
    /// System woke from sleep
    Wake,

    // Internal events
    /// Connection/reconnection attempt timed out
    Timeout,
    /// Connection definitively failed (VPN doesn't exist, invalid config, etc.)
    ConnectionFailed { reason: String },
    /// Endpoint/server failed, should try another
    EndpointFailed { reason: String },
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::UserEnable { server } => write!(f, "UserEnable({})", server),
            Event::UserDisable => write!(f, "UserDisable"),
            Event::NmVpnUp { server } => write!(f, "NmVpnUp({})", server),
            Event::NmVpnDown => write!(f, "NmVpnDown"),
            Event::NmVpnChanged { server } => write!(f, "NmVpnChanged({})", server),
            Event::NmDeviceChanged => write!(f, "NmDeviceChanged"),
            Event::HealthOk => write!(f, "HealthOk"),
            Event::HealthDegraded => write!(f, "HealthDegraded"),
            Event::HealthDead => write!(f, "HealthDead"),
            Event::Sleep => write!(f, "Sleep"),
            Event::Wake => write!(f, "Wake"),
            Event::Timeout => write!(f, "Timeout"),
            Event::ConnectionFailed { reason } => write!(f, "ConnectionFailed({})", reason),
            Event::EndpointFailed { reason } => write!(f, "EndpointFailed({})", reason),
        }
    }
}

/// Reason for a state transition (for logging and diagnostics)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum TransitionReason {
    /// User requested the change
    UserRequested,
    /// VPN connection was established
    VpnEstablished,
    /// VPN connection was lost unexpectedly
    VpnLost,
    /// VPN connection was re-established after reconnection
    VpnReestablished,
    /// Health check failed
    HealthCheckFailed,
    /// Health check shows tunnel is dead
    HealthCheckDead,
    /// Connection attempt timed out
    Timeout,
    /// Retrying after failure
    Retrying,
    /// All retries exhausted
    RetriesExhausted,
    /// Connection definitively failed (invalid VPN, doesn't exist, etc.)
    ConnectionFailed,
    /// System wake from sleep
    WakeResync,
    /// External VPN change detected
    ExternalChange,
    /// Unknown/unspecified reason
    Unknown,
}

impl fmt::Display for TransitionReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TransitionReason::UserRequested => "user_requested",
            TransitionReason::VpnEstablished => "vpn_established",
            TransitionReason::VpnLost => "vpn_lost",
            TransitionReason::VpnReestablished => "vpn_reestablished",
            TransitionReason::HealthCheckFailed => "health_check_failed",
            TransitionReason::HealthCheckDead => "health_check_dead",
            TransitionReason::Timeout => "timeout",
            TransitionReason::Retrying => "retrying",
            TransitionReason::RetriesExhausted => "retries_exhausted",
            TransitionReason::ConnectionFailed => "connection_failed",
            TransitionReason::WakeResync => "wake_resync",
            TransitionReason::ExternalChange => "external_change",
            TransitionReason::Unknown => "unknown",
        };
        write!(f, "{}", s)
    }
}

/// NetworkManager VPN connection state (from nmcli)
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NmVpnState {
    /// VPN is activating (connecting)
    Activating,
    /// VPN is fully activated (connected)
    Activated,
    /// VPN is deactivating (disconnecting)
    Deactivating,
    /// VPN is not active
    Inactive,
}

impl fmt::Display for NmVpnState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            NmVpnState::Activating => "activating",
            NmVpnState::Activated => "activated",
            NmVpnState::Deactivating => "deactivating",
            NmVpnState::Inactive => "inactive",
        };
        write!(f, "{}", s)
    }
}

/// Result from querying active VPN with state information
#[derive(Debug, Clone)]
pub struct ActiveVpnInfo {
    /// Connection name
    pub name: String,
    /// Current state
    pub state: NmVpnState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vpn_state_server_name() {
        let state = VpnState::Connected {
            server: "test-server".to_string(),
        };
        assert_eq!(state.server_name(), Some("test-server"));

        let state = VpnState::Disconnected;
        assert_eq!(state.server_name(), None);

        let state = VpnState::Degraded {
            server: "degraded-server".to_string(),
        };
        assert_eq!(state.server_name(), Some("degraded-server"));
    }

    #[test]
    fn test_vpn_state_is_active() {
        assert!(!VpnState::Disconnected.is_active());
        assert!(VpnState::Connected { server: "s".into() }.is_active());
        assert!(VpnState::Connecting { server: "s".into() }.is_active());
        assert!(VpnState::Degraded { server: "s".into() }.is_active());
        assert!(!VpnState::Failed {
            server: "s".into(),
            reason: "r".into()
        }
        .is_active());
    }

    #[test]
    fn test_vpn_state_is_busy() {
        assert!(!VpnState::Disconnected.is_busy());
        assert!(!VpnState::Connected { server: "s".into() }.is_busy());
        assert!(VpnState::Connecting { server: "s".into() }.is_busy());
        assert!(VpnState::Reconnecting {
            server: "s".into(),
            attempt: 1,
            max_attempts: 5
        }
        .is_busy());
    }

    #[test]
    fn test_vpn_state_name() {
        assert_eq!(VpnState::Disconnected.name(), "Disconnected");
        assert_eq!(VpnState::Connecting { server: "s".into() }.name(), "Connecting");
        assert_eq!(VpnState::Connected { server: "s".into() }.name(), "Connected");
        assert_eq!(VpnState::Degraded { server: "s".into() }.name(), "Degraded");
        assert_eq!(VpnState::Reconnecting { server: "s".into(), attempt: 1, max_attempts: 5 }.name(), "Reconnecting");
        assert_eq!(VpnState::Failed { server: "s".into(), reason: "r".into() }.name(), "Failed");
    }

    #[test]
    fn test_vpn_state_display() {
        let state = VpnState::Connected { server: "my-vpn".into() };
        let display = format!("{}", state);
        assert!(display.contains("Connected"));
        assert!(display.contains("my-vpn"));

        let state = VpnState::Reconnecting { server: "s".into(), attempt: 3, max_attempts: 10 };
        let display = format!("{}", state);
        assert!(display.contains("3"));
        assert!(display.contains("10"));
    }

    #[test]
    fn test_vpn_state_clone() {
        let state = VpnState::Connected { server: "test".into() };
        let cloned = state.clone();
        assert_eq!(state, cloned);
    }

    #[test]
    fn test_vpn_state_equality() {
        let s1 = VpnState::Connected { server: "a".into() };
        let s2 = VpnState::Connected { server: "a".into() };
        let s3 = VpnState::Connected { server: "b".into() };
        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
    }

    #[test]
    fn test_event_display() {
        let event = Event::UserEnable { server: "vpn".into() };
        let display = format!("{}", event);
        assert!(display.contains("UserEnable"));
        assert!(display.contains("vpn"));

        let event = Event::ConnectionFailed { reason: "timeout".into() };
        let display = format!("{}", event);
        assert!(display.contains("ConnectionFailed"));
        assert!(display.contains("timeout"));
    }

    #[test]
    fn test_event_clone() {
        let event = Event::NmVpnUp { server: "test".into() };
        let cloned = event.clone();
        assert!(matches!(cloned, Event::NmVpnUp { server } if server == "test"));
    }

    #[test]
    fn test_transition_reason_display() {
        let reason = TransitionReason::UserRequested;
        assert_eq!(format!("{}", reason), "user_requested");

        let reason = TransitionReason::VpnEstablished;
        assert_eq!(format!("{}", reason), "vpn_established");

        let reason = TransitionReason::RetriesExhausted;
        assert_eq!(format!("{}", reason), "retries_exhausted");
    }

    #[test]
    fn test_nm_vpn_state_display() {
        assert_eq!(format!("{}", NmVpnState::Activating), "activating");
        assert_eq!(format!("{}", NmVpnState::Activated), "activated");
        assert_eq!(format!("{}", NmVpnState::Deactivating), "deactivating");
        assert_eq!(format!("{}", NmVpnState::Inactive), "inactive");
    }

    #[test]
    fn test_nm_vpn_state_equality() {
        assert_eq!(NmVpnState::Activated, NmVpnState::Activated);
        assert_ne!(NmVpnState::Activated, NmVpnState::Activating);
    }

    #[test]
    fn test_active_vpn_info() {
        let info = ActiveVpnInfo {
            name: "my-vpn".to_string(),
            state: NmVpnState::Activated,
        };
        assert_eq!(info.name, "my-vpn");
        assert_eq!(info.state, NmVpnState::Activated);
    }

    #[test]
    fn test_all_event_variants() {
        // Ensure all event variants can be constructed
        let events = vec![
            Event::UserEnable { server: "s".into() },
            Event::UserDisable,
            Event::NmVpnUp { server: "s".into() },
            Event::NmVpnDown,
            Event::NmVpnChanged { server: "s".into() },
            Event::NmDeviceChanged,
            Event::HealthOk,
            Event::HealthDegraded,
            Event::HealthDead,
            Event::Sleep,
            Event::Wake,
            Event::Timeout,
            Event::ConnectionFailed { reason: "r".into() },
            Event::EndpointFailed { reason: "r".into() },
        ];
        assert_eq!(events.len(), 14);
    }

    #[test]
    fn test_all_transition_reasons() {
        let reasons = vec![
            TransitionReason::UserRequested,
            TransitionReason::VpnEstablished,
            TransitionReason::VpnLost,
            TransitionReason::VpnReestablished,
            TransitionReason::HealthCheckFailed,
            TransitionReason::HealthCheckDead,
            TransitionReason::Timeout,
            TransitionReason::Retrying,
            TransitionReason::RetriesExhausted,
            TransitionReason::ConnectionFailed,
            TransitionReason::WakeResync,
            TransitionReason::ExternalChange,
            TransitionReason::Unknown,
        ];
        assert_eq!(reasons.len(), 13);
    }
}
