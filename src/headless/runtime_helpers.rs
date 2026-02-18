// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Headless runtime helpers — pure functions, easily testable.
//!
//! Configuration validation, PID file handling, signal classification,
//! and runtime state management without any async I/O.

use std::time::Duration;

/// Runtime lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePhase {
    Starting,
    Running,
    Stopping,
    Stopped,
}

impl RuntimePhase {
    pub fn can_accept_commands(&self) -> bool {
        matches!(self, RuntimePhase::Running)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, RuntimePhase::Stopped)
    }
}

/// Signal classification for headless mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalAction {
    Shutdown,
    Reload,
    LogStatus,
    Ignore,
}

/// Map a Unix signal number to the action the daemon should take.
pub fn classify_signal(signal: i32) -> SignalAction {
    match signal {
        libc::SIGTERM | libc::SIGINT | libc::SIGQUIT => SignalAction::Shutdown,
        libc::SIGHUP => SignalAction::Reload,
        libc::SIGUSR1 => SignalAction::LogStatus,
        _ => SignalAction::Ignore,
    }
}

/// Format a PID as content suitable for a PID file.
pub fn format_pid(pid: u32) -> String {
    format!("{}\n", pid)
}

/// Parse a PID from PID-file content.
pub fn parse_pid(content: &str) -> Option<u32> {
    content.trim().parse().ok()
}

/// Validate a PID-file path.
pub fn validate_pid_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("PID file path cannot be empty".into());
    }
    if path.contains('\0') {
        return Err("PID file path contains null byte".into());
    }
    Ok(())
}

/// Default socket path (XDG-aware).
pub fn default_socket_path() -> String {
    std::env::var("XDG_RUNTIME_DIR")
        .map(|dir| format!("{}/shroud.sock", dir))
        .unwrap_or_else(|_| "/tmp/shroud.sock".into())
}

/// Default PID-file path (XDG-aware).
pub fn default_pid_path() -> String {
    std::env::var("XDG_RUNTIME_DIR")
        .map(|dir| format!("{}/shroud.pid", dir))
        .unwrap_or_else(|_| "/tmp/shroud.pid".into())
}

/// Parse `WATCHDOG_USEC` environment value into a `Duration`.
///
/// Systemd convention: ping at half the configured interval.
pub fn parse_watchdog_usec(usec_str: &str) -> Option<Duration> {
    let usec: u64 = usec_str.parse().ok()?;
    if usec == 0 {
        return None;
    }
    Some(Duration::from_micros(usec / 2))
}

