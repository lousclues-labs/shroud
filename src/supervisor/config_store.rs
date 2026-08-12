// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

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

    /// Config store backed by a throwaway file, for tests.
    ///
    /// Supervisor tests drive the real handlers, which persist config; without
    /// this they rewrite the config of whoever runs the suite.
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};

        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "shroud-test-{}-{}.toml",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );

        Self {
            manager: ConfigManager::with_path(std::env::temp_dir().join(unique)),
            config: Config::default(),
            is_first_run: true,
        }
    }

    /// Reload config from disk (e.g., after SIGHUP or IPC reload command).
    pub(crate) fn reload(&mut self) -> Config {
        self.config = self.manager.load_validated();
        info!("Configuration reloaded from disk");
        self.config.clone()
    }
}
