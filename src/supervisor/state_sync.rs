//! State synchronization utilities
//!
//! Methods for synchronizing VpnSupervisor state with:
//! - The state machine
//! - The shared state (for tray access)
//! - The system tray UI
//! - Desktop notifications
//! - NetworkManager (source of truth)

use log::{debug, info, warn};
use notify_rust::Notification;

use crate::nm::get_active_vpn as nm_get_active_vpn;
use crate::state::{Event, TransitionReason, VpnState};
use crate::tray::VpnTray;

impl super::VpnSupervisor {
    /// Dispatch an event to the state machine and sync the shared state
    pub(crate) fn dispatch(&mut self, event: Event) -> Option<TransitionReason> {
        let reason = self.machine.handle_event(event);

        // Reset health checker when we successfully connect
        if let VpnState::Connected { ref server } = self.machine.state {
            self.health_checker.reset();

            // Save last connected server to config
            if self.app_config.last_server.as_ref() != Some(server) {
                self.app_config.last_server = Some(server.clone());
                if let Err(e) = self.config_manager.save(&self.app_config) {
                    warn!("Failed to save last_server to config: {}", e);
                }
            }
        }

        // Always sync shared state after event processing
        if let Ok(mut state) = self.shared_state.try_write() {
            state.state = self.machine.state.clone();
        }

        reason
    }

    /// Sync the shared state with current machine state (for async contexts)
    pub(crate) async fn sync_shared_state(&self) {
        let mut state = self.shared_state.write().await;
        state.state = self.machine.state.clone();
    }

    /// Update the tray icon with current state
    pub(crate) fn update_tray(&self) {
        let current_state = match self.shared_state.try_read() {
            Ok(guard) => {
                debug!(
                    "update_tray: state={:?}, auto_reconnect={}, kill_switch={}",
                    guard.state, guard.auto_reconnect, guard.kill_switch
                );
                guard.clone()
            }
            Err(_) => {
                warn!("update_tray: Failed to read shared_state");
                return;
            }
        };

        let tray_handle = self.tray_handle.clone();
        std::thread::spawn(move || {
            if let Ok(handle_guard) = tray_handle.lock() {
                if let Some(handle) = handle_guard.as_ref() {
                    let result = handle.update(move |tray: &mut VpnTray| {
                        if let Ok(mut cached) = tray.cached_state.write() {
                            debug!("Tray cached_state updated to: {:?}", current_state.state);
                            *cached = current_state.clone();
                        }
                    });
                    if result.is_none() {
                        warn!("Tray handle.update() returned None - service may be shutdown");
                    }
                } else {
                    warn!("Tray handle is None");
                }
            } else {
                warn!("Failed to lock tray_handle");
            }
        });
    }

    /// Show a desktop notification
    pub(crate) fn show_notification(&self, title: &str, body: &str) {
        let title = title.to_string();
        let body = body.to_string();
        std::thread::spawn(move || {
            let _ = Notification::new()
                .summary(&title)
                .body(&body)
                .timeout(5000)
                .show();
        });
    }

    /// Sync internal state with NetworkManager reality
    ///
    /// This is the critical function for handling external VPN changes.
    /// It queries NetworkManager for the actual VPN state and updates
    /// our internal state to match, handling all edge cases.
    ///
    /// Returns true if state was corrected, false if already in sync.
    pub(crate) async fn sync_state_from_nm(&mut self) -> bool {
        let active = nm_get_active_vpn().await;
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
                self.update_tray();
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
                self.update_tray();
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
                self.update_tray();
                true
            }

            // We're reconnecting, but VPN is already active
            (VpnState::Reconnecting { .. }, Some(ref conn)) => {
                info!(
                    "State sync: VPN '{}' connected during reconnect attempt",
                    conn
                );
                self.reconnect_cancelled = true;
                self.dispatch(Event::NmVpnUp {
                    server: conn.clone(),
                });
                self.sync_shared_state().await;
                self.update_tray();
                true
            }

            // We're in failed state but VPN is now active
            (VpnState::Failed { .. }, Some(ref conn)) => {
                info!("State sync: VPN '{}' recovered from failed state", conn);
                self.dispatch(Event::NmVpnUp {
                    server: conn.clone(),
                });
                self.sync_shared_state().await;
                self.update_tray();
                true
            }

            // We're connecting but no VPN and not auto-reconnecting
            (VpnState::Connecting { .. }, None) if !auto_reconnect => {
                info!("State sync: Connection attempt seems to have failed");
                self.machine
                    .set_state(VpnState::Disconnected, TransitionReason::Timeout);
                self.sync_shared_state().await;
                self.update_tray();
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
