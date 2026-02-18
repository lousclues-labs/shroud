// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Mock NetworkManager client for testing
//!
//! Provides a mock implementation of NetworkManager operations that allows
//! integration tests to run without spawning real processes or requiring D-Bus.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Represents a VPN connection in the mock
#[derive(Debug, Clone)]
pub struct MockVpnConnection {
    pub name: String,
    pub uuid: String,
    pub vpn_type: MockVpnType,
    pub active: bool,
}

/// VPN type enumeration for mocks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockVpnType {
    OpenVpn,
    WireGuard,
    Unknown,
}

impl std::fmt::Display for MockVpnType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MockVpnType::OpenVpn => write!(f, "openvpn"),
            MockVpnType::WireGuard => write!(f, "wireguard"),
            MockVpnType::Unknown => write!(f, "vpn"),
        }
    }
}

/// Mock NetworkManager client for testing
///
/// This mock maintains state in memory and can be configured to:
/// - Return preset VPN connections
/// - Simulate failures on next call
/// - Track all method calls for verification
#[derive(Clone)]
pub struct MockNetworkManager {
    connections: Arc<Mutex<HashMap<String, MockVpnConnection>>>,
    active_vpn: Arc<Mutex<Option<String>>>,
    fail_next: Arc<Mutex<Option<MockNmError>>>,
    call_log: Arc<Mutex<Vec<MockNmCall>>>,
    state: Arc<Mutex<MockNmState>>,
}

/// State of the mock NetworkManager
#[derive(Debug, Clone, Default)]
pub struct MockNmState {
    /// Whether NetworkManager is "running"
    pub nm_running: bool,
    /// Whether we have network connectivity
    pub has_connectivity: bool,
    /// Activation delay in milliseconds (simulates slow activations)
    pub activation_delay_ms: u64,
    /// Whether to auto-fail after N successful activations
    pub fail_after_n_activations: Option<u32>,
    /// Current activation count
    pub activation_count: u32,
}

impl MockNmState {
    pub fn healthy() -> Self {
        Self {
            nm_running: true,
            has_connectivity: true,
            activation_delay_ms: 0,
            fail_after_n_activations: None,
            activation_count: 0,
        }
    }
}

/// Errors that can be returned by the mock
#[derive(Debug, Clone)]
pub enum MockNmError {
    /// NetworkManager is not running
    NotRunning,
    /// No network connectivity
    NoConnectivity,
    /// VPN not found
    VpnNotFound(String),
    /// Activation failed
    ActivationFailed(String),
    /// Deactivation failed
    DeactivationFailed(String),
    /// Timeout
    Timeout,
    /// Custom error message
    Custom(String),
}

