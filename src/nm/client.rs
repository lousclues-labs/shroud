//! NetworkManager interface via nmcli
//!
//! Provides functions for interacting with NetworkManager to manage VPN connections.
//! Currently uses nmcli subprocess calls. Future: migrate to D-Bus for event-driven updates.

use log::{debug, error, info, warn};
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use crate::state::{ActiveVpnInfo, NmVpnState};

/// Timeout for nmcli commands in seconds
const NMCLI_TIMEOUT_SECS: u64 = 30;

/// Run nmcli and return the output, handling timeout and errors
async fn run_nmcli(args: &[&str]) -> Option<std::process::Output> {
    match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new("nmcli")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) if output.status.success() => Some(output),
        Ok(Ok(_)) => {
            debug!("nmcli returned non-zero exit status");
            None
        }
        Ok(Err(e)) => {
            warn!("Failed to execute nmcli: {}", e);
            None
        }
        Err(_) => {
            warn!("nmcli timed out after {} seconds", NMCLI_TIMEOUT_SECS);
            None
        }
    }
}

/// Parse VPN connections from nmcli output
/// Returns all VPNs with their states
fn parse_active_vpns(stdout: &str) -> Vec<ActiveVpnInfo> {
    let mut vpns = Vec::new();

    for line in stdout.lines() {
        // Split on colon from the right to handle names with colons
        let parts: Vec<&str> = line.rsplitn(3, ':').collect();
        if parts.len() >= 3 {
            let state_str = parts[0];
            let conn_type = parts[1];
            let name = parts[2];

            if conn_type == "vpn" {
                if let Some(state) = match state_str {
                    "activated" => Some(NmVpnState::Activated),
                    "activating" => Some(NmVpnState::Activating),
                    "deactivating" => Some(NmVpnState::Deactivating),
                    _ => None,
                } {
                    vpns.push(ActiveVpnInfo {
                        name: name.to_string(),
                        state,
                    });
                }
            }
        }
    }

    vpns
}

/// Parse VPN connection names from nmcli "NAME:TYPE" output
fn parse_vpn_connections(stdout: &str) -> Vec<String> {
    let mut connections = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.rsplitn(2, ':').collect();
        if parts.len() >= 2 && parts[0] == "vpn" {
            connections.push(parts[1].to_string());
        }
    }
    connections
}

/// Parse VPN UUID from nmcli "UUID:NAME:TYPE" output for a specific connection
fn parse_vpn_uuid(stdout: &str, connection_name: &str) -> Option<String> {
    for line in stdout.lines() {
        // Format: UUID:NAME:TYPE - split from right to handle names with colons
        let parts: Vec<&str> = line.rsplitn(3, ':').collect();
        if parts.len() >= 3 && parts[0] == "vpn" && parts[1] == connection_name {
            return Some(parts[2].to_string());
        }
    }
    None
}

/// Get the active VPN connection name from NetworkManager (legacy compatibility wrapper)
#[inline]
pub async fn get_active_vpn() -> Option<String> {
    get_active_vpn_with_state()
        .await
        .filter(|info| info.state == NmVpnState::Activated)
        .map(|info| info.name)
}

/// Get the active VPN with detailed state information from NetworkManager
pub async fn get_active_vpn_with_state() -> Option<ActiveVpnInfo> {
    let output = run_nmcli(&["-t", "-f", "NAME,TYPE,STATE", "con", "show", "--active"]).await?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    debug!("nmcli active connections: {}", stdout.trim());

    let vpns = parse_active_vpns(&stdout);

    // Priority: activated > activating > deactivating
    vpns.iter()
        .find(|v| v.state == NmVpnState::Activated)
        .or_else(|| vpns.iter().find(|v| v.state == NmVpnState::Activating))
        .or_else(|| vpns.iter().find(|v| v.state == NmVpnState::Deactivating))
        .cloned()
}

