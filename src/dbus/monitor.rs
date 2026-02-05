//! NetworkManager D-Bus monitor
//!
//! Subscribes to NetworkManager D-Bus signals for real-time VPN state changes.
//! This replaces polling for state detection while keeping nmcli for commands.

use futures_lite::StreamExt;
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::time::Instant;
use tokio::sync::mpsc;
use zbus::message::Message;
use zbus::Connection;

/// Deduplication window for D-Bus events (milliseconds)
/// Events with the same (vpn_name, event_type) within this window are dropped
const EVENT_DEDUP_WINDOW_MS: u64 = 500;

/// Events emitted by the NetworkManager monitor
#[derive(Debug, Clone)]
pub enum NmEvent {
    /// A VPN connection became active
    VpnActivated { name: String },
    /// A VPN connection was deactivated
    VpnDeactivated { name: String },
    /// A VPN connection is activating
    VpnActivating { name: String },
    /// A VPN connection failed to activate
    VpnFailed { name: String, reason: String },
    /// NetworkManager connectivity changed
    #[allow(dead_code)]
    ConnectivityChanged { connected: bool },
}

/// NetworkManager D-Bus monitor
pub struct NmMonitor {
    /// Sender for NM events
    tx: mpsc::Sender<NmEvent>,
    /// Recent events cache for deduplication: (vpn_name, event_type) -> timestamp
    recent_events: HashMap<(String, String), Instant>,
}

impl NmMonitor {
    /// Create a new NetworkManager monitor
    pub fn new(tx: mpsc::Sender<NmEvent>) -> Self {
        Self {
            tx,
            recent_events: HashMap::new(),
        }
    }

    /// Check if an event should be processed or is a duplicate
    ///
    /// Returns true if the event should be processed, false if it's a duplicate
    fn should_process_event(&mut self, vpn_name: &str, event_type: &str) -> bool {
        // Filter out "unknown" VPN names (can happen during rapid state changes)
        if vpn_name == "unknown" {
            debug!("Filtering event for 'unknown' VPN: {}", event_type);
            return false;
        }

        let key = (vpn_name.to_string(), event_type.to_string());
        let now = Instant::now();

        if let Some(last_time) = self.recent_events.get(&key) {
            if now.duration_since(*last_time).as_millis() < EVENT_DEDUP_WINDOW_MS as u128 {
                debug!(
                    "Deduplicating {} event for VPN '{}' (within {}ms window)",
                    event_type, vpn_name, EVENT_DEDUP_WINDOW_MS
                );
                return false;
            }
        }

        // Clean up old entries (older than 2x dedup window)
        let cleanup_threshold = EVENT_DEDUP_WINDOW_MS * 2;
        self.recent_events
            .retain(|_, v| now.duration_since(*v).as_millis() < cleanup_threshold as u128);

        // Record this event
        self.recent_events.insert(key, now);
        true
    }

    /// Start monitoring NetworkManager D-Bus signals
    ///
    /// This runs in a loop and sends events via the channel.
    /// Should be spawned as a background task.
    pub async fn run(mut self) -> Result<(), zbus::Error> {
        use tokio::time::{timeout, Duration};

        info!("Starting NetworkManager D-Bus monitor");

        // HARDENING: Add timeout to D-Bus connection to prevent hanging
        // if D-Bus is unavailable (e.g., container environments)
        const DBUS_CONNECT_TIMEOUT_SECS: u64 = 10;

        let connection = match timeout(
            Duration::from_secs(DBUS_CONNECT_TIMEOUT_SECS),
            Connection::system(),
        )
        .await
        {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => {
                error!("D-Bus connection failed: {}", e);
                return Err(e);
            }
            Err(_) => {
                error!(
                    "D-Bus connection timed out after {}s - is dbus-daemon running?",
                    DBUS_CONNECT_TIMEOUT_SECS
                );
                return Err(zbus::Error::Failure("D-Bus connection timeout".into()));
            }
        };
        info!("Connected to system D-Bus");

        // Create a message stream to receive all signals
        let mut stream = zbus::MessageStream::from(&connection);

        // Add match rules for the signals we care about
        connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "AddMatch",
                &("type='signal',interface='org.freedesktop.NetworkManager.VPN.Connection',member='VpnStateChanged'",),
            )
            .await?;
        debug!("Subscribed to VPN.Connection.VpnStateChanged signals");

        connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "AddMatch",
                &("type='signal',interface='org.freedesktop.NetworkManager.Connection.Active',member='StateChanged'",),
            )
            .await?;
        debug!("Subscribed to Connection.Active.StateChanged signals");

        // Cache of active connection paths to names
        let mut connection_names: HashMap<String, String> = HashMap::new();

        // Initial population of connection names
        if let Err(e) = self
            .populate_connection_names(&connection, &mut connection_names)
            .await
        {
            warn!("Failed to populate initial connection names: {}", e);
        }

        while let Some(msg) = stream.next().await {
            match msg {
                Ok(msg) => {
                    if let Err(e) = self
                        .handle_message(&msg, &connection, &mut connection_names)
                        .await
                    {
                        debug!("Error handling D-Bus message: {}", e);
                    }
                }
                Err(e) => {
                    error!("D-Bus stream error: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Populate the cache of active connection paths to names
    async fn populate_connection_names(
        &self,
        connection: &Connection,
        cache: &mut HashMap<String, String>,
    ) -> Result<(), zbus::Error> {
        // Get list of active connections from NetworkManager
        let reply = connection
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                "/org/freedesktop/NetworkManager",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.freedesktop.NetworkManager", "ActiveConnections"),
            )
            .await?;

        let body = reply.body();
        if let Ok(zbus::zvariant::Value::Array(arr)) = body.deserialize::<zbus::zvariant::Value>() {
            for path_val in arr.iter() {
                if let zbus::zvariant::Value::ObjectPath(path) = path_val {
                    if let Ok(name) = self.get_connection_name(connection, path.as_str()).await {
                        cache.insert(path.to_string(), name);
                    }
                }
            }
        }

        debug!("Populated {} active connection names", cache.len());
        Ok(())
    }

    /// Get the connection name from its path
    async fn get_connection_name(
        &self,
        connection: &Connection,
        path: &str,
    ) -> Result<String, zbus::Error> {
        let reply = connection
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                path,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.freedesktop.NetworkManager.Connection.Active", "Id"),
            )
            .await?;

        let body = reply.body();
        if let Ok(zbus::zvariant::Value::Str(s)) = body.deserialize::<zbus::zvariant::Value>() {
            return Ok(s.to_string());
        }

        Err(zbus::Error::Failure("Failed to get connection name".into()))
    }

    /// Check if a connection is a VPN type
    async fn is_vpn_connection(
        &self,
        connection: &Connection,
        path: &str,
    ) -> Result<bool, zbus::Error> {
        let reply = connection
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                path,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.freedesktop.NetworkManager.Connection.Active", "Type"),
            )
            .await?;

        let body = reply.body();
        if let Ok(zbus::zvariant::Value::Str(s)) = body.deserialize::<zbus::zvariant::Value>() {
            return Ok(s.as_str() == "vpn");
        }

        Ok(false)
    }

    /// Handle a D-Bus message
    async fn handle_message(
        &mut self,
        msg: &Message,
        connection: &Connection,
        cache: &mut HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let header = msg.header();
        let member = header.member().map(|m| m.as_str());
        let interface = header.interface().map(|i| i.as_str());
        let path = header.path().map(|p| p.as_str());

        match (interface, member) {
            (Some("org.freedesktop.NetworkManager.VPN.Connection"), Some("VpnStateChanged")) => {
                self.handle_vpn_state_changed(msg, path, connection, cache)
                    .await?;
            }
            (Some("org.freedesktop.NetworkManager.Connection.Active"), Some("StateChanged")) => {
                self.handle_active_state_changed(msg, path, connection, cache)
                    .await?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Handle VPN-specific state change signal
    async fn handle_vpn_state_changed(
        &mut self,
        msg: &Message,
        path: Option<&str>,
        connection: &Connection,
        cache: &mut HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let path = path.ok_or("No path in message")?;

        // VpnStateChanged sends (state: u32, reason: u32)
        let body = msg.body();
        let (state, reason): (u32, u32) = body.deserialize()?;

        // Get connection name
        let name = if let Some(cached) = cache.get(path) {
            cached.clone()
        } else if let Ok(n) = self.get_connection_name(connection, path).await {
            cache.insert(path.to_string(), n.clone());
            n
        } else {
            "unknown".to_string()
        };

        // Determine event type for deduplication
        let event_type = match state {
            1..=4 => "activating",
            5 => "activated",
            6 => "failed",
            7 => "deactivated",
            _ => return Ok(()),
        };

        // Apply deduplication
        if !self.should_process_event(&name, event_type) {
            return Ok(());
        }

        // NM_VPN_CONNECTION_STATE values:
        // 0 = Unknown, 1 = Prepare, 2 = NeedAuth, 3 = Connect, 4 = IPConfigGet
        // 5 = Activated, 6 = Failed, 7 = Disconnected
        let event = match state {
            1..=4 => {
                info!("VPN '{}' is activating (state={})", name, state);
                Some(NmEvent::VpnActivating { name })
            }
            5 => {
                info!("VPN '{}' activated", name);
                Some(NmEvent::VpnActivated { name })
            }
            6 => {
                let reason_str = vpn_failure_reason(reason);
                warn!("VPN '{}' failed: {}", name, reason_str);
                Some(NmEvent::VpnFailed {
                    name,
                    reason: reason_str,
                })
            }
            7 => {
                info!("VPN '{}' disconnected", name);
                cache.remove(path);
                Some(NmEvent::VpnDeactivated { name })
            }
            _ => None,
        };

        if let Some(event) = event {
            if self.tx.send(event).await.is_err() {
                debug!("Event receiver dropped");
            }
        }

        Ok(())
    }

    /// Handle ActiveConnection state change
    async fn handle_active_state_changed(
        &mut self,
        msg: &Message,
        path: Option<&str>,
        connection: &Connection,
        cache: &mut HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let path = path.ok_or("No path in message")?;

        // Check if this is a VPN connection
        if !self
            .is_vpn_connection(connection, path)
            .await
            .unwrap_or(false)
        {
            return Ok(());
        }

        // StateChanged sends (state: u32, reason: u32)
        let body = msg.body();
        let (state, _reason): (u32, u32) = body.deserialize()?;

        // Get connection name
        let name = if let Some(cached) = cache.get(path) {
            cached.clone()
        } else if let Ok(n) = self.get_connection_name(connection, path).await {
            cache.insert(path.to_string(), n.clone());
            n
        } else {
            return Ok(());
        };

        // Determine event type for deduplication
        let event_type = match state {
            1 => "activating",
            2 => "activated",
            4 => "deactivated",
            _ => return Ok(()),
        };

        // Apply deduplication
        if !self.should_process_event(&name, event_type) {
            return Ok(());
        }

        // NM_ACTIVE_CONNECTION_STATE values:
        // 0 = Unknown, 1 = Activating, 2 = Activated, 3 = Deactivating, 4 = Deactivated
        let event = match state {
            1 => {
                debug!("ActiveConnection '{}' activating", name);
                Some(NmEvent::VpnActivating { name })
            }
            2 => {
                debug!("ActiveConnection '{}' activated", name);
                Some(NmEvent::VpnActivated { name })
            }
            4 => {
                debug!("ActiveConnection '{}' deactivated", name);
                cache.remove(path);
                Some(NmEvent::VpnDeactivated { name })
            }
            _ => None,
        };

        if let Some(event) = event {
            if self.tx.send(event).await.is_err() {
                debug!("Event receiver dropped");
            }
        }

        Ok(())
    }
}

/// Convert VPN failure reason code to string
fn vpn_failure_reason(reason: u32) -> String {
    match reason {
        0 => "Unknown".to_string(),
        1 => "Not provided".to_string(),
        2 => "User disconnected".to_string(),
        3 => "Service stopped".to_string(),
        4 => "IP config invalid".to_string(),
        5 => "Connect timeout".to_string(),
        6 => "Service start timeout".to_string(),
        7 => "Service start failed".to_string(),
        8 => "No secrets".to_string(),
        9 => "Login failed".to_string(),
        10 => "Connection removed".to_string(),
        _ => format!("Unknown reason ({})", reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vpn_failure_reason() {
        assert_eq!(vpn_failure_reason(5), "Connect timeout");
        assert_eq!(vpn_failure_reason(9), "Login failed");
        assert!(vpn_failure_reason(99).contains("Unknown"));
    }

    #[test]
    fn test_vpn_failure_reason_basic_codes() {
        assert_eq!(vpn_failure_reason(0), "Unknown");
        assert_eq!(vpn_failure_reason(1), "Not provided");
    }
}
