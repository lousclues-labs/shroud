// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! Daemon utilities
//!
//! Provides utilities for running the Shroud daemon, including
//! instance locking to prevent multiple daemons from running.

pub mod lock;

pub use lock::{acquire_instance_lock, release_instance_lock};