impl std::fmt::Display for MockNmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MockNmError::NotRunning => write!(f, "NetworkManager is not running"),
            MockNmError::NoConnectivity => write!(f, "No network connectivity"),
            MockNmError::VpnNotFound(name) => write!(f, "VPN '{}' not found", name),
            MockNmError::ActivationFailed(msg) => write!(f, "Activation failed: {}", msg),
            MockNmError::DeactivationFailed(msg) => write!(f, "Deactivation failed: {}", msg),
            MockNmError::Timeout => write!(f, "Operation timed out"),
            MockNmError::Custom(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for MockNmError {}

/// Record of a call made to the mock
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockNmCall {
    ListConnections,
    GetActiveVpn,
    ActivateVpn(String),
    DeactivateVpn(String),
    GetVpnState(String),
    IsNmRunning,
}

impl MockNetworkManager {
    /// Create a new mock with no VPN connections
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
            active_vpn: Arc::new(Mutex::new(None)),
            fail_next: Arc::new(Mutex::new(None)),
            call_log: Arc::new(Mutex::new(Vec::new())),
            state: Arc::new(Mutex::new(MockNmState::healthy())),
        }
    }

    /// Create with preset VPN connections
    pub fn with_vpns(vpn_names: &[&str]) -> Self {
        let mock = Self::new();
        for name in vpn_names {
            mock.add_vpn(name, MockVpnType::OpenVpn);
        }
        mock
    }

    /// Create with specific VPN types
    pub fn with_typed_vpns(vpns: &[(&str, MockVpnType)]) -> Self {
        let mock = Self::new();
        for (name, vpn_type) in vpns {
            mock.add_vpn(name, *vpn_type);
        }
        mock
    }

    /// Add a VPN connection
    pub fn add_vpn(&self, name: &str, vpn_type: MockVpnType) {
        let mut conns = self.connections.lock().unwrap();
        conns.insert(
            name.to_string(),
            MockVpnConnection {
                name: name.to_string(),
                uuid: format!("mock-uuid-{}", name),
                vpn_type,
                active: false,
            },
        );
    }

    /// Remove a VPN connection
    pub fn remove_vpn(&self, name: &str) {
        let mut conns = self.connections.lock().unwrap();
        conns.remove(name);
    }

    /// Set the next call to fail with given error
    pub fn fail_next_call(&self, error: MockNmError) {
        *self.fail_next.lock().unwrap() = Some(error);
    }

    /// Set state parameters
    pub fn set_state(&self, state: MockNmState) {
        *self.state.lock().unwrap() = state;
    }

    /// Mark a VPN as already active (for testing disconnect scenarios)
    pub fn set_active(&self, name: &str) {
        let mut conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get_mut(name) {
            conn.active = true;
        }
        *self.active_vpn.lock().unwrap() = Some(name.to_string());
    }

    /// Get all calls made to this mock
    pub fn calls(&self) -> Vec<MockNmCall> {
        self.call_log.lock().unwrap().clone()
    }

    /// Check if a specific call was made
    pub fn was_called(&self, call: &MockNmCall) -> bool {
        self.call_log.lock().unwrap().contains(call)
    }

    /// Count calls of a specific type
    pub fn call_count(&self, call_type: &MockNmCall) -> usize {
        self.call_log
            .lock()
            .unwrap()
            .iter()
            .filter(|c| std::mem::discriminant(*c) == std::mem::discriminant(call_type))
            .count()
    }

    /// Clear call log
    pub fn clear_calls(&self) {
        self.call_log.lock().unwrap().clear();
    }

    // Internal helper to log calls
    fn log_call(&self, call: MockNmCall) {
        self.call_log.lock().unwrap().push(call);
    }

    // Internal helper to check for preset failures
    fn check_fail(&self) -> Result<(), MockNmError> {
        if let Some(err) = self.fail_next.lock().unwrap().take() {
            return Err(err);
        }

        let state = self.state.lock().unwrap();
        if !state.nm_running {
            return Err(MockNmError::NotRunning);
        }
        Ok(())
    }

    // ========================================================================
    // Public API (mimics real NetworkManager operations)
    // ========================================================================

    /// List all VPN connections
    pub fn list_vpn_connections(&self) -> Result<Vec<MockVpnConnection>, MockNmError> {
        self.log_call(MockNmCall::ListConnections);
        self.check_fail()?;

        let conns = self.connections.lock().unwrap();
        Ok(conns.values().cloned().collect())
    }

    /// Get currently active VPN name
    pub fn get_active_vpn(&self) -> Result<Option<String>, MockNmError> {
        self.log_call(MockNmCall::GetActiveVpn);
        self.check_fail()?;

        Ok(self.active_vpn.lock().unwrap().clone())
    }

    /// Activate a VPN connection
    pub fn activate_vpn(&self, name: &str) -> Result<(), MockNmError> {
        self.log_call(MockNmCall::ActivateVpn(name.to_string()));
        self.check_fail()?;

        // Check if VPN exists
        let mut conns = self.connections.lock().unwrap();
        if !conns.contains_key(name) {
            return Err(MockNmError::VpnNotFound(name.to_string()));
        }

        // Check activation limit
        {
            let mut state = self.state.lock().unwrap();
            state.activation_count += 1;
            if let Some(limit) = state.fail_after_n_activations {
                if state.activation_count > limit {
                    return Err(MockNmError::ActivationFailed(
                        "Activation limit exceeded".to_string(),
                    ));
                }
            }
        }

        // Deactivate any currently active VPN
        for conn in conns.values_mut() {
            conn.active = false;
        }

        // Activate the requested VPN
        if let Some(conn) = conns.get_mut(name) {
            conn.active = true;
        }
        *self.active_vpn.lock().unwrap() = Some(name.to_string());

        Ok(())
    }

    /// Deactivate a VPN connection
    pub fn deactivate_vpn(&self, name: &str) -> Result<(), MockNmError> {
        self.log_call(MockNmCall::DeactivateVpn(name.to_string()));
        self.check_fail()?;

        let mut conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get_mut(name) {
            conn.active = false;
        }

        let mut active = self.active_vpn.lock().unwrap();
        if active.as_deref() == Some(name) {
            *active = None;
        }

        Ok(())
    }

    /// Get state of a specific VPN
    pub fn get_vpn_state(&self, name: &str) -> Result<MockVpnState, MockNmError> {
        self.log_call(MockNmCall::GetVpnState(name.to_string()));
        self.check_fail()?;

        let conns = self.connections.lock().unwrap();
        match conns.get(name) {
            Some(conn) if conn.active => Ok(MockVpnState::Activated),
            Some(_) => Ok(MockVpnState::Deactivated),
            None => Err(MockNmError::VpnNotFound(name.to_string())),
        }
    }

    /// Check if NetworkManager is running
    pub fn is_running(&self) -> Result<bool, MockNmError> {
        self.log_call(MockNmCall::IsNmRunning);
        // Don't use check_fail here - we want to return false, not error
        let state = self.state.lock().unwrap();
        Ok(state.nm_running)
    }
}

