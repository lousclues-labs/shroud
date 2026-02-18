// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Desktop notification system
//!
//! Provides categorized, configurable notifications for VPN events
//! with throttling, deduplication, and per-category enable/disable.

pub mod manager;
pub mod types;

pub use manager::NotificationManager;
pub use types::{Notification, NotificationCategory};
