// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Test context with mocked system dependencies

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::sync::mpsc;

/// Test context providing isolated environment and mocks
pub struct TestContext {
    /// Temporary directory for test files
    pub temp_dir: TempDir,
    /// Mock NetworkManager connections
    pub connections: Arc<Mutex<HashMap<String, VpnConnection>>>,
    /// Mock command executor
    pub executor: Arc<MockExecutor>,
    /// Event channel for testing
    pub events: EventChannel,
    /// Whether running with privileges
    pub privileged: bool,
}

#[derive(Clone, Debug)]
pub struct VpnConnection {
    pub name: String,
    pub uuid: String,
    pub active: bool,
    pub conn_type: String,
}

pub struct EventChannel {
    pub tx: mpsc::Sender<String>,
    pub rx: Arc<Mutex<mpsc::Receiver<String>>>,
}

impl TestContext {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(100);
        Self {
            temp_dir: TempDir::new().unwrap(),
            connections: Arc::new(Mutex::new(HashMap::new())),
            executor: Arc::new(MockExecutor::new()),
            events: EventChannel {
                tx,
                rx: Arc::new(Mutex::new(rx)),
            },
            privileged: nix::unistd::geteuid().is_root(),
        }
    }

    pub fn with_vpns(names: &[&str]) -> Self {
        let ctx = Self::new();
        for name in names {
            ctx.add_vpn(name);
        }
        ctx
    }

    pub fn add_vpn(&self, name: &str) {
        let mut conns = self.connections.lock().unwrap();
        conns.insert(
            name.to_string(),
            VpnConnection {
                name: name.to_string(),
                uuid: format!("uuid-{}", name),
                active: false,
                conn_type: "vpn".to_string(),
            },
        );
    }

    pub fn activate_vpn(&self, name: &str) -> Result<(), String> {
        let mut conns = self.connections.lock().unwrap();
        if conns.contains_key(name) {
            // Deactivate others first
            for c in conns.values_mut() {
                c.active = false;
            }
            conns.get_mut(name).unwrap().active = true;
            Ok(())
        } else {
            Err(format!("VPN '{}' not found", name))
        }
    }

    pub fn active_vpn(&self) -> Option<String> {
        self.connections
            .lock()
            .unwrap()
            .values()
            .find(|c| c.active)
            .map(|c| c.name.clone())
    }

    pub fn config_dir(&self) -> PathBuf {
        let dir = self.temp_dir.path().join("config");
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    pub fn socket_path(&self) -> PathBuf {
        self.temp_dir.path().join("shroud.sock")
    }

    /// Skip test if not running as root
    pub fn require_root(&self) {
        if !self.privileged {
            println!("SKIPPED: requires root");
            std::process::exit(77); // Standard skip exit code
        }
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Mock command executor for testing without system calls
pub struct MockExecutor {
    commands: Mutex<Vec<String>>,
    results: Mutex<HashMap<String, Result<String, String>>>,
}

impl MockExecutor {
    pub fn new() -> Self {
        Self {
            commands: Mutex::new(Vec::new()),
            results: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_result(&self, pattern: &str, result: Result<String, String>) {
        self.results
            .lock()
            .unwrap()
            .insert(pattern.to_string(), result);
    }

    pub fn execute(&self, cmd: &str) -> Result<String, String> {
        self.commands.lock().unwrap().push(cmd.to_string());

        let results = self.results.lock().unwrap();
        for (pattern, result) in results.iter() {
            if cmd.contains(pattern) {
                return result.clone();
            }
        }
        Ok(String::new())
    }

    pub fn commands(&self) -> Vec<String> {
        self.commands.lock().unwrap().clone()
    }

    pub fn was_called(&self, pattern: &str) -> bool {
        self.commands
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.contains(pattern))
    }

    pub fn clear(&self) {
        self.commands.lock().unwrap().clear();
    }
}

impl Default for MockExecutor {
    fn default() -> Self {
        Self::new()
    }
}
