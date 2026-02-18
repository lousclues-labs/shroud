// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Integration Test Suite
//!
//! These tests verify module interactions work correctly together.
//! They run as Rust tests and don't require spawning the full binary.
//!
//! # Running Integration Tests
//!
//! ```bash
//! # Run all integration tests
//! cargo test --test integration
//!
//! # Run specific category
//! cargo test --test integration config
//! cargo test --test integration state
//! cargo test --test integration tray
//! ```
//!
//! Tests marked with `#[ignore]` require the daemon to be running
//! or other specific setup.

mod common;

// Core integration tests (no daemon required)
mod config_tests;
mod state_machine_tests;
mod tray_channel_tests;

// Integration tests that may require daemon
mod cli_tests;
mod daemon_tests;
mod import_tests;
mod validation_tests;