/// Get ALL active VPN connections from NetworkManager (to detect multiple simultaneous VPNs)
pub async fn get_all_active_vpns() -> Vec<ActiveVpnInfo> {
    let output = match run_nmcli(&["-t", "-f", "NAME,TYPE,STATE", "con", "show", "--active"]).await
    {
        Some(o) => o,
        None => return Vec::new(),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_active_vpns(&stdout)
}

/// Get the precise state of a specific VPN connection
#[inline]
pub async fn get_vpn_state(connection_name: &str) -> Option<NmVpnState> {
    let output = run_nmcli(&["-t", "-f", "NAME,TYPE,STATE", "con", "show", "--active"]).await?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    parse_active_vpns(&stdout)
        .into_iter()
        .find(|v| v.name == connection_name)
        .map(|v| v.state)
}

/// List all VPN connections configured in NetworkManager
pub async fn list_vpn_connections() -> Vec<String> {
    debug!("Listing VPN connections from NetworkManager");

    let output = match run_nmcli(&["-t", "-f", "NAME,TYPE", "con", "show"]).await {
        Some(o) => o,
        None => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let connections = parse_vpn_connections(&stdout);

    info!("Found {} VPN connection(s)", connections.len());
    connections
}

/// Get the UUID of a VPN connection by name
pub async fn get_vpn_uuid(connection_name: &str) -> Option<String> {
    let output = run_nmcli(&["-t", "-f", "UUID,NAME,TYPE", "con", "show"]).await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_vpn_uuid(&stdout, connection_name)
}

/// Connect to a VPN via NetworkManager
pub async fn connect(connection_name: &str) -> Result<(), String> {
    info!("Activating VPN connection: {}", connection_name);

    let output = match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new("nmcli")
            .args(["con", "up", connection_name])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            let msg = format!("Failed to execute nmcli: {}", e);
            error!("{}", msg);
            return Err(msg);
        }
        Err(_) => {
            let msg = format!("nmcli timed out after {} seconds", NMCLI_TIMEOUT_SECS);
            error!("{}", msg);
            return Err(msg);
        }
    };

    if output.status.success() {
        info!("VPN activation successful");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = format!("nmcli failed: {}", stderr.trim());
        error!("{}", msg);
        Err(msg)
    }
}

/// Disconnect a VPN via NetworkManager
pub async fn disconnect(connection_name: &str) -> Result<(), String> {
    info!("Deactivating VPN connection: {}", connection_name);

    // First, try to get UUID for more reliable disconnection
    let uuid_opt = get_vpn_uuid(connection_name).await;

    // Try disconnecting by UUID first (more reliable)
    if let Some(uuid) = uuid_opt {
        debug!("Attempting disconnect by UUID: {}", uuid);
        let output_result = timeout(
            Duration::from_secs(NMCLI_TIMEOUT_SECS),
            Command::new("nmcli")
                .args(["con", "down", "uuid", &uuid])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output(),
        )
        .await;

        match output_result {
            Ok(Ok(output)) => {
                if output.status.success() {
                    info!("VPN deactivation by UUID successful");
                    return Ok(());
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    warn!(
                        "Disconnect by UUID failed: {}, trying by name",
                        stderr.trim()
                    );
                }
            }
            Ok(Err(e)) => {
                warn!("Failed to execute nmcli with UUID: {}, trying by name", e);
            }
            Err(_) => {
                warn!(
                    "nmcli timed out after {} seconds with UUID, trying by name",
                    NMCLI_TIMEOUT_SECS
                );
            }
        }
    }

    // Fallback: Try disconnecting by name
    debug!("Attempting disconnect by name: {}", connection_name);
    let output = match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new("nmcli")
            .args(["con", "down", connection_name])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            let msg = format!("Failed to execute nmcli: {}", e);
            error!("{}", msg);
            return Err(msg);
        }
        Err(_) => {
            let msg = format!("nmcli timed out after {} seconds", NMCLI_TIMEOUT_SECS);
            error!("{}", msg);
            return Err(msg);
        }
    };

    if output.status.success() {
        info!("VPN deactivation by name successful");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!("Disconnect by name also failed: {}", stderr.trim());

        // Last resort: Try device-level disconnect
        debug!("Attempting device-level disconnect as last resort");
        disconnect_vpn_device().await
    }
}

