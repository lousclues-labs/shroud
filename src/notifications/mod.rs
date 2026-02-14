//! Desktop notification system
//!
//! Provides categorized, configurable notifications for VPN events
//! with throttling, deduplication, and per-category enable/disable.

pub mod manager;
pub mod types;

pub use manager::NotificationManager;
pub use types::{Notification, NotificationCategory};
