// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Configuration module
//!
//! Provides persistent configuration storage for user preferences.

pub mod settings;

pub use settings::{Config, ConfigManager, DnsMode, HeadlessConfig, Ipv6Mode};