/// Disconnect VPN by finding and disconnecting the tun device
async fn disconnect_vpn_device() -> Result<(), String> {
    let dev_output = match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new("nmcli")
            .args(["-t", "-f", "DEVICE,TYPE", "dev"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            let msg = format!("Failed to list devices: {}", e);
            error!("{}", msg);
            return Err(msg);
        }
        Err(_) => {
            let msg = "Device list timed out".to_string();
            error!("{}", msg);
            return Err(msg);
        }
    };

    if dev_output.status.success() {
        let dev_stdout = String::from_utf8_lossy(&dev_output.stdout);
        for line in dev_stdout.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 && parts[1] == "tun" {
                let device = parts[0];
                debug!("Found VPN device: {}, attempting disconnect", device);

                let disconnect_output = match timeout(
                    Duration::from_secs(NMCLI_TIMEOUT_SECS),
                    Command::new("nmcli")
                        .args(["dev", "disconnect", device])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .output(),
                )
                .await
                {
                    Ok(Ok(output)) => output,
                    Ok(Err(e)) => {
                        warn!("Failed to disconnect device: {}", e);
                        continue;
                    }
                    Err(_) => {
                        warn!("Device disconnect timed out");
                        continue;
                    }
                };

                if disconnect_output.status.success() {
                    info!("VPN device disconnect successful");
                    return Ok(());
                }
            }
        }
    }

    let msg = "All disconnect methods failed".to_string();
    error!("{}", msg);
    Err(msg)
}

