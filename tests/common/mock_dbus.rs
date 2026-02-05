//! Mock D-Bus client for testing
//!
//! Provides a mock implementation of D-Bus/NetworkManager event subscription
//! that allows testing event handling without a real D-Bus connection.

use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Mock D-Bus event types (mirrors real NmEvent)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockDbusEvent {
    /// VPN connection state changed
    VpnStateChanged { name: String, state: MockVpnState },
    /// NetworkManager connectivity changed
    ConnectivityChanged { connectivity: MockConnectivity },
    /// NetworkManager is going down
    NmStopping,
    /// Network interface changed
    InterfaceChanged { interface: String, up: bool },
}

/// VPN state for mock events
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockVpnState {
    Disconnected,
    Prepare,
    NeedAuth,
    Connecting,
    GettingIpConfig,
    Connected,
    Failed,
    Disconnecting,
}

/// Connectivity state for mock events
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockConnectivity {
    Unknown,
    None,
    Portal,
    Limited,
    Full,
}

/// Mock D-Bus connection for testing
#[derive(Clone)]
pub struct MockDbus {
    events_tx: mpsc::Sender<MockDbusEvent>,
    events_rx: Arc<Mutex<Option<mpsc::Receiver<MockDbusEvent>>>>,
    connected: Arc<Mutex<bool>>,
    event_log: Arc<Mutex<Vec<MockDbusEvent>>>,
}

impl MockDbus {
    /// Create a new mock D-Bus with a channel for injecting events
    pub fn new() -> (Self, mpsc::Sender<MockDbusEvent>) {
        let (tx, rx) = mpsc::channel(100);
        let mock = Self {
            events_tx: tx.clone(),
            events_rx: Arc::new(Mutex::new(Some(rx))),
            connected: Arc::new(Mutex::new(true)),
            event_log: Arc::new(Mutex::new(Vec::new())),
        };
        (mock, tx)
    }

    /// Take the event receiver (can only be done once)
    pub fn take_receiver(&self) -> Option<mpsc::Receiver<MockDbusEvent>> {
        self.events_rx.lock().unwrap().take()
    }

    /// Inject an event
    pub async fn inject_event(&self, event: MockDbusEvent) {
        self.event_log.lock().unwrap().push(event.clone());
        let _ = self.events_tx.send(event).await;
    }

    /// Inject multiple events
    pub async fn inject_events(&self, events: &[MockDbusEvent]) {
        for event in events {
            self.inject_event(event.clone()).await;
        }
    }

    /// Simulate D-Bus disconnect
    pub fn disconnect(&self) {
        *self.connected.lock().unwrap() = false;
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        *self.connected.lock().unwrap()
    }

    /// Get all events that were injected
    pub fn events(&self) -> Vec<MockDbusEvent> {
        self.event_log.lock().unwrap().clone()
    }

    // ========================================================================
    // Convenience methods for common event sequences
    // ========================================================================

    /// Inject a VPN connection sequence (connect -> connected)
    pub async fn simulate_vpn_connect(&self, vpn_name: &str) {
        let events = [
            MockDbusEvent::VpnStateChanged {
                name: vpn_name.to_string(),
                state: MockVpnState::Prepare,
            },
            MockDbusEvent::VpnStateChanged {
                name: vpn_name.to_string(),
                state: MockVpnState::Connecting,
            },
            MockDbusEvent::VpnStateChanged {
                name: vpn_name.to_string(),
                state: MockVpnState::GettingIpConfig,
            },
            MockDbusEvent::VpnStateChanged {
                name: vpn_name.to_string(),
                state: MockVpnState::Connected,
            },
        ];
        self.inject_events(&events).await;
    }

    /// Inject a VPN disconnection sequence
    pub async fn simulate_vpn_disconnect(&self, vpn_name: &str) {
        let events = [
            MockDbusEvent::VpnStateChanged {
                name: vpn_name.to_string(),
                state: MockVpnState::Disconnecting,
            },
            MockDbusEvent::VpnStateChanged {
                name: vpn_name.to_string(),
                state: MockVpnState::Disconnected,
            },
        ];
        self.inject_events(&events).await;
    }

    /// Inject a VPN failure
    pub async fn simulate_vpn_failure(&self, vpn_name: &str) {
        self.inject_event(MockDbusEvent::VpnStateChanged {
            name: vpn_name.to_string(),
            state: MockVpnState::Failed,
        })
        .await;
    }

    /// Inject connectivity loss
    pub async fn simulate_connectivity_loss(&self) {
        self.inject_event(MockDbusEvent::ConnectivityChanged {
            connectivity: MockConnectivity::None,
        })
        .await;
    }

    /// Inject connectivity restored
    pub async fn simulate_connectivity_restored(&self) {
        self.inject_event(MockDbusEvent::ConnectivityChanged {
            connectivity: MockConnectivity::Full,
        })
        .await;
    }
}

impl Default for MockDbus {
    fn default() -> Self {
        Self::new().0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_dbus_event_injection() {
        let (mock, _tx) = MockDbus::new();
        let mut rx = mock.take_receiver().unwrap();

        mock.inject_event(MockDbusEvent::VpnStateChanged {
            name: "vpn1".to_string(),
            state: MockVpnState::Connected,
        })
        .await;

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, MockDbusEvent::VpnStateChanged { .. }));
    }

    #[tokio::test]
    async fn test_mock_dbus_vpn_connect_sequence() {
        let (mock, _tx) = MockDbus::new();
        let mut rx = mock.take_receiver().unwrap();

        mock.simulate_vpn_connect("test-vpn").await;

        // Should receive all events in sequence
        let mut states = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let MockDbusEvent::VpnStateChanged { state, .. } = event {
                states.push(state);
            }
        }

        assert_eq!(states.len(), 4);
        assert_eq!(states[0], MockVpnState::Prepare);
        assert_eq!(states[3], MockVpnState::Connected);
    }
}
