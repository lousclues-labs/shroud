// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Common test utilities for all VPNShroud tests
//!
//! This module provides:
//! - Test context management
//! - Mock implementations (NetworkManager, D-Bus, command executor)
//! - Assertions for system state
//! - Test fixtures and data generators

#![allow(dead_code)]
#![allow(unused_imports)]

// Core test utilities
pub mod assertions;
pub mod context;
pub mod results;
pub mod system;

// Mock implementations for integration testing
pub mod fixtures;
pub mod mock_dbus;
pub mod mock_executor;
pub mod mock_nm;

// Re-export core utilities
pub use assertions::*;
pub use context::*;
pub use results::*;
pub use system::*;

// Re-export mocks
pub use fixtures::{TestConfig, TestEnv};
pub use mock_dbus::{MockConnectivity, MockDbus, MockDbusEvent, MockVpnState};
pub use mock_executor::{MockCommand, MockExecutor, MockResult};
pub use mock_nm::{MockNetworkManager, MockNmCall, MockNmError, MockVpnConnection, MockVpnType};

use std::path::PathBuf;
use std::sync::Once;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;

static INIT: Once = Once::new();

/// Initialize test environment (called once per test run)
pub fn init() {
    INIT.call_once(|| {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
        let _ = fmt()
            .with_env_filter(filter)
            .with_test_writer()
            .try_init();
    });
}

/// Get path to project root
pub fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Get path to test results directory
pub fn results_dir() -> PathBuf {
    let dir = project_root().join("target").join("test-results");
    std::fs::create_dir_all(&dir).ok();
    dir
}
