//! Headless mode runtime for Shroud.
//!
//! This module provides server-optimized operation:
//! - No tray icon or desktop notifications
//! - Systemd integration (notify, watchdog)
//! - Journald logging
//! - Auto-connect on startup
//! - Infinite reconnection attempts

#[allow(dead_code)]
pub mod config;
pub mod runtime;
pub mod systemd;

#[cfg(test)]
mod tests;

pub use runtime::run_headless;

// Re-export systemd functions for external use
#[allow(unused_imports)]
pub use systemd::{notify_ready, notify_status, notify_stopping, notify_watchdog};
