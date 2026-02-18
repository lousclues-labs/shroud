// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

use serde::Serialize;
use std::path::PathBuf;

use super::detector::VpnConfigType;

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub path: PathBuf,
    pub name: Option<String>,
    pub connect: bool,
    pub force: bool,
    pub recursive: bool,
    pub dry_run: bool,
    pub config_type: Option<VpnConfigType>,
    pub quiet: bool,
    pub json: bool,
}

#[derive(Debug, Serialize)]
pub struct ImportResultJson {
    pub success: bool,
    pub imported: Vec<ImportedConnection>,
    pub skipped: Vec<SkippedFile>,
    pub failed: Vec<FailedFile>,
    pub summary: ImportSummaryJson,
}

#[derive(Debug, Serialize)]
pub struct ImportedConnection {
    pub name: String,
    #[serde(rename = "type")]
    pub config_type: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct FailedFile {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct ImportSummaryJson {
    pub total: usize,
    pub imported: usize,
    pub skipped: usize,
    pub failed: usize,
}
