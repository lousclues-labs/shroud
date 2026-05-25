// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! NetworkManager event handlers

use tokio::time::{sleep, Duration};
use tracing::{debug, info, instrument, warn};

use crate::state::{Event, NmVpnState, TransitionReason, VpnState};

use super::super::POST_DISCONNECT_GRACE_SECS;

impl super::super::VpnSupervisor {
    /// Initial sync with NetworkManager on startup
    #[instrument(skip(self))]
    pub(crate) async fn initial_nm_sync(&mut self) {
        // First, check for and clean up multiple simultaneous VPNs.
        // NOTE: Keeps the first VPN reported by NM (arbitrary nmcli order),
        // unlike the D-Bus handler which keeps the newest (most recently activated).
        // Multi-VPN-at-boot is rare; this gives deterministic cleanup.
        let all_vpns = self.nm.get_all_active_vpns().await;
        if all_vpns.len() > 1 {
            warn!(
                "Found {} VPNs active on startup, cleaning up extras",
                all_vpns.len()
            );
            for extra_vpn in &all_vpns[1..] {
                warn!("Disconnecting extra VPN: {}", extra_vpn.name);
                let _ = self.nm.disconnect(&extra_vpn.name).await;
            }
            // Wait a moment for disconnect to complete
            sleep(Duration::from_secs(1)).await;
        }

        let active_vpn_info = self.nm.get_active_vpn_with_state().await;

        if let Some(info) = active_vpn_info {
            match info.state {
                NmVpnState::Activated => {
                    info!("Initial sync: VPN {} is active", info.name);
                    self.dispatch(Event::NmVpnUp { server: info.name });
                }
                NmVpnState::Activating => {
                    info!("Initial sync: VPN {} is activating", info.name);
                    self.dispatch(Event::UserEnable { server: info.name });
                }
                _ => {}
            }
        }

        self.sync_shared_state().await;
        self.tray.update(&self.shared_state);
    }

    /// Poll NetworkManager state and dispatch appropriate events
    pub(crate) async fn poll_nm_state(&mut self) {
        // v2.4.1: The per-poll `sync_killswitch_state()` call was removed.
        // Previously this issued a blocking `sudo nft list table inet
        // shroud_killswitch` every NM_POLL_INTERVAL_SECS (2s), producing
        // ~26,700 sudo invocations per 14h of uptime, ~2% sustained CPU,
        // and a flooded auth journal — all to detect a desync that the
        // 30s health-check path already catches with adequate latency.
        // The shutdown handlers still call `sync_state()` explicitly,
        // and the supervisor constructor still does a one-shot startup
        // sync to detect rules left over from a previous session.

        // CRITICAL: Skip polling entirely while a VPN switch is in progress
        if self.switch_ctx.in_progress {
            debug!("Skipping NM poll during VPN switch");
            return;
        }

        // Check if we're in grace period after intentional disconnect
        if let Some(disconnect_time) = self.timing.last_disconnect_time {
            if disconnect_time.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                debug!("In grace period after intentional disconnect");
                return;
            } else {
                self.timing.last_disconnect_time = None;
            }
        }

        // CRITICAL: Detect multiple simultaneous VPNs and clean up extras
        let all_vpns = self.nm.get_all_active_vpns().await;
        if all_vpns.len() > 1 {
            warn!(
                "Poll detected {} VPNs active: {:?}",
                all_vpns.len(),
                all_vpns.iter().map(|v| &v.name).collect::<Vec<_>>()
            );

            // Determine which VPN to keep:
            // 1. If our state says we're connected to one of them, keep that one
            // 2. Otherwise keep the first one (most recently activated)
            let keep_vpn = if let Some(our_server) = self.machine.state.server_name() {
                if all_vpns.iter().any(|v| v.name == our_server) {
                    our_server.to_string()
                } else {
                    all_vpns[0].name.clone()
                }
            } else {
                all_vpns[0].name.clone()
            };

            info!("Keeping VPN '{}', disconnecting others", keep_vpn);
            for vpn in &all_vpns {
                if vpn.name != keep_vpn {
                    warn!("Disconnecting extra VPN: {}", vpn.name);
                    let _ = self.nm.disconnect(&vpn.name).await;
                }
            }

            // Update our state to match the kept VPN
            if self.machine.state.server_name() != Some(&keep_vpn) {
                info!("Updating state to match kept VPN: {}", keep_vpn);
                self.dispatch(Event::NmVpnUp { server: keep_vpn });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
            }
            return; // Don't run the rest of the poll logic
        }

