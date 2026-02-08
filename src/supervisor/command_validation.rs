//! Command validation and formatting — pure functions, easily testable.
//!
//! These functions contain no I/O, no async, no system calls.
//! They validate commands, format output, and parse actions.

use crate::state::VpnState;

/// Result of command validation
#[derive(Debug, PartialEq)]
pub enum CommandValidation {
    Valid,
    InvalidVpnName(String),
    VpnNotFound(String),
    AlreadyConnected(String),
    NotConnected,
    #[allow(dead_code)]
    InvalidAction(String),
}

/// Validate a connect command against current state.
pub fn validate_connect(
    vpn_name: &str,
    available_vpns: &[String],
    current_state: &VpnState,
) -> CommandValidation {
    if vpn_name.is_empty() {
        return CommandValidation::InvalidVpnName("VPN name cannot be empty".into());
    }

    if vpn_name.len() > 255 {
        return CommandValidation::InvalidVpnName("VPN name too long".into());
    }

    if !available_vpns.iter().any(|v| v == vpn_name) {
        return CommandValidation::VpnNotFound(vpn_name.into());
    }

    if let VpnState::Connected { server } = current_state {
        if server == vpn_name {
            return CommandValidation::AlreadyConnected(vpn_name.into());
        }
    }

    CommandValidation::Valid
}

/// Validate a disconnect command against current state.
pub fn validate_disconnect(current_state: &VpnState) -> CommandValidation {
    match current_state {
        VpnState::Disconnected => CommandValidation::NotConnected,
        _ => CommandValidation::Valid,
    }
}

/// Format a human-readable status string.
pub fn format_status(state: &VpnState, kill_switch_enabled: bool) -> String {
    let state_str = match state {
        VpnState::Disconnected => "Disconnected".to_string(),
        VpnState::Connecting { server } => format!("Connecting to {}", server),
        VpnState::Connected { server } => format!("Connected to {}", server),
        VpnState::Reconnecting {
            server,
            attempt,
            max_attempts,
        } => format!(
            "Reconnecting to {} (attempt {}/{})",
            server, attempt, max_attempts
        ),
        VpnState::Degraded { server } => format!("Degraded connection to {}", server),
        VpnState::Failed { server, reason } => format!("Failed: {} - {}", server, reason),
    };

    let ks_str = if kill_switch_enabled {
        "enabled"
    } else {
        "disabled"
    };

    format!("Status: {}\nKill Switch: {}", state_str, ks_str)
}

