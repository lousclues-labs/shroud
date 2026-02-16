// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! State synchronization utilities
//!
//! Methods for synchronizing VpnSupervisor state with:
//! - The state machine
//! - The shared state (for tray access)
//! - The system tray UI
//! - Desktop notifications
//! - NetworkManager (source of truth)

use tracing::{debug, info, instrument};

use crate::state::{Event, TransitionReason, VpnState};

impl super::VpnSupervisor {
    /// Dispatch an event to the state machine and sync the shared state
    pub(crate) fn dispatch(&mut self, event: Event) -> Option<TransitionReason> {
        let reason = self.machine.handle_event(event);

        // Reset health checker when we successfully connect
        if let VpnState::Connected { ref server } = self.machine.state {
            self.health_checker.reset();

            // Save last connected server to config
            if self.config_store.config.last_server.as_ref() != Some(server) {
                self.config_store.config.last_server = Some(server.clone());
                self.config_store.save();
            }
        }

        // Best-effort sync: try_write() avoids blocking the synchronous dispatch
        // path. The subsequent sync_shared_state().await guarantees eventual consistency.
        if let Ok(mut state) = self.shared_state.try_write() {
            state.state = self.machine.state.clone();
        } else {
            debug!("dispatch: shared_state write lock contended, deferring to sync_shared_state");
        }

        reason
    }

    /// Sync the shared state with current machine state (for async contexts)
    #[instrument(skip(self))]
    pub(crate) async fn sync_shared_state(&self) {
        let mut state = self.shared_state.write().await;
        state.state = self.machine.state.clone();
    }

    /// Sync internal state with NetworkManager reality
    ///
    /// This is the critical function for handling external VPN changes.
    /// It queries NetworkManager for the actual VPN state and updates
    /// our internal state to match, handling all edge cases.
    ///
    /// Returns true if state was corrected, false if already in sync.
    #[instrument(skip(self))]
    pub(crate) async fn sync_state_from_nm(&mut self) -> bool {
        let active = self.nm.get_active_vpn().await;
        let auto_reconnect = self.shared_state.read().await.auto_reconnect;

        match (&self.machine.state, active) {
            // We think we're disconnected, but VPN is active
            (VpnState::Disconnected, Some(ref conn)) => {
                info!(
                    "State sync: VPN '{}' is active but we thought disconnected",
                    conn
                );
                self.dispatch(Event::NmVpnUp {
                    server: conn.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                true
            }

            // We think we're connected, but no VPN active
            (VpnState::Connected { ref server }, None) => {
                info!(
                    "State sync: VPN '{}' is not active but we thought connected",
                    server
                );
                // Don't auto-reconnect during sync - let normal D-Bus events handle that
                self.machine
                    .set_state(VpnState::Disconnected, TransitionReason::ExternalChange);
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                true
            }

            // We think we're connected to A, but B is active
            (VpnState::Connected { ref server }, Some(ref conn)) if server != conn => {
                info!(
                    "State sync: Different VPN active ('{}' vs '{}'), user switched manually",
                    conn, server
                );
                self.dispatch(Event::NmVpnUp {
                    server: conn.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                true
            }

            // We're reconnecting, but VPN is already active
            (VpnState::Reconnecting { .. }, Some(ref conn)) => {
                info!(
                    "State sync: VPN '{}' connected during reconnect attempt",
                    conn
                );
                self.timing.reconnect_cancelled = true;
                self.dispatch(Event::NmVpnUp {
                    server: conn.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                true
            }

            // We're in failed state but VPN is now active
            (VpnState::Failed { .. }, Some(ref conn)) => {
                info!("State sync: VPN '{}' recovered from failed state", conn);
                self.dispatch(Event::NmVpnUp {
                    server: conn.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                true
            }

            // We think we're degraded, but a different VPN is active — user switched
            (VpnState::Degraded { ref server }, Some(ref conn)) if server != conn => {
                info!(
                    "State sync: Different VPN '{}' active while degraded on '{}', user switched",
                    conn, server
                );
                self.dispatch(Event::NmVpnUp {
                    server: conn.clone(),
                });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                true
            }

            // We think we're degraded, but VPN is gone — silent death
            (VpnState::Degraded { ref server }, None) => {
                info!("State sync: VPN '{}' died while in Degraded state", server);
                self.machine
                    .set_state(VpnState::Disconnected, TransitionReason::VpnLost);
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                true
            }

            // We're connecting but no VPN and not auto-reconnecting
            (VpnState::Connecting { .. }, None) if !auto_reconnect => {
                info!("State sync: Connection attempt seems to have failed");
                self.machine
                    .set_state(VpnState::Disconnected, TransitionReason::Timeout);
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                true
            }

            // States match or are transitional, nothing to do
            _ => {
                debug!("State sync: internal state matches NetworkManager");
                false
            }
        }
    }

    /// Sync kill switch internal state with actual iptables state
    pub(crate) fn sync_killswitch_state(&mut self) {
        self.kill_switch.sync_state();
        // Update shared state if needed
        if let Ok(mut state) = self.shared_state.try_write() {
            state.kill_switch = self.kill_switch.is_enabled();
        }
    }
}
