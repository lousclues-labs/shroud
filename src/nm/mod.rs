//! NetworkManager module
//!
//! Provides the interface to NetworkManager for managing VPN connections.
//! Currently uses nmcli subprocess calls; future work will add D-Bus event subscription.

pub mod client;
pub mod connections;

pub use client::{
    connect, disconnect, get_active_vpn, get_active_vpn_with_state, get_all_active_vpns,
    get_vpn_state, kill_orphan_openvpn_processes, list_vpn_connections,
};
#[allow(unused_imports)]
pub use connections::{get_vpn_type, list_vpn_connections_with_types, VpnConnection, VpnType};
