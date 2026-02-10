//! Trait abstraction over NetworkManager operations.
//!
//! This allows the supervisor to be tested with mock implementations
//! that don't spawn processes or require D-Bus.

use async_trait::async_trait;

use crate::state::{ActiveVpnInfo, NmVpnState};

/// Errors from NM operations (re-uses existing NmError)
pub use super::client::NmError;

/// Trait abstracting NetworkManager operations.
///
/// The real implementation (`NmCliClient`) calls nmcli via subprocess.
/// Test implementations can return preset results, inject failures,
/// and log all calls for verification.
#[async_trait]
pub trait NmClient: Send + Sync {
    /// List all configured VPN connection names.
    async fn list_vpn_connections(&self) -> Vec<String>;

    /// Get the name of the currently active VPN, if any.
    async fn get_active_vpn(&self) -> Option<String>;

    /// Get detailed info about the active VPN (name + NM state).
    async fn get_active_vpn_with_state(&self) -> Option<ActiveVpnInfo>;

    /// Get ALL active VPNs (for detecting multiple simultaneous connections).
    async fn get_all_active_vpns(&self) -> Vec<ActiveVpnInfo>;

    /// Get the precise NM state of a specific connection.
    async fn get_vpn_state(&self, name: &str) -> Option<NmVpnState>;

    /// Activate a VPN connection by name.
    async fn connect(&self, name: &str) -> Result<(), NmError>;

    /// Deactivate a VPN connection by name.
    async fn disconnect(&self, name: &str) -> Result<(), NmError>;

    /// Kill orphan OpenVPN processes.
    async fn kill_orphan_openvpn_processes(&self);
}

/// Production implementation that shells out to nmcli.
pub struct NmCliClient;

#[async_trait]
impl NmClient for NmCliClient {
    async fn list_vpn_connections(&self) -> Vec<String> {
        super::client::list_vpn_connections().await
    }

    async fn get_active_vpn(&self) -> Option<String> {
        super::client::get_active_vpn().await
    }

    async fn get_active_vpn_with_state(&self) -> Option<ActiveVpnInfo> {
        super::client::get_active_vpn_with_state().await
    }

    async fn get_all_active_vpns(&self) -> Vec<ActiveVpnInfo> {
        super::client::get_all_active_vpns().await
    }

    async fn get_vpn_state(&self, name: &str) -> Option<NmVpnState> {
        super::client::get_vpn_state(name).await
    }

    async fn connect(&self, name: &str) -> Result<(), NmError> {
        super::client::connect(name).await
    }

    async fn disconnect(&self, name: &str) -> Result<(), NmError> {
        super::client::disconnect(name).await
    }

    async fn kill_orphan_openvpn_processes(&self) {
        super::client::kill_orphan_openvpn_processes().await
    }
}
