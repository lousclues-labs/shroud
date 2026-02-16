// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! Health check module
//!
//! Provides connectivity verification for VPN tunnels to detect degraded states.

pub mod checker;

#[cfg(test)]
mod tests;

pub use checker::{HealthChecker, HealthResult};
