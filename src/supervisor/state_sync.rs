//! State synchronization utilities
//!
//! Methods for synchronizing VpnSupervisor state with:
//! - The state machine
//! - The shared state (for tray access)
//! - The system tray UI
//! - Desktop notifications

use log::{debug, warn};
use notify_rust::Notification;

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

    /// Update kill switch based on current VPN state (call after state transitions)
    #[allow(dead_code)]
    pub(crate) async fn update_kill_switch_for_state(&mut self) {
        use log::info;

        // Only act if kill switch is enabled in config
        if !self.app_config.kill_switch_enabled {
            return;
        }

        match &self.machine.state {
            VpnState::Connected { .. } | VpnState::Degraded { .. } => {
                // Enable/update kill switch when connected
                if !self.kill_switch.is_enabled() {
                    info!("VPN connected - enabling kill switch");
                    if let Err(e) = self.kill_switch.enable().await {
                        warn!("Failed to enable kill switch: {}", e);
                    }
                } else if let Err(e) = self.kill_switch.update().await {
                    warn!("Failed to update kill switch: {}", e);
                }
            }
            VpnState::Disconnected => {
                // Keep kill switch enabled when disconnected (blocks all traffic)
                // This is the core kill switch behavior - prevent leaks when VPN drops
                if self.kill_switch.is_enabled() {
                    debug!("Kill switch active: blocking non-VPN traffic until VPN reconnects");
                }
            }
            _ => {
                // Connecting/Reconnecting/Failed - keep current rules
            }
        }
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
}