/// Kill orphan OpenVPN processes that may be blocking new connections
/// Uses pkill for safety - it atomically checks process name at kill time,
/// avoiding TOCTOU race conditions where a PID could be reused
pub async fn kill_orphan_openvpn_processes() {
    debug!("Checking for orphan OpenVPN processes");

    // Use pkill with exact match - it handles the race condition internally
    // by checking process name at kill time (atomic operation)
    let result = Command::new("pkill")
        .args(["-x", "openvpn"]) // -x for exact match only
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await;

    match result {
        Ok(output) => {
            // pkill returns 0 if processes were killed, 1 if none found
            if output.status.success() {
                info!("Cleaned up orphan OpenVPN processes");
            } else {
                debug!("No orphan OpenVPN processes found");
            }
        }
        Err(e) => {
            debug!("Failed to run pkill: {}", e);
        }
    }

    // Give processes time to terminate
    tokio::time::sleep(Duration::from_millis(500)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_active_vpns() {
        let output = "my-vpn:vpn:activated\nwifi:802-11-wireless:activated\n";
        let vpns = parse_active_vpns(output);
        assert_eq!(vpns.len(), 1);
        assert_eq!(vpns[0].name, "my-vpn");
        assert_eq!(vpns[0].state, NmVpnState::Activated);
    }

    #[test]
    fn test_parse_active_vpns_with_colons_in_name() {
        let output = "vpn:server:123:vpn:activating\n";
        let vpns = parse_active_vpns(output);
        assert_eq!(vpns.len(), 1);
        assert_eq!(vpns[0].name, "vpn:server:123");
        assert_eq!(vpns[0].state, NmVpnState::Activating);
    }

    #[test]
    fn test_parse_active_vpns_multiple() {
        let output = "vpn1:vpn:activated\nvpn2:vpn:deactivating\n";
        let vpns = parse_active_vpns(output);
        assert_eq!(vpns.len(), 2);
    }

    // --- parse_vpn_connections tests ---

    #[test]
    fn test_parse_vpn_connections_basic() {
        let output = "my-vpn:vpn\nwifi:802-11-wireless\nwork-vpn:vpn\n";
        let connections = parse_vpn_connections(output);
        assert_eq!(connections, vec!["my-vpn", "work-vpn"]);
    }

    #[test]
    fn test_parse_vpn_connections_with_colons_in_name() {
        let output = "vpn:server:east:vpn\nregular-vpn:vpn\n";
        let connections = parse_vpn_connections(output);
        assert_eq!(connections, vec!["vpn:server:east", "regular-vpn"]);
    }

    #[test]
    fn test_parse_vpn_connections_empty_output() {
        let output = "";
        let connections = parse_vpn_connections(output);
        assert!(connections.is_empty());
    }

    #[test]
    fn test_parse_vpn_connections_no_vpns() {
        let output = "wifi:802-11-wireless\nethernet:802-3-ethernet\n";
        let connections = parse_vpn_connections(output);
        assert!(connections.is_empty());
    }

    #[test]
    fn test_parse_vpn_connections_with_spaces_in_name() {
        let output = "My VPN Connection:vpn\n";
        let connections = parse_vpn_connections(output);
        assert_eq!(connections, vec!["My VPN Connection"]);
    }

    // --- parse_vpn_uuid tests ---

    #[test]
    fn test_parse_vpn_uuid_basic() {
        let output = "abc-123:my-vpn:vpn\ndef-456:other:vpn\n";
        let uuid = parse_vpn_uuid(output, "my-vpn");
        assert_eq!(uuid, Some("abc-123".to_string()));
    }

    #[test]
    fn test_parse_vpn_uuid_with_single_colon_in_uuid() {
        // Real-world UUID format is hyphenated, not colons
        let output = "550e8400-e29b-41d4-a716-446655440000:my-vpn:vpn\n";
        let uuid = parse_vpn_uuid(output, "my-vpn");
        assert_eq!(
            uuid,
            Some("550e8400-e29b-41d4-a716-446655440000".to_string())
        );
    }

    #[test]
    fn test_parse_vpn_uuid_not_found() {
        let output = "abc-123:my-vpn:vpn\n";
        let uuid = parse_vpn_uuid(output, "nonexistent");
        assert!(uuid.is_none());
    }

    #[test]
    fn test_parse_vpn_uuid_ignores_non_vpn() {
        let output = "abc-123:my-connection:802-11-wireless\n";
        let uuid = parse_vpn_uuid(output, "my-connection");
        assert!(uuid.is_none());
    }

    #[test]
    fn test_parse_vpn_uuid_empty_output() {
        let output = "";
        let uuid = parse_vpn_uuid(output, "anything");
        assert!(uuid.is_none());
    }

    // --- Active VPN selection priority tests ---

    #[test]
    fn test_parse_active_vpns_priority_activated_over_activating() {
        let output = "activating-vpn:vpn:activating\nactive-vpn:vpn:activated\n";
        let vpns = parse_active_vpns(output);
        assert_eq!(vpns.len(), 2);

        let preferred = vpns
            .iter()
            .find(|v| v.state == NmVpnState::Activated)
            .or_else(|| vpns.iter().find(|v| v.state == NmVpnState::Activating));
        assert_eq!(preferred.unwrap().name, "active-vpn");
    }

    #[test]
    fn test_parse_active_vpns_activating_over_deactivating() {
        let output = "leaving-vpn:vpn:deactivating\njoining-vpn:vpn:activating\n";
        let vpns = parse_active_vpns(output);

        let preferred = vpns
            .iter()
            .find(|v| v.state == NmVpnState::Activated)
            .or_else(|| vpns.iter().find(|v| v.state == NmVpnState::Activating))
            .or_else(|| vpns.iter().find(|v| v.state == NmVpnState::Deactivating));
        assert_eq!(preferred.unwrap().name, "joining-vpn");
    }

    #[test]
    fn test_parse_active_vpns_only_deactivating() {
        let output = "leaving-vpn:vpn:deactivating\n";
        let vpns = parse_active_vpns(output);
        assert_eq!(vpns.len(), 1);
        assert_eq!(vpns[0].state, NmVpnState::Deactivating);
    }

    #[test]
    fn test_parse_active_vpns_unknown_state_ignored() {
        let output = "weird-vpn:vpn:unknown_state\nnormal-vpn:vpn:activated\n";
        let vpns = parse_active_vpns(output);
        assert_eq!(vpns.len(), 1);
        assert_eq!(vpns[0].name, "normal-vpn");
    }

    #[test]
    fn test_parse_active_vpns_empty_output() {
        let output = "";
        let vpns = parse_active_vpns(output);
        assert!(vpns.is_empty());
    }
}
