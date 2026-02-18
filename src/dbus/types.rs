// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! D-Bus type conversions and state mapping — pure functions, easily testable.
//!
//! Provides NM state enums, D-Bus path parsing utilities, and
//! signal classification without requiring a live D-Bus connection.

/// NetworkManager VPN connection state codes.
///
/// Reference: <https://networkmanager.dev/docs/api/latest/nm-dbus-types.html>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NmVpnState {
    Unknown,
    Prepare,
    NeedAuth,
    Connect,
    IpConfigGet,
    Activated,
    Failed,
    Disconnected,
}

impl NmVpnState {
    /// Parse from the u32 value sent over D-Bus.
    pub fn from_u32(value: u32) -> Self {
        match value {
            1 => NmVpnState::Prepare,
            2 => NmVpnState::NeedAuth,
            3 => NmVpnState::Connect,
            4 => NmVpnState::IpConfigGet,
            5 => NmVpnState::Activated,
            6 => NmVpnState::Failed,
            7 => NmVpnState::Disconnected,
            _ => NmVpnState::Unknown,
        }
    }

    pub fn is_activating(&self) -> bool {
        matches!(
            self,
            NmVpnState::Prepare
                | NmVpnState::NeedAuth
                | NmVpnState::Connect
                | NmVpnState::IpConfigGet
        )
    }

    pub fn is_active(&self) -> bool {
        matches!(self, NmVpnState::Activated)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, NmVpnState::Failed)
    }

    pub fn is_disconnected(&self) -> bool {
        matches!(self, NmVpnState::Disconnected | NmVpnState::Unknown)
    }

    pub fn description(&self) -> &'static str {
        match self {
            NmVpnState::Unknown => "Unknown",
            NmVpnState::Prepare => "Preparing",
            NmVpnState::NeedAuth => "Authenticating",
            NmVpnState::Connect => "Connecting",
            NmVpnState::IpConfigGet => "Getting IP",
            NmVpnState::Activated => "Active",
            NmVpnState::Failed => "Failed",
            NmVpnState::Disconnected => "Disconnected",
        }
    }
}

/// NetworkManager active-connection state codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NmActiveState {
    Unknown,
    Activating,
    Activated,
    Deactivating,
    Deactivated,
}

impl NmActiveState {
    pub fn from_u32(value: u32) -> Self {
        match value {
            0 => NmActiveState::Unknown,
            1 => NmActiveState::Activating,
            2 => NmActiveState::Activated,
            3 => NmActiveState::Deactivating,
            4 => NmActiveState::Deactivated,
            _ => NmActiveState::Unknown,
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self, NmActiveState::Activated)
    }

    pub fn is_transitioning(&self) -> bool {
        matches!(
            self,
            NmActiveState::Activating | NmActiveState::Deactivating
        )
    }
}

/// NetworkManager device state codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NmDeviceState {
    Unknown,
    Unmanaged,
    Unavailable,
    Disconnected,
    Prepare,
    Config,
    NeedAuth,
    IpConfig,
    IpCheck,
    Secondaries,
    Activated,
    Deactivating,
    Failed,
}

impl NmDeviceState {
    pub fn from_u32(value: u32) -> Self {
        match value {
            0 => NmDeviceState::Unknown,
            10 => NmDeviceState::Unmanaged,
            20 => NmDeviceState::Unavailable,
            30 => NmDeviceState::Disconnected,
            40 => NmDeviceState::Prepare,
            50 => NmDeviceState::Config,
            60 => NmDeviceState::NeedAuth,
            70 => NmDeviceState::IpConfig,
            80 => NmDeviceState::IpCheck,
            90 => NmDeviceState::Secondaries,
            100 => NmDeviceState::Activated,
            110 => NmDeviceState::Deactivating,
            120 => NmDeviceState::Failed,
            _ => NmDeviceState::Unknown,
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self, NmDeviceState::Activated)
    }
}

// ---------- D-Bus path utilities ----------

/// Extract the last segment from a D-Bus object path.
///
/// e.g. `/org/freedesktop/NetworkManager/ActiveConnection/123` → `"123"`
pub fn parse_path_id(path: &str) -> Option<&str> {
    path.rsplit('/').next()
}