        let active_vpn_info = self.nm.get_active_vpn_with_state().await;
        let current_state = self.machine.state.clone();
        let auto_reconnect = self.shared_state.read().await.auto_reconnect;

        // Determine what event to dispatch based on NM state vs our state
        match (&current_state, &active_vpn_info) {
            // We think we're connected, but NM shows nothing -> VPN dropped
            (VpnState::Connected { server }, None) => {
                warn!("Connection to {} dropped unexpectedly", server);
                if auto_reconnect {
                    info!("Auto-reconnect enabled, will attempt reconnection");
                    let server_clone = server.clone();
                    self.dispatch(Event::NmVpnDown);
                    self.sync_shared_state().await;
                    self.tray.update(&self.shared_state);
                    self.tray
                        .notify("VPN Disconnected", "Connection dropped, reconnecting...");
                    self.attempt_reconnect(&server_clone).await;
                } else {
                    // Auto-reconnect disabled: go directly to Disconnected, not Reconnecting
                    self.machine
                        .set_state(VpnState::Disconnected, TransitionReason::VpnLost);
                    self.sync_shared_state().await;
                    self.tray.update(&self.shared_state);
                    self.tray
                        .notify("VPN Disconnected", &format!("Disconnected from {}", server));
                }
            }

            // We think we're connected to X, but NM shows Y -> external switch
            (VpnState::Connected { server: our_server }, Some(info))
                if info.state == NmVpnState::Activated && &info.name != our_server =>
            {
                info!(
                    "VPN changed externally from {} to {}",
                    our_server, info.name
                );
                self.dispatch(Event::NmVpnChanged {
                    server: info.name.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
            }

            // We're disconnected but NM shows a VPN -> external connection
            (VpnState::Disconnected, Some(info)) if info.state == NmVpnState::Activated => {
                info!("Detected external VPN connection: {}", info.name);
                self.dispatch(Event::NmVpnUp {
                    server: info.name.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
            }

            // We're disconnected but NM shows activating -> external activation
            (VpnState::Disconnected, Some(info)) if info.state == NmVpnState::Activating => {
                info!("Detected external VPN activation: {}", info.name);
                self.dispatch(Event::UserEnable {
                    server: info.name.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
            }

            // We're connecting and NM confirms it's up -> success
            (VpnState::Connecting { server: target }, Some(info))
                if info.state == NmVpnState::Activated && &info.name == target =>
            {
                info!("Connection to {} confirmed by NM poll", target);
                self.dispatch(Event::NmVpnUp {
                    server: info.name.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
            }

            // We're in Failed state but NM shows connected -> recovered
            (VpnState::Failed { .. }, Some(info)) if info.state == NmVpnState::Activated => {
                info!("VPN recovered, now connected to {}", info.name);
                self.dispatch(Event::NmVpnUp {
                    server: info.name.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
            }

            // Everything else: no event needed
            _ => {}
        }
    }

    /// Force a complete state resync with NetworkManager (after wake from sleep)
    pub(crate) async fn force_state_resync(&mut self) {
        info!("Forcing complete state resync with NetworkManager");
        self.timing.last_disconnect_time = None;
        self.refresh_connections().await;

        let active_vpn_info = self.nm.get_active_vpn_with_state().await;

        // Force set the state based on what NM reports
        match active_vpn_info {
            Some(info) => match info.state {
                NmVpnState::Activated => {
                    info!("Resync: VPN {} is fully active", info.name);
                    self.machine.set_state(
                        VpnState::Connected { server: info.name },
                        TransitionReason::WakeResync,
                    );
                }
                NmVpnState::Activating => {
                    info!("Resync: VPN {} is activating", info.name);
                    self.machine.set_state(
                        VpnState::Connecting { server: info.name },
                        TransitionReason::WakeResync,
                    );
                }
                _ => {
                    info!("Resync: No active VPN");
                    self.machine
                        .set_state(VpnState::Disconnected, TransitionReason::WakeResync);
                }
            },
            None => {
                if !self.machine.state.is_busy() {
                    info!("Resync: No VPN detected");
                    self.machine
                        .set_state(VpnState::Disconnected, TransitionReason::WakeResync);
                }
            }
        }

        self.sync_shared_state().await;
        self.tray.update(&self.shared_state);
    }

    /// Refresh the list of available VPN connections
    #[instrument(skip(self))]
    pub(crate) async fn refresh_connections(&mut self) {
        info!("Refreshing VPN connections");
        let connections = self.nm.list_vpn_connections().await;
        {
            let mut state = self.shared_state.write().await;
            state.connections = connections;
        }
        self.tray.update(&self.shared_state);
    }
}
