// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! NetworkManager module
//!
//! Provides the interface to NetworkManager for managing VPN connections.
//! Currently uses nmcli subprocess calls; future work will add D-Bus event subscription.

pub mod client;
pub mod connections;
#[cfg(test)]
pub mod mock;
pub mod parsing;
pub mod traits;

/// Get the nmcli command path (centralized for all NM modules).
///
/// Supports `SHROUD_NMCLI` env override for non-standard installations (NixOS,
/// custom prefix) and for test mocking.
///
/// # Security
///
/// This env var is trusted because the daemon's environment is set at
/// launch time by the owning user. IPC clients cannot influence it.
/// If the user's session is compromised, `SHROUD_NMCLI` is the least
/// of their problems.
pub(crate) fn nmcli_command() -> String {
    if let Ok(path) = std::env::var("SHROUD_NMCLI") {
        return path;
    }
    "nmcli".to_string()
}

// Re-exports used by headless runtime (nm::connect, nm::get_active_vpn)
pub use client::{connect, get_active_vpn};
// Re-exports used by supervisor handlers
pub use connections::{get_vpn_type, list_vpn_connections_with_types};
#[cfg(test)]
pub use mock::{MockNmClient, NmCall};
#[cfg(test)]
pub use traits::NmError;
pub use traits::{NmCliClient, NmClient};