/// Format a VPN connection list with an optional active marker.
pub fn format_list(vpns: &[String], active_vpn: Option<&str>) -> String {
    if vpns.is_empty() {
        return "No VPN connections configured".to_string();
    }

    vpns.iter()
        .map(|vpn| {
            if Some(vpn.as_str()) == active_vpn {
                format!("* {} (active)", vpn)
            } else {
                format!("  {}", vpn)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parsed kill-switch action.
#[derive(Debug, PartialEq)]
pub enum KsAction {
    Enable,
    Disable,
    Status,
}

/// Parse a user-supplied kill-switch action string.
pub fn parse_ks_action(action: &str) -> Result<KsAction, String> {
    match action.to_lowercase().as_str() {
        "on" | "enable" | "1" | "true" => Ok(KsAction::Enable),
        "off" | "disable" | "0" | "false" => Ok(KsAction::Disable),
        "status" | "state" | "?" => Ok(KsAction::Status),
        _ => Err(format!("Invalid kill switch action: {}", action)),
    }
}

/// Determine whether a state transition should trigger a tray icon update.
pub fn should_update_tray(old: &VpnState, new: &VpnState) -> bool {
    std::mem::discriminant(old) != std::mem::discriminant(new)
        || old.server_name() != new.server_name()
}

/// Determine whether a state is eligible for auto-reconnect.
pub fn should_auto_reconnect(state: &VpnState, auto_reconnect_enabled: bool) -> bool {
    if !auto_reconnect_enabled {
        return false;
    }
    matches!(
        state,
        VpnState::Degraded { .. } | VpnState::Reconnecting { .. }
    )
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ----- validate_connect -----

    mod validate_connect_tests {
        use super::*;

        #[test]
        fn test_valid_connect() {
            let vpns = vec!["vpn1".into(), "vpn2".into()];
            let state = VpnState::Disconnected;
            assert_eq!(
                validate_connect("vpn1", &vpns, &state),
                CommandValidation::Valid
            );
        }

        #[test]
        fn test_empty_vpn_name() {
            let vpns = vec!["vpn1".into()];
            let state = VpnState::Disconnected;
            assert!(matches!(
                validate_connect("", &vpns, &state),
                CommandValidation::InvalidVpnName(_)
            ));
        }

        #[test]
        fn test_vpn_name_too_long() {
            let vpns = vec!["vpn1".into()];
            let state = VpnState::Disconnected;
            let long = "x".repeat(256);
            assert!(matches!(
                validate_connect(&long, &vpns, &state),
                CommandValidation::InvalidVpnName(_)
            ));
        }

        #[test]
        fn test_vpn_not_found() {
            let vpns = vec!["vpn1".into(), "vpn2".into()];
            let state = VpnState::Disconnected;
            assert_eq!(
                validate_connect("vpn3", &vpns, &state),
                CommandValidation::VpnNotFound("vpn3".into())
            );
        }

        #[test]
        fn test_already_connected_to_same_vpn() {
            let vpns = vec!["vpn1".into()];
            let state = VpnState::Connected {
                server: "vpn1".into(),
            };
            assert_eq!(
                validate_connect("vpn1", &vpns, &state),
                CommandValidation::AlreadyConnected("vpn1".into())
            );
        }

        #[test]
        fn test_connect_to_different_vpn_while_connected() {
            let vpns = vec!["vpn1".into(), "vpn2".into()];
            let state = VpnState::Connected {
                server: "vpn1".into(),
            };
            assert_eq!(
                validate_connect("vpn2", &vpns, &state),
                CommandValidation::Valid
            );
        }

        #[test]
        fn test_connect_from_failed_state() {
            let vpns = vec!["vpn1".into()];
            let state = VpnState::Failed {
                server: "vpn1".into(),
                reason: "timeout".into(),
            };
            assert_eq!(
                validate_connect("vpn1", &vpns, &state),
                CommandValidation::Valid
            );
        }

        #[test]
        fn test_connect_from_reconnecting_state() {
            let vpns = vec!["vpn1".into()];
            let state = VpnState::Reconnecting {
                server: "vpn1".into(),
                attempt: 2,
                max_attempts: 10,
            };
            assert_eq!(
                validate_connect("vpn1", &vpns, &state),
                CommandValidation::Valid
            );
        }

        #[test]
        fn test_empty_available_list() {
            let vpns: Vec<String> = vec![];
            let state = VpnState::Disconnected;
            assert!(matches!(
                validate_connect("vpn1", &vpns, &state),
                CommandValidation::VpnNotFound(_)
            ));
        }
    }

    // ----- validate_disconnect -----

    mod validate_disconnect_tests {
        use super::*;

        #[test]
        fn test_disconnect_when_connected() {
            let state = VpnState::Connected {
                server: "vpn1".into(),
            };
            assert_eq!(validate_disconnect(&state), CommandValidation::Valid);
        }

        #[test]
        fn test_disconnect_when_not_connected() {
            assert_eq!(
                validate_disconnect(&VpnState::Disconnected),
                CommandValidation::NotConnected
            );
        }

        #[test]
        fn test_disconnect_when_connecting() {
            let state = VpnState::Connecting {
                server: "vpn1".into(),
            };
            assert_eq!(validate_disconnect(&state), CommandValidation::Valid);
        }

        #[test]
        fn test_disconnect_when_degraded() {
            let state = VpnState::Degraded {
                server: "vpn1".into(),
            };
            assert_eq!(validate_disconnect(&state), CommandValidation::Valid);
        }

        #[test]
        fn test_disconnect_when_reconnecting() {
            let state = VpnState::Reconnecting {
                server: "vpn1".into(),
                attempt: 3,
                max_attempts: 10,
            };
            assert_eq!(validate_disconnect(&state), CommandValidation::Valid);
        }

        #[test]
        fn test_disconnect_when_failed() {
            let state = VpnState::Failed {
                server: "vpn1".into(),
                reason: "err".into(),
            };
            assert_eq!(validate_disconnect(&state), CommandValidation::Valid);
        }
    }

    // ----- format_status -----

    mod format_status_tests {
        use super::*;

        #[test]
        fn test_disconnected_status() {
            let out = format_status(&VpnState::Disconnected, false);
            assert!(out.contains("Disconnected"));
            assert!(out.contains("disabled"));
        }

        #[test]
        fn test_connected_status_with_ks() {
            let state = VpnState::Connected {
                server: "my-vpn".into(),
            };
            let out = format_status(&state, true);
            assert!(out.contains("Connected to my-vpn"));
            assert!(out.contains("enabled"));
        }

        #[test]
        fn test_connecting_status() {
            let state = VpnState::Connecting {
                server: "test".into(),
            };
            let out = format_status(&state, false);
            assert!(out.contains("Connecting to test"));
        }

        #[test]
        fn test_reconnecting_status() {
            let state = VpnState::Reconnecting {
                server: "vpn".into(),
                attempt: 3,
                max_attempts: 10,
            };
            let out = format_status(&state, false);
            assert!(out.contains("Reconnecting"));
            assert!(out.contains("3/10"));
        }

        #[test]
        fn test_degraded_status() {
            let state = VpnState::Degraded {
                server: "vpn".into(),
            };
            let out = format_status(&state, false);
            assert!(out.contains("Degraded"));
        }

        #[test]
        fn test_failed_status() {
            let state = VpnState::Failed {
                server: "vpn".into(),
                reason: "timeout".into(),
            };
            let out = format_status(&state, true);
            assert!(out.contains("Failed"));
            assert!(out.contains("timeout"));
            assert!(out.contains("enabled"));
        }
    }

    // ----- format_list -----

    mod format_list_tests {
        use super::*;

        #[test]
        fn test_empty_list() {
            let out = format_list(&[], None);
            assert!(out.contains("No VPN"));
        }

        #[test]
        fn test_list_with_vpns() {
            let vpns = vec!["vpn1".into(), "vpn2".into(), "vpn3".into()];
            let out = format_list(&vpns, None);
            assert!(out.contains("vpn1"));
            assert!(out.contains("vpn2"));
            assert!(out.contains("vpn3"));
        }

        #[test]
        fn test_list_with_active() {
            let vpns = vec!["vpn1".into(), "vpn2".into()];
            let out = format_list(&vpns, Some("vpn1"));
            assert!(out.contains("* vpn1 (active)"));
            assert!(out.contains("  vpn2"));
        }

        #[test]
        fn test_list_none_active_in_list() {
            let vpns = vec!["vpn1".into()];
            let out = format_list(&vpns, Some("other"));
            assert!(!out.contains("(active)"));
        }

        #[test]
        fn test_list_single_vpn() {
            let vpns = vec!["only-vpn".into()];
            let out = format_list(&vpns, None);
            assert_eq!(out, "  only-vpn");
        }
    }

    // ----- parse_ks_action -----

    mod parse_ks_action_tests {
        use super::*;

        #[test]
        fn test_enable_variants() {
            assert_eq!(parse_ks_action("on"), Ok(KsAction::Enable));
            assert_eq!(parse_ks_action("enable"), Ok(KsAction::Enable));
            assert_eq!(parse_ks_action("1"), Ok(KsAction::Enable));
            assert_eq!(parse_ks_action("true"), Ok(KsAction::Enable));
            assert_eq!(parse_ks_action("ON"), Ok(KsAction::Enable));
            assert_eq!(parse_ks_action("Enable"), Ok(KsAction::Enable));
        }

        #[test]
        fn test_disable_variants() {
            assert_eq!(parse_ks_action("off"), Ok(KsAction::Disable));
            assert_eq!(parse_ks_action("disable"), Ok(KsAction::Disable));
            assert_eq!(parse_ks_action("0"), Ok(KsAction::Disable));
            assert_eq!(parse_ks_action("false"), Ok(KsAction::Disable));
            assert_eq!(parse_ks_action("OFF"), Ok(KsAction::Disable));
        }

        #[test]
        fn test_status_variants() {
            assert_eq!(parse_ks_action("status"), Ok(KsAction::Status));
            assert_eq!(parse_ks_action("state"), Ok(KsAction::Status));
            assert_eq!(parse_ks_action("?"), Ok(KsAction::Status));
        }

        #[test]
        fn test_invalid_action() {
            assert!(parse_ks_action("invalid").is_err());
            assert!(parse_ks_action("").is_err());
            assert!(parse_ks_action("maybe").is_err());
        }
    }

    // ----- should_update_tray -----

    mod tray_update_tests {
        use super::*;

        #[test]
        fn test_same_state_no_update() {
            let s = VpnState::Disconnected;
            assert!(!should_update_tray(&s, &s));
        }

        #[test]
        fn test_different_state_updates() {
            let a = VpnState::Disconnected;
            let b = VpnState::Connecting {
                server: "vpn".into(),
            };
            assert!(should_update_tray(&a, &b));
        }

        #[test]
        fn test_same_variant_different_server_updates() {
            let a = VpnState::Connected {
                server: "vpn1".into(),
            };
            let b = VpnState::Connected {
                server: "vpn2".into(),
            };
            assert!(should_update_tray(&a, &b));
        }

        #[test]
        fn test_same_connected_same_server_no_update() {
            let a = VpnState::Connected {
                server: "vpn1".into(),
            };
            let b = VpnState::Connected {
                server: "vpn1".into(),
            };
            assert!(!should_update_tray(&a, &b));
        }
    }

    // ----- should_auto_reconnect -----

    mod auto_reconnect_tests {
        use super::*;

        #[test]
        fn test_disabled_never_reconnects() {
            let state = VpnState::Degraded {
                server: "vpn".into(),
            };
            assert!(!should_auto_reconnect(&state, false));
        }

        #[test]
        fn test_degraded_reconnects() {
            let state = VpnState::Degraded {
                server: "vpn".into(),
            };
            assert!(should_auto_reconnect(&state, true));
        }

        #[test]
        fn test_reconnecting_reconnects() {
            let state = VpnState::Reconnecting {
                server: "vpn".into(),
                attempt: 1,
                max_attempts: 10,
            };
            assert!(should_auto_reconnect(&state, true));
        }

        #[test]
        fn test_connected_does_not_reconnect() {
            let state = VpnState::Connected {
                server: "vpn".into(),
            };
            assert!(!should_auto_reconnect(&state, true));
        }

        #[test]
        fn test_disconnected_does_not_reconnect() {
            assert!(!should_auto_reconnect(&VpnState::Disconnected, true));
        }

        #[test]
        fn test_failed_does_not_reconnect() {
            let state = VpnState::Failed {
                server: "vpn".into(),
                reason: "err".into(),
            };
            assert!(!should_auto_reconnect(&state, true));
        }
    }
}
