// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Headless mode runtime for Shroud.
//!
//! This module provides server-optimized operation:
//! - No tray icon or desktop notifications
//! - Systemd integration (notify, watchdog)
//! - Journald logging
//! - Auto-connect on startup
//! - Infinite reconnection attempts

#[cfg(test)]
pub mod config; // Test-only config parsing helpers
pub mod runtime;
#[cfg(test)]
pub mod runtime_helpers; // Test-only runtime helpers
pub mod systemd;

#[cfg(test)]
mod tests;

pub use runtime::run_headless;

// Re-export systemd functions for external use
#[allow(unused_imports)]
pub use systemd::{notify_ready, notify_status, notify_stopping, notify_watchdog};
