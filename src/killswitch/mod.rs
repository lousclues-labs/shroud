// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! Kill switch module
//!
//! Provides VPN kill switch functionality using iptables.
//! When enabled, blocks all traffic except through the VPN tunnel.

pub mod boot;
pub mod cleanup;
pub mod cleanup_logic;
pub mod firewall;
pub mod paths;
pub mod rules;
pub mod sudo_check;
pub mod verify;

#[cfg(test)]
mod tests;

pub use cleanup::{cleanup_stale_on_startup, cleanup_with_fallback, CleanupResult};
pub use firewall::{KillSwitch, KillSwitchError};
pub use sudo_check::validate_sudoers_on_startup;
