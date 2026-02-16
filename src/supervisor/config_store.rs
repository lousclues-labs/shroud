// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

use tracing::{info, warn};

use crate::config::{Config, ConfigManager};

/// Persistent configuration storage for the supervisor.
///
/// Wraps `ConfigManager` (file I/O) and the live `Config` (in-memory).
/// All config mutations should go through this struct to ensure
/// changes are persisted atomically.
pub(crate) struct ConfigStore {
    /// Handles config file read/write (atomic save via temp+rename)
    manager: ConfigManager,
    /// Current in-memory configuration
    pub(crate) config: Config,
    /// Whether this is the first run (no pre-existing config file)
    pub(crate) is_first_run: bool,
}

impl ConfigStore {
    pub(crate) fn load() -> Self {
        let manager = ConfigManager::new();
        let is_first_run = !manager.config_path().exists();
        let config = manager.load_validated();
        info!(
            "Loaded config: auto_reconnect={}, last_server={:?}",
            config.auto_reconnect, config.last_server
        );
        Self {
            manager,
            config,
            is_first_run,
        }
    }

    /// Save the current config to disk (atomic write).
    pub(crate) fn save(&self) {
        if let Err(e) = self.manager.save(&self.config) {
            warn!("Failed to save config: {}", e);
        }
    }

    /// Reload config from disk (e.g., after SIGHUP or IPC reload command).
    pub(crate) fn reload(&mut self) -> Config {
        self.config = self.manager.load_validated();
        info!("Configuration reloaded from disk");
        self.config.clone()
    }
}