impl Default for MockNetworkManager {
    fn default() -> Self {
        Self::new()
    }
}

/// VPN state enumeration for mocks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockVpnState {
    Activated,
    Activating,
    Deactivating,
    Deactivated,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_nm_basic_operations() {
        let mock = MockNetworkManager::with_vpns(&["vpn1", "vpn2"]);

        // List connections
        let vpns = mock.list_vpn_connections().unwrap();
        assert_eq!(vpns.len(), 2);

        // No active VPN initially
        assert!(mock.get_active_vpn().unwrap().is_none());

        // Activate VPN
        mock.activate_vpn("vpn1").unwrap();
        assert_eq!(mock.get_active_vpn().unwrap(), Some("vpn1".to_string()));

        // Verify calls logged
        let calls = mock.calls();
        assert!(calls.contains(&MockNmCall::ListConnections));
        assert!(calls.contains(&MockNmCall::ActivateVpn("vpn1".to_string())));
    }

    #[test]
    fn test_mock_nm_fail_injection() {
        let mock = MockNetworkManager::with_vpns(&["vpn1"]);

        // Set up failure
        mock.fail_next_call(MockNmError::Timeout);

        // Next call should fail
        let result = mock.activate_vpn("vpn1");
        assert!(result.is_err());

        // Subsequent calls should succeed
        let result = mock.activate_vpn("vpn1");
        assert!(result.is_ok());
    }

    #[test]
    fn test_mock_nm_activation_limit() {
        let mock = MockNetworkManager::with_vpns(&["vpn1"]);
        mock.set_state(MockNmState {
            nm_running: true,
            has_connectivity: true,
            activation_delay_ms: 0,
            fail_after_n_activations: Some(2),
            activation_count: 0,
        });

        // First two activations succeed
        assert!(mock.activate_vpn("vpn1").is_ok());
        mock.deactivate_vpn("vpn1").ok();
        assert!(mock.activate_vpn("vpn1").is_ok());

        // Third fails
        mock.deactivate_vpn("vpn1").ok();
        assert!(mock.activate_vpn("vpn1").is_err());
    }
}