/// Build a settings connection path from a UUID.
pub fn build_settings_path(uuid: &str) -> String {
    format!("/org/freedesktop/NetworkManager/Settings/{}", uuid)
}

/// Build an active-connection path from an id.
pub fn build_active_path(id: &str) -> String {
    format!("/org/freedesktop/NetworkManager/ActiveConnection/{}", id)
}

/// Classify a NM connection type string.
pub fn is_vpn_type(conn_type: &str) -> bool {
    matches!(conn_type, "vpn" | "wireguard")
}

/// Convert a VPN failure reason code to a human-readable string.
pub fn vpn_failure_reason(reason: u32) -> &'static str {
    match reason {
        0 => "Unknown",
        1 => "Not provided",
        2 => "User disconnected",
        3 => "Service stopped",
        4 => "IP config invalid",
        5 => "Connect timeout",
        6 => "Service start timeout",
        7 => "Service start failed",
        8 => "No secrets",
        9 => "Login failed",
        10 => "Connection removed",
        _ => "Unknown reason",
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod nm_vpn_state {
        use super::*;

        #[test]
        fn test_from_u32_known() {
            assert_eq!(NmVpnState::from_u32(1), NmVpnState::Prepare);
            assert_eq!(NmVpnState::from_u32(2), NmVpnState::NeedAuth);
            assert_eq!(NmVpnState::from_u32(3), NmVpnState::Connect);
            assert_eq!(NmVpnState::from_u32(4), NmVpnState::IpConfigGet);
            assert_eq!(NmVpnState::from_u32(5), NmVpnState::Activated);
            assert_eq!(NmVpnState::from_u32(6), NmVpnState::Failed);
            assert_eq!(NmVpnState::from_u32(7), NmVpnState::Disconnected);
        }

        #[test]
        fn test_from_u32_unknown() {
            assert_eq!(NmVpnState::from_u32(0), NmVpnState::Unknown);
            assert_eq!(NmVpnState::from_u32(99), NmVpnState::Unknown);
            assert_eq!(NmVpnState::from_u32(u32::MAX), NmVpnState::Unknown);
        }

        #[test]
        fn test_is_activating() {
            assert!(NmVpnState::Prepare.is_activating());
            assert!(NmVpnState::NeedAuth.is_activating());
            assert!(NmVpnState::Connect.is_activating());
            assert!(NmVpnState::IpConfigGet.is_activating());
            assert!(!NmVpnState::Activated.is_activating());
            assert!(!NmVpnState::Failed.is_activating());
            assert!(!NmVpnState::Disconnected.is_activating());
        }

        #[test]
        fn test_is_active() {
            assert!(NmVpnState::Activated.is_active());
            assert!(!NmVpnState::Connect.is_active());
            assert!(!NmVpnState::Failed.is_active());
            assert!(!NmVpnState::Unknown.is_active());
        }

        #[test]
        fn test_is_failed() {
            assert!(NmVpnState::Failed.is_failed());
            assert!(!NmVpnState::Activated.is_failed());
            assert!(!NmVpnState::Unknown.is_failed());
        }

        #[test]
        fn test_is_disconnected() {
            assert!(NmVpnState::Disconnected.is_disconnected());
            assert!(NmVpnState::Unknown.is_disconnected());
            assert!(!NmVpnState::Activated.is_disconnected());
            assert!(!NmVpnState::Failed.is_disconnected());
        }

        #[test]
        fn test_descriptions() {
            assert_eq!(NmVpnState::Activated.description(), "Active");
            assert_eq!(NmVpnState::Connect.description(), "Connecting");
            assert_eq!(NmVpnState::Failed.description(), "Failed");
            assert_eq!(NmVpnState::Disconnected.description(), "Disconnected");
            assert_eq!(NmVpnState::Unknown.description(), "Unknown");
            assert_eq!(NmVpnState::Prepare.description(), "Preparing");
            assert_eq!(NmVpnState::NeedAuth.description(), "Authenticating");
            assert_eq!(NmVpnState::IpConfigGet.description(), "Getting IP");
        }
    }

    mod nm_active_state {
        use super::*;

        #[test]
        fn test_from_u32() {
            assert_eq!(NmActiveState::from_u32(0), NmActiveState::Unknown);
            assert_eq!(NmActiveState::from_u32(1), NmActiveState::Activating);
            assert_eq!(NmActiveState::from_u32(2), NmActiveState::Activated);
            assert_eq!(NmActiveState::from_u32(3), NmActiveState::Deactivating);
            assert_eq!(NmActiveState::from_u32(4), NmActiveState::Deactivated);
            assert_eq!(NmActiveState::from_u32(99), NmActiveState::Unknown);
        }

        #[test]
        fn test_is_connected() {
            assert!(NmActiveState::Activated.is_connected());
            assert!(!NmActiveState::Activating.is_connected());
            assert!(!NmActiveState::Deactivated.is_connected());
        }

        #[test]
        fn test_is_transitioning() {
            assert!(NmActiveState::Activating.is_transitioning());
            assert!(NmActiveState::Deactivating.is_transitioning());
            assert!(!NmActiveState::Activated.is_transitioning());
            assert!(!NmActiveState::Deactivated.is_transitioning());
        }
    }

    mod nm_device_state {
        use super::*;

        #[test]
        fn test_from_u32() {
            assert_eq!(NmDeviceState::from_u32(0), NmDeviceState::Unknown);
            assert_eq!(NmDeviceState::from_u32(30), NmDeviceState::Disconnected);
            assert_eq!(NmDeviceState::from_u32(100), NmDeviceState::Activated);
            assert_eq!(NmDeviceState::from_u32(120), NmDeviceState::Failed);
            assert_eq!(NmDeviceState::from_u32(10), NmDeviceState::Unmanaged);
            assert_eq!(NmDeviceState::from_u32(20), NmDeviceState::Unavailable);
            assert_eq!(NmDeviceState::from_u32(999), NmDeviceState::Unknown);
        }

        #[test]
        fn test_is_connected() {
            assert!(NmDeviceState::Activated.is_connected());
            assert!(!NmDeviceState::Disconnected.is_connected());
            assert!(!NmDeviceState::Failed.is_connected());
        }
    }

    mod path_parsing {
        use super::*;

        #[test]
        fn test_parse_path_id() {
            let path = "/org/freedesktop/NetworkManager/ActiveConnection/123";
            assert_eq!(parse_path_id(path), Some("123"));
        }

        #[test]
        fn test_parse_path_id_simple() {
            assert_eq!(parse_path_id("/a/b/c"), Some("c"));
        }

        #[test]
        fn test_parse_path_id_root() {
            assert_eq!(parse_path_id("/"), Some(""));
        }

        #[test]
        fn test_parse_path_id_no_slash() {
            assert_eq!(parse_path_id("single"), Some("single"));
        }

        #[test]
        fn test_build_settings_path() {
            let path = build_settings_path("abc-123");
            assert_eq!(path, "/org/freedesktop/NetworkManager/Settings/abc-123");
        }

        #[test]
        fn test_build_active_path() {
            let path = build_active_path("42");
            assert_eq!(path, "/org/freedesktop/NetworkManager/ActiveConnection/42");
        }
    }

    mod type_detection {
        use super::*;

        #[test]
        fn test_vpn_types() {
            assert!(is_vpn_type("vpn"));
            assert!(is_vpn_type("wireguard"));
        }

        #[test]
        fn test_non_vpn_types() {
            assert!(!is_vpn_type("802-11-wireless"));
            assert!(!is_vpn_type("802-3-ethernet"));
            assert!(!is_vpn_type("bridge"));
            assert!(!is_vpn_type(""));
        }
    }

    mod failure_reasons {
        use super::*;

        #[test]
        fn test_known_reasons() {
            assert_eq!(vpn_failure_reason(0), "Unknown");
            assert_eq!(vpn_failure_reason(5), "Connect timeout");
            assert_eq!(vpn_failure_reason(9), "Login failed");
            assert_eq!(vpn_failure_reason(10), "Connection removed");
        }

        #[test]
        fn test_unknown_reason() {
            assert_eq!(vpn_failure_reason(99), "Unknown reason");
            assert_eq!(vpn_failure_reason(u32::MAX), "Unknown reason");
        }

        #[test]
        fn test_all_known_reasons_non_empty() {
            for code in 0..=10 {
                let desc = vpn_failure_reason(code);
                assert!(!desc.is_empty(), "Reason for code {} is empty", code);
            }
        }
    }
}
