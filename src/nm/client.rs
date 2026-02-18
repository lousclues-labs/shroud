// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! NetworkManager interface via nmcli
//!
//! Provides functions for interacting with NetworkManager to manage VPN connections.
//! Currently uses nmcli subprocess calls. Future: migrate to D-Bus for event-driven updates.

use std::process::Stdio;
use thiserror::Error;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

use crate::state::{ActiveVpnInfo, NmVpnState};

/// Errors that can occur during NetworkManager operations.
#[derive(Error, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum NmError {
    /// nmcli command failed with error message
    #[error("nmcli failed: {0}")]
    Command(String),

    /// nmcli command timed out
    #[error("nmcli timed out after {0} seconds")]
    Timeout(u64),

    /// Failed to execute nmcli
    #[error("Failed to execute nmcli: {0}")]
    Execution(#[source] std::io::Error),

    /// All disconnect methods failed
    #[error("Failed to disconnect: all methods exhausted")]
    Disconnect,
}

/// Timeout for nmcli commands in seconds
const NMCLI_TIMEOUT_SECS: u64 = 30;

/// Run nmcli and return the output, handling timeout and errors
async fn run_nmcli(args: &[&str]) -> Option<std::process::Output> {
    let nmcli = super::nmcli_command();
    match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new(&nmcli)
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

            if conn_type == "vpn" || conn_type == "wireguard" {
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
        if parts.len() >= 2 && (parts[0] == "vpn" || parts[0] == "wireguard") {
            connections.push(parts[1].to_string());
        }
    }
    connections
}

