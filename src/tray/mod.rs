// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! System tray module
//!
//! Provides the system tray UI for the VPN manager.

pub mod icons;
pub mod service;

#[cfg(test)]
mod tests;

pub use service::{SharedState, VpnCommand, VpnTray};