/// Validate a runtime configuration bundle.
pub fn validate_runtime(
    socket_path: &str,
    auto_connect: Option<&str>,
    watchdog_enabled: bool,
    watchdog_interval: Duration,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if socket_path.is_empty() {
        errors.push("Socket path cannot be empty".into());
    }
    if let Some(vpn) = auto_connect {
        if vpn.is_empty() {
            errors.push("Auto-connect VPN name cannot be empty".into());
        }
    }
    if watchdog_enabled && watchdog_interval < Duration::from_secs(1) {
        errors.push("Watchdog interval must be at least 1 second".into());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod phase {
        use super::*;

        #[test]
        fn test_starting_cannot_accept() {
            assert!(!RuntimePhase::Starting.can_accept_commands());
        }

        #[test]
        fn test_running_can_accept() {
            assert!(RuntimePhase::Running.can_accept_commands());
        }

        #[test]
        fn test_stopping_cannot_accept() {
            assert!(!RuntimePhase::Stopping.can_accept_commands());
        }

        #[test]
        fn test_stopped_is_terminal() {
            assert!(RuntimePhase::Stopped.is_terminal());
        }

        #[test]
        fn test_running_is_not_terminal() {
            assert!(!RuntimePhase::Running.is_terminal());
        }
    }

    mod signals {
        use super::*;

        #[test]
        fn test_sigterm_shutdown() {
            assert_eq!(classify_signal(libc::SIGTERM), SignalAction::Shutdown);
        }

        #[test]
        fn test_sigint_shutdown() {
            assert_eq!(classify_signal(libc::SIGINT), SignalAction::Shutdown);
        }

        #[test]
        fn test_sigquit_shutdown() {
            assert_eq!(classify_signal(libc::SIGQUIT), SignalAction::Shutdown);
        }

        #[test]
        fn test_sighup_reload() {
            assert_eq!(classify_signal(libc::SIGHUP), SignalAction::Reload);
        }

        #[test]
        fn test_sigusr1_log_status() {
            assert_eq!(classify_signal(libc::SIGUSR1), SignalAction::LogStatus);
        }

        #[test]
        fn test_unknown_signal_ignore() {
            assert_eq!(classify_signal(99), SignalAction::Ignore);
        }
    }

    mod pid {
        use super::*;

        #[test]
        fn test_format_pid() {
            assert_eq!(format_pid(12345), "12345\n");
            assert_eq!(format_pid(1), "1\n");
        }

        #[test]
        fn test_parse_pid_valid() {
            assert_eq!(parse_pid("12345\n"), Some(12345));
            assert_eq!(parse_pid("  42  "), Some(42));
        }

        #[test]
        fn test_parse_pid_invalid() {
            assert_eq!(parse_pid(""), None);
            assert_eq!(parse_pid("abc"), None);
            assert_eq!(parse_pid("-1"), None);
        }

        #[test]
        fn test_roundtrip() {
            let content = format_pid(9999);
            assert_eq!(parse_pid(&content), Some(9999));
        }

        #[test]
        fn test_validate_pid_path_valid() {
            assert!(validate_pid_path("/tmp/test.pid").is_ok());
            assert!(validate_pid_path("/run/shroud.pid").is_ok());
        }

        #[test]
        fn test_validate_pid_path_empty() {
            assert!(validate_pid_path("").is_err());
        }

        #[test]
        fn test_validate_pid_path_null() {
            assert!(validate_pid_path("/tmp/\0bad").is_err());
        }
    }

    mod paths {
        use super::*;

        #[test]
        fn test_default_socket_path() {
            let path = default_socket_path();
            assert!(path.contains("shroud.sock"));
        }

        #[test]
        fn test_default_pid_path() {
            let path = default_pid_path();
            assert!(path.contains("shroud.pid"));
        }
    }

    mod watchdog {
        use super::*;

        #[test]
        fn test_parse_valid() {
            let d = parse_watchdog_usec("60000000").unwrap();
            assert_eq!(d, Duration::from_secs(30)); // half
        }

        #[test]
        fn test_parse_small() {
            let d = parse_watchdog_usec("2000000").unwrap();
            assert_eq!(d, Duration::from_secs(1));
        }

        #[test]
        fn test_parse_zero() {
            assert!(parse_watchdog_usec("0").is_none());
        }

        #[test]
        fn test_parse_invalid() {
            assert!(parse_watchdog_usec("abc").is_none());
            assert!(parse_watchdog_usec("").is_none());
        }
    }

    mod validate {
        use super::*;

        #[test]
        fn test_valid_config() {
            assert!(validate_runtime("/run/shroud.sock", None, false, Duration::ZERO).is_ok());
        }

        #[test]
        fn test_empty_socket() {
            let r = validate_runtime("", None, false, Duration::ZERO);
            assert!(r.is_err());
        }

        #[test]
        fn test_empty_auto_connect() {
            let r = validate_runtime("/x", Some(""), false, Duration::ZERO);
            assert!(r.is_err());
        }

        #[test]
        fn test_short_watchdog() {
            let r = validate_runtime("/x", None, true, Duration::from_millis(100));
            assert!(r.is_err());
        }

        #[test]
        fn test_multiple_errors() {
            let r = validate_runtime("", Some(""), true, Duration::from_millis(1));
            let errs = r.unwrap_err();
            assert!(errs.len() >= 2);
        }
    }
}
