// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! Mock NmClient for behavioral testing.
//!
//! Bridges the existing MockNetworkManager patterns into the NmClient trait so
//! the supervisor can be tested without subprocess calls.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::state::{ActiveVpnInfo, NmVpnState};

use super::client::NmError;
use super::traits::NmClient;

/// Record of calls made to the mock (for test assertions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NmCall {
    ListConnections,
    GetActiveVpn,
    GetActiveVpnWithState,
    GetAllActiveVpns,
    GetVpnState(String),
    Connect(String),
    Disconnect(String),
    KillOrphans,
}

/// Mock NM client that tracks calls and returns configurable responses.
#[allow(clippy::type_complexity)]
#[derive(Clone)]
pub struct MockNmClient {
    /// VPN connections: name → (vpn_type_str, is_active, nm_state)
    connections: Arc<Mutex<HashMap<String, (String, bool, NmVpnState)>>>,
    /// Name of the currently active VPN
    active_vpn: Arc<Mutex<Option<String>>>,
    /// Queue of errors to inject (FIFO — next connect/disconnect pops one)
    error_queue: Arc<Mutex<Vec<NmError>>>,
    /// Log of all calls made
    calls: Arc<Mutex<Vec<NmCall>>>,
}

impl MockNmClient {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
            active_vpn: Arc::new(Mutex::new(None)),
            error_queue: Arc::new(Mutex::new(Vec::new())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Add a VPN that starts inactive.
    pub fn add_vpn(&self, name: &str) {
        self.connections.lock().unwrap().insert(
            name.to_string(),
            ("openvpn".to_string(), false, NmVpnState::Inactive),
        );
    }

    /// Pre-set a VPN as already active.
    pub fn set_active(&self, name: &str) {
        if let Some(entry) = self.connections.lock().unwrap().get_mut(name) {
            entry.1 = true;
            entry.2 = NmVpnState::Activated;
        }
        *self.active_vpn.lock().unwrap() = Some(name.to_string());
    }

    /// Queue an error that the next connect() or disconnect() call will return.
    pub fn queue_error(&self, err: NmError) {
        self.error_queue.lock().unwrap().push(err);
    }

    /// Get all calls made (for assertions).
    #[allow(dead_code)]
    pub fn calls(&self) -> Vec<NmCall> {
        self.calls.lock().unwrap().clone()
    }

    /// Check if a specific call was made.
    pub fn was_called(&self, call: &NmCall) -> bool {
        self.calls.lock().unwrap().contains(call)
    }

    /// Count how many times connect() was called.
    pub fn connect_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| matches!(c, NmCall::Connect(_)))
            .count()
    }

    fn pop_error(&self) -> Option<NmError> {
        let mut q = self.error_queue.lock().unwrap();
        if q.is_empty() {
            None
        } else {
            Some(q.remove(0))
        }
    }

    fn log(&self, call: NmCall) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait]
impl NmClient for MockNmClient {
    async fn list_vpn_connections(&self) -> Vec<String> {
        self.log(NmCall::ListConnections);
        self.connections.lock().unwrap().keys().cloned().collect()
    }

    async fn get_active_vpn(&self) -> Option<String> {
        self.log(NmCall::GetActiveVpn);
        self.active_vpn.lock().unwrap().clone()
    }

    async fn get_active_vpn_with_state(&self) -> Option<ActiveVpnInfo> {
        self.log(NmCall::GetActiveVpnWithState);
        let active = self.active_vpn.lock().unwrap().clone()?;
        let conns = self.connections.lock().unwrap();
        let (_, _, state) = conns.get(&active)?;
        Some(ActiveVpnInfo {
            name: active,
            state: *state,
        })
    }

    async fn get_all_active_vpns(&self) -> Vec<ActiveVpnInfo> {
        self.log(NmCall::GetAllActiveVpns);
        self.connections
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, (_, active, _))| *active)
            .map(|(name, (_, _, state))| ActiveVpnInfo {
                name: name.clone(),
                state: *state,
            })
            .collect()
    }

    async fn get_vpn_state(&self, name: &str) -> Option<NmVpnState> {
        self.log(NmCall::GetVpnState(name.to_string()));
        self.connections
            .lock()
            .unwrap()
            .get(name)
            .map(|(_, _, state)| *state)
    }

    async fn connect(&self, name: &str) -> Result<(), NmError> {
        self.log(NmCall::Connect(name.to_string()));
        if let Some(err) = self.pop_error() {
            return Err(err);
        }
        // Deactivate all, activate target
        let mut conns = self.connections.lock().unwrap();
        for (_, entry) in conns.iter_mut() {
            entry.1 = false;
            entry.2 = NmVpnState::Inactive;
        }
        if let Some(entry) = conns.get_mut(name) {
            entry.1 = true;
            entry.2 = NmVpnState::Activated;
            *self.active_vpn.lock().unwrap() = Some(name.to_string());
            Ok(())
        } else {
            Err(NmError::Command(format!("VPN '{}' not found", name)))
        }
    }

    async fn disconnect(&self, name: &str) -> Result<(), NmError> {
        self.log(NmCall::Disconnect(name.to_string()));
        if let Some(err) = self.pop_error() {
            return Err(err);
        }
        let mut conns = self.connections.lock().unwrap();
        if let Some(entry) = conns.get_mut(name) {
            entry.1 = false;
            entry.2 = NmVpnState::Inactive;
        }
        let mut active = self.active_vpn.lock().unwrap();
        if active.as_deref() == Some(name) {
            *active = None;
        }
        Ok(())
    }

    async fn kill_orphan_openvpn_processes(&self) {
        self.log(NmCall::KillOrphans);
    }
}
