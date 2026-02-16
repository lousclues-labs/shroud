// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! Systemd integration for headless mode.
//!
//! Implements the systemd service notification protocol:
//! - READY=1: Service is ready
//! - STOPPING=1: Service is stopping
//! - STATUS=...: Human-readable status
//! - WATCHDOG=1: Watchdog keep-alive
//!
//! Reference: <https://www.freedesktop.org/software/systemd/man/sd_notify.html>

use std::env;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, trace};

/// Get the systemd notify socket path from environment.
fn notify_socket() -> Option<PathBuf> {
    env::var("NOTIFY_SOCKET").ok().map(PathBuf::from)
}

/// Send a notification to systemd.
fn notify(message: &str) -> bool {
    let socket_path = match notify_socket() {
        Some(path) => path,
        None => {
            trace!("No NOTIFY_SOCKET, skipping sd_notify");
            return false;
        }
    };

    let socket = match UnixDatagram::unbound() {
        Ok(s) => s,
        Err(e) => {
            debug!("Failed to create notify socket: {}", e);
            return false;
        }
    };

    match socket.send_to(message.as_bytes(), &socket_path) {
        Ok(_) => {
            trace!("sd_notify: {}", message);
            true
        }
        Err(e) => {
            debug!("Failed to send to notify socket: {}", e);
            false
        }
    }
}

/// Notify systemd that the service is ready.
///
/// Call this after all initialization is complete.
pub fn notify_ready() {
    notify("READY=1");
}

/// Notify systemd that the service is stopping.
///
/// Call this at the beginning of shutdown.
pub fn notify_stopping() {
    notify("STOPPING=1");
}

/// Update the human-readable status.
///
/// This appears in `systemctl status shroud`.
pub fn notify_status(status: &str) {
    notify(&format!("STATUS={}", status));
}

/// Send watchdog keep-alive.
///
/// Must be called regularly if WatchdogSec is configured.
pub fn notify_watchdog() {
    notify("WATCHDOG=1");
}

/// Notify that the service is reloading configuration.
#[allow(dead_code)]
pub fn notify_reloading() {
    notify("RELOADING=1");
}

/// Get the watchdog interval from environment.
///
/// Returns None if watchdog is not configured.
pub fn watchdog_interval() -> Option<Duration> {
    let usec: u64 = env::var("WATCHDOG_USEC").ok()?.parse().ok()?;
    Some(Duration::from_micros(usec))
}

/// Check if running under systemd.
#[allow(dead_code)]
pub fn is_systemd_service() -> bool {
    notify_socket().is_some() || env::var("INVOCATION_ID").is_ok()
}

/// Get the systemd invocation ID.
#[allow(dead_code)]
pub fn invocation_id() -> Option<String> {
    env::var("INVOCATION_ID").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_socket_returns_false() {
        // When no NOTIFY_SOCKET is set, notify should return false
        // (assuming test environment doesn't have it)
        env::remove_var("NOTIFY_SOCKET");
        assert!(!notify("TEST=1"));
    }

    #[test]
    fn test_watchdog_interval_none() {
        env::remove_var("WATCHDOG_USEC");
        assert!(watchdog_interval().is_none());
    }
}