/// Parse VPN UUID from nmcli "UUID:NAME:TYPE" output for a specific connection.
///
/// Handles VPN names containing colons by splitting UUID on the first `:` (UUIDs
/// are fixed 36-char format with no colons in the value) and type on the last `:`.
fn parse_vpn_uuid(stdout: &str, connection_name: &str) -> Option<String> {
    for line in stdout.lines() {
        // UUID is the first 36 chars (8-4-4-4-12 hex), followed by ':'
        // Format: UUID:NAME:TYPE
        // Split on first ':' to isolate UUID, then rsplitn(2) on rest for type + name
        if let Some((uuid, rest)) = line.split_once(':') {
            if let Some((name, conn_type)) = rest.rsplit_once(':') {
                if (conn_type == "vpn" || conn_type == "wireguard") && name == connection_name {
                    return Some(uuid.to_string());
                }
            }
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

    debug!(
        "nmcli active connections: {}",
        stdout.trim().replace('\n', " | ")
    );

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

/// Check if a specific VPN connection is currently active
pub async fn is_connection_active(connection_name: &str) -> bool {
    match get_active_vpn().await {
        Some(active) => active == connection_name,
        None => false,
    }
}

/// Connect to a VPN via NetworkManager.
///
/// Handles race conditions gracefully:
/// - If connection is already active, returns Ok (success)
/// - If a different VPN is active, proceeds with connection (NM will handle it)
///
/// # Errors
///
/// Returns [`NmError::Timeout`] if `nmcli` does not respond within the configured timeout.
///
/// Returns [`NmError::Execution`] if the `nmcli` binary cannot be executed (missing or not in `$PATH`).
///
/// Returns [`NmError::Command`] if `nmcli` returns a non-zero status (e.g., connection not found).
pub async fn connect(connection_name: &str) -> Result<(), NmError> {
    info!("Activating VPN connection: {}", connection_name);

    // Pre-check: if already connected to this VPN, consider it success
    if is_connection_active(connection_name).await {
        info!(
            "VPN '{}' is already active, no action needed",
            connection_name
        );
        return Ok(());
    }

    let nmcli = super::nmcli_command();
    let output = match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new(&nmcli)
            .args(["con", "up", connection_name])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            error!("Failed to execute nmcli: {}", e);
            return Err(NmError::Execution(e));
        }
        Err(_) => {
            error!("nmcli timed out after {} seconds", NMCLI_TIMEOUT_SECS);
            return Err(NmError::Timeout(NMCLI_TIMEOUT_SECS));
        }
    };

    if output.status.success() {
        info!("VPN activation successful");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim().to_string();

        // Handle "already active" as success - race condition resolved
        if msg.contains("already active") || msg.contains("already activated") {
            info!(
                "VPN '{}' is already active (race condition resolved)",
                connection_name
            );
            return Ok(());
        }

        error!("nmcli failed: {}", msg);
        Err(NmError::Command(msg))
    }
}

/// Disconnect a VPN via NetworkManager.
///
/// # Errors
///
/// Returns [`NmError::Timeout`] if `nmcli` does not respond within the configured timeout.
///
/// Returns [`NmError::Execution`] if the `nmcli` binary cannot be executed (missing or not in `$PATH`).
///
/// Returns [`NmError::Command`] if `nmcli` returns a non-zero status.
///
/// Returns [`NmError::Disconnect`] if all disconnect methods (nmcli + pkill fallbacks) are exhausted.
pub async fn disconnect(connection_name: &str) -> Result<(), NmError> {
    info!("Deactivating VPN connection: {}", connection_name);

    // First, try to get UUID for more reliable disconnection
    let uuid_opt = get_vpn_uuid(connection_name).await;

    // Try disconnecting by UUID first (more reliable)
    if let Some(uuid) = uuid_opt {
        debug!("Attempting disconnect by UUID: {}", uuid);
        let output_result = timeout(
            Duration::from_secs(NMCLI_TIMEOUT_SECS),
            Command::new(super::nmcli_command())
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
        Command::new(super::nmcli_command())
            .args(["con", "down", connection_name])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            error!("Failed to execute nmcli: {}", e);
            return Err(NmError::Execution(e));
        }
        Err(_) => {
            let msg = format!("nmcli timed out after {} seconds", NMCLI_TIMEOUT_SECS);
            error!("{}", msg);
            return Err(NmError::Timeout(NMCLI_TIMEOUT_SECS));
        }
    };

    if output.status.success() {
        info!("VPN deactivation by name successful");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Sometimes nmcli says "Connection '...' is not active" - we consider that a success
        if stderr.contains("not active") {
            warn!("VPN was not active");
            return Ok(());
        }

        warn!("Disconnect by name also failed: {}", stderr.trim());

        // Last resort: Try device-level disconnect
        debug!("Attempting device-level disconnect as last resort");
        disconnect_vpn_device().await
    }
}

/// Disconnect VPN by finding and disconnecting the tun device
async fn disconnect_vpn_device() -> Result<(), NmError> {
    let dev_output = match timeout(
        Duration::from_secs(NMCLI_TIMEOUT_SECS),
        Command::new(super::nmcli_command())
            .args(["-t", "-f", "DEVICE,TYPE", "dev"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            error!("Failed to list devices: {}", e);
            return Err(NmError::Execution(e));
        }
        Err(_) => {
            error!("Device list timed out");
            return Err(NmError::Timeout(NMCLI_TIMEOUT_SECS));
        }
    };

    if dev_output.status.success() {
        let dev_stdout = String::from_utf8_lossy(&dev_output.stdout);
        for line in dev_stdout.lines() {
            // SECURITY: rsplitn for colon-safe parsing (SHROUD-VULN-027)
            let parts: Vec<&str> = line.rsplitn(2, ':').collect();
            if parts.len() >= 2 && parts[0] == "tun" {
                // rsplitn reverses: [type, name]
                let device = parts[1];
                debug!("Found VPN device: {}, attempting disconnect", device);

                let disconnect_output = match timeout(
                    Duration::from_secs(NMCLI_TIMEOUT_SECS),
                    Command::new(super::nmcli_command())
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

    error!("All disconnect methods failed");
    Err(NmError::Disconnect)
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
        let output =
            "my-vpn:vpn:activated\nwg-us:wireguard:activated\nwifi:802-11-wireless:activated\n";
        let vpns = parse_active_vpns(output);
        assert_eq!(vpns.len(), 2);
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
        let output = "vpn1:vpn:activated\nwg1:wireguard:deactivating\n";
        let vpns = parse_active_vpns(output);
        assert_eq!(vpns.len(), 2);
    }

    // --- parse_vpn_connections tests ---

    #[test]
    fn test_parse_vpn_connections_basic() {
        let output = "my-vpn:vpn\nwg-us:wireguard\nwifi:802-11-wireless\nwork-vpn:vpn\n";
        let connections = parse_vpn_connections(output);
        assert_eq!(connections, vec!["my-vpn", "wg-us", "work-vpn"]);
    }

    #[test]
    fn test_parse_vpn_connections_with_colons_in_name() {
        let output = "vpn:server:east:vpn\nwg:server:wireguard\nregular-vpn:vpn\n";
        let connections = parse_vpn_connections(output);
        assert_eq!(
            connections,
            vec!["vpn:server:east", "wg:server", "regular-vpn"]
        );
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

    #[cfg(test)]
    mod security_tests {
        use super::*;

        #[test]
        fn test_vpn_name_with_shell_metacharacters() {
            // VPN names from nmcli should be safe to use
            let dangerous_names = vec![
                ("normal-vpn", true),
                ("vpn with spaces", true),
                ("vpn; rm -rf /", true),
                ("$(whoami)", true),
                ("", false),
            ];

            for (name, should_accept) in dangerous_names {
                println!(
                    "Name {:?} should be {}",
                    name,
                    if should_accept {
                        "accepted"
                    } else {
                        "rejected"
                    }
                );
            }
        }

        #[test]
        fn test_parse_vpn_connections_malicious_output() {
            let long_name = format!("{}:vpn\n", "A".repeat(10000));
            let malicious_outputs = vec![
                "my-vpn:vpn\n",
                "",
                "\n\n\n",
                "vpn:with:colons:vpn\n",
                &long_name,
                "vpn\x00hidden:vpn\n",
                "$(whoami):vpn\n",
                "; rm -rf /:vpn\n",
            ];

            for output in malicious_outputs {
                let result = std::panic::catch_unwind(|| parse_vpn_connections(output));

                assert!(
                    result.is_ok(),
                    "Parser panicked on: {:?}",
                    output.chars().take(50).collect::<String>()
                );
            }
        }

        #[test]
        fn test_parse_active_vpns_invalid_states() {
            let outputs = vec![
                "my-vpn:vpn:activated\n",
                "my-vpn:vpn:activating\n",
                "my-vpn:vpn:deactivating\n",
                "my-vpn:vpn:unknown\n",
                "my-vpn:vpn:\n",
                "my-vpn:vpn:ACTIVATED\n",
                "my-vpn:vpn:12345\n",
            ];

            for output in outputs {
                let result = std::panic::catch_unwind(|| parse_active_vpns(output));

                assert!(result.is_ok(), "Parser panicked on: {:?}", output);
            }
        }
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
