// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Security Test Suite
//!
//! These tests verify the security properties of VPNShroud:
//! - Signal handling during critical operations
//! - Privilege escalation prevention
//! - Resource exhaustion handling
//! - Crash recovery
//! - Race condition safety
//! - IPC socket security
//! - D-Bus security
//! - Configuration security
//! - VPN leak prevention
//!
//! # Running Security Tests
//!
//! Most security tests require privileged access and are marked with `#[ignore]`.
//!
//! ```bash
//! # Run all non-privileged security tests
//! cargo test --test security
//!
//! # Run all security tests (requires sudo)
//! sudo -E cargo test --test security -- --ignored --nocapture
//!
//! # Run specific category
//! sudo -E cargo test --test security signal -- --ignored
//! sudo -E cargo test --test security privilege -- --ignored
//! sudo -E cargo test --test security race -- --ignored
//! ```
//!
//! # Requirements for Privileged Tests
//!
//! - Root/sudo access for iptables manipulation
//! - NetworkManager running with VPN connections configured
//! - D-Bus session available
//! - iptables/nftables installed

mod common;

mod config_tests;
mod crash_tests;
mod dbus_tests;
mod ipc_tests;
mod leak_tests;
mod privilege_tests;
mod race_tests;
mod resource_tests;
mod signal_tests;
