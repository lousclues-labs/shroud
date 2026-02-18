// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Library re-exports for integration testing.
//!
//! Shroud is primarily a binary crate. This thin library target exposes
//! leaf modules (no deep dependency chains) so that `tests/` integration
//! tests can exercise real types (StateMachine, HealthChecker, etc.)
//! instead of relying on fragile `include_str!` assertions.

pub mod health;
pub mod state;
