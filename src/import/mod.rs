// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! VPN config import functionality
//!
//! Supports importing WireGuard (.conf) and OpenVPN (.ovpn) config files
//! into NetworkManager.

pub mod detector;
pub mod importer;
pub mod types;
pub mod validator;

#[allow(unused_imports)]
pub use detector::{detect_config_type, VpnConfigType};
#[allow(unused_imports)]
pub use importer::{
    check_networkmanager, import_directory, import_file, ImportError, ImportResult, ImportSummary,
};
#[allow(unused_imports)]
pub use types::ImportOptions;
#[allow(unused_imports)]
pub use validator::{validate, ValidationError};
