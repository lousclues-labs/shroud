//! Supervisor event handlers

use log::{debug, error, info, warn};
use std::time::Instant;
use tokio::time::{sleep, Duration};

use crate::daemon::lock::release_instance_lock;
use crate::dbus::NmEvent;
use crate::health::HealthResult;
use crate::logging;
use crate::nm::{
    connect as nm_connect, disconnect as nm_disconnect,
    get_active_vpn_with_state as nm_get_active_vpn_with_state,
    get_all_active_vpns as nm_get_all_active_vpns, get_vpn_state as nm_get_vpn_state,
    get_vpn_type as nm_get_vpn_type, kill_orphan_openvpn_processes,
    list_vpn_connections as nm_list_vpn_connections,
    list_vpn_connections_with_types as nm_list_vpn_connections_with_types,
};
use crate::state::{Event, NmVpnState, TransitionReason, VpnState};

use super::{
    CONNECTION_MONITOR_INTERVAL_MS, CONNECTION_MONITOR_MAX_ATTEMPTS, DISCONNECT_VERIFY_INTERVAL_MS,
    DISCONNECT_VERIFY_MAX_ATTEMPTS, MAX_CONNECT_ATTEMPTS, POST_DISCONNECT_GRACE_SECS,
    POST_DISCONNECT_SETTLE_SECS,
};

impl super::VpnSupervisor {
    /// Handle D-Bus event from NetworkManager
    pub(crate) async fn handle_dbus_event(&mut self, event: NmEvent) {
        debug!("Received D-Bus event: {:?}", event);

        // CRITICAL: Ignore ALL D-Bus events while a VPN switch is in progress
        // handle_connect manages everything during a switch - D-Bus events only cause interference
        if self.switching_in_progress {
            debug!("Ignoring D-Bus event during VPN switch: {:?}", event);
            return;
        }

        // CRITICAL: Ignore late deactivation events from VPN we recently switched FROM
        // D-Bus events can arrive after we've already connected to the new VPN
        if let Some(ref from_server) = self.switching_from {
            if let NmEvent::VpnDeactivated { ref name } = event {
                if name == from_server {
                    // Check if we're within the grace window after switch completed
                    if let Some(completed) = self.switch_completed_time {
                        if completed.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                            info!(
                                "Ignoring late deactivation event for switched-from VPN: {}",
                                name
                            );
                            return;
                        }
                    }
                    // Clear the switching_from after processing
                    self.switching_from = None;
                    self.switch_completed_time = None;
                }
            }
        }

        // Check if we're in grace period after intentional disconnect
        if let Some(disconnect_time) = self.last_disconnect_time {
            if disconnect_time.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                debug!("Ignoring D-Bus event during grace period");
                return;
            } else {
                self.last_disconnect_time = None;
            }
        }

        let auto_reconnect = self.shared_state.read().await.auto_reconnect;

        match event {
            NmEvent::VpnActivated { name } => {
                info!("D-Bus: VPN '{}' activated", name);

                // CRITICAL: If we already have a different VPN connected, disconnect the OLD one
                // Policy: newest VPN wins (the one that just activated)
                if let Some(current) = self.machine.state.server_name() {
                    if current != name {
                        info!("External VPN '{}' activated while connected to '{}' - disconnecting old VPN", name, current);
                        let old_vpn = current.to_string();
                        // Update our state to the new VPN first
                        self.dispatch(Event::NmVpnUp {
                            server: name.clone(),
                        });
                        self.sync_shared_state().await;
                        self.update_tray();
                        // Then disconnect the old one
                        if let Err(e) = nm_disconnect(&old_vpn).await {
                            warn!("Failed to disconnect old VPN '{}': {}", old_vpn, e);
                        }
                        self.show_notification(
                            "VPN Switched",
                            &format!("Now connected to {}", name),
                        );
                        return;
                    }
                }

                // Also check for any other active VPNs in NetworkManager
                let all_active = nm_get_all_active_vpns().await;
                if all_active.len() > 1 {
                    info!(
                        "Multiple VPNs detected ({}) - cleaning up extras",
                        all_active.len()
                    );
                    for vpn in &all_active {
                        if vpn.name != name {
                            info!("Disconnecting extra VPN: {}", vpn.name);
                            let _ = nm_disconnect(&vpn.name).await;
                        }
                    }
                }

                self.dispatch(Event::NmVpnUp { server: name });
                self.sync_shared_state().await;
                self.update_tray();
            }
            NmEvent::VpnActivating { name } => {
                // Only update if we're not already aware of this activation
                if !matches!(&self.machine.state, VpnState::Connecting { server } if server == &name)
                {
                    info!("D-Bus: VPN '{}' activating (external)", name);
                    self.dispatch(Event::UserEnable { server: name });
                    self.sync_shared_state().await;
                    self.update_tray();
                }
            }
            NmEvent::VpnDeactivated { name } => {
                info!("D-Bus: VPN '{}' deactivated", name);

                // Check if this was our connected VPN
                if let Some(current) = self.machine.state.server_name() {
                    if current == name {
                        if auto_reconnect
                            && matches!(
                                self.machine.state,
                                VpnState::Connected { .. } | VpnState::Degraded { .. }
                            )
                        {
                            let server = name.clone();
                            self.dispatch(Event::NmVpnDown);
                            self.sync_shared_state().await;
                            self.update_tray();
                            self.show_notification(
                                "VPN Disconnected",
                                "Connection dropped, reconnecting...",
                            );
                            self.attempt_reconnect(&server).await;
                        } else {
                            // Auto-reconnect disabled: go directly to Disconnected, not Reconnecting
                            self.machine
                                .set_state(VpnState::Disconnected, TransitionReason::VpnLost);
                            self.sync_shared_state().await;
                            self.update_tray();
                            self.show_notification(
                                "VPN Disconnected",
                                &format!("Disconnected from {}", name),
                            );
                        }
                    }
                }
            }
            NmEvent::VpnFailed { name, reason } => {
                warn!("D-Bus: VPN '{}' failed: {}", name, reason);

                if auto_reconnect {
                    self.dispatch(Event::NmVpnDown);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification("VPN Failed", &format!("{}: {}", name, reason));
                    self.attempt_reconnect(&name).await;
                } else {
                    self.machine.set_state(
                        VpnState::Failed {
                            server: name,
                            reason,
                        },
                        TransitionReason::VpnLost,
                    );
                    self.sync_shared_state().await;
                    self.update_tray();
                }
            }
            NmEvent::ConnectivityChanged { connected } => {
                debug!("D-Bus: Connectivity changed: {}", connected);
                // Could trigger health check here
            }
        }
    }

    /// Initial sync with NetworkManager on startup
    pub(crate) async fn initial_nm_sync(&mut self) {
        // First, check for and clean up multiple simultaneous VPNs
        let all_vpns = nm_get_all_active_vpns().await;
        if all_vpns.len() > 1 {
            warn!(
                "Found {} VPNs active on startup, cleaning up extras",
                all_vpns.len()
            );
            for extra_vpn in &all_vpns[1..] {
                warn!("Disconnecting extra VPN: {}", extra_vpn.name);
                let _ = nm_disconnect(&extra_vpn.name).await;
            }
            // Wait a moment for disconnect to complete
            sleep(Duration::from_secs(1)).await;
        }

        let active_vpn_info = nm_get_active_vpn_with_state().await;

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
        self.update_tray();
    }

    /// Poll NetworkManager state and dispatch appropriate events
    pub(crate) async fn poll_nm_state(&mut self) {
        // CRITICAL: Skip polling entirely while a VPN switch is in progress
        if self.switching_in_progress {
            debug!("Skipping NM poll during VPN switch");
            return;
        }

        // Check if we're in grace period after intentional disconnect
        if let Some(disconnect_time) = self.last_disconnect_time {
            if disconnect_time.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                debug!("In grace period after intentional disconnect");
                return;
            } else {
                self.last_disconnect_time = None;
            }
        }

        // CRITICAL: Detect multiple simultaneous VPNs and clean up extras
        let all_vpns = nm_get_all_active_vpns().await;
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
                    let _ = nm_disconnect(&vpn.name).await;
                }
            }

            // Update our state to match the kept VPN
            if self.machine.state.server_name() != Some(&keep_vpn) {
                info!("Updating state to match kept VPN: {}", keep_vpn);
                self.dispatch(Event::NmVpnUp { server: keep_vpn });
                self.sync_shared_state().await;
                self.update_tray();
            }
            return; // Don't run the rest of the poll logic
        }

        let active_vpn_info = nm_get_active_vpn_with_state().await;
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
                    self.update_tray();
                    self.show_notification(
                        "VPN Disconnected",
                        "Connection dropped, reconnecting...",
                    );
                    self.attempt_reconnect(&server_clone).await;
                } else {
                    // Auto-reconnect disabled: go directly to Disconnected, not Reconnecting
                    self.machine
                        .set_state(VpnState::Disconnected, TransitionReason::VpnLost);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification(
                        "VPN Disconnected",
                        &format!("Disconnected from {}", server),
                    );
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
                self.update_tray();
            }

            // We're disconnected but NM shows a VPN -> external connection
            (VpnState::Disconnected, Some(info)) if info.state == NmVpnState::Activated => {
                info!("Detected external VPN connection: {}", info.name);
                self.dispatch(Event::NmVpnUp {
                    server: info.name.clone(),
                });
                self.sync_shared_state().await;
                self.update_tray();
            }

            // We're disconnected but NM shows activating -> external activation
            (VpnState::Disconnected, Some(info)) if info.state == NmVpnState::Activating => {
                info!("Detected external VPN activation: {}", info.name);
                self.dispatch(Event::UserEnable {
                    server: info.name.clone(),
                });
                self.sync_shared_state().await;
                self.update_tray();
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
                self.update_tray();
            }

            // We're in Failed state but NM shows connected -> recovered
            (VpnState::Failed { .. }, Some(info)) if info.state == NmVpnState::Activated => {
                info!("VPN recovered, now connected to {}", info.name);
                self.dispatch(Event::NmVpnUp {
                    server: info.name.clone(),
                });
                self.sync_shared_state().await;
                self.update_tray();
            }

            // Everything else: no event needed
            _ => {}
        }
    }

    /// Force a complete state resync with NetworkManager (after wake from sleep)
    pub(crate) async fn force_state_resync(&mut self) {
        info!("Forcing complete state resync with NetworkManager");
        self.last_disconnect_time = None;
        self.refresh_connections().await;

        let active_vpn_info = nm_get_active_vpn_with_state().await;

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
        self.update_tray();
    }

    /// Run health check when connected
    pub(crate) async fn run_health_check(&mut self) {
        // CRITICAL: First sync with NetworkManager state
        // This catches external VPN changes before we do health checks
        if self.sync_state_from_nm().await {
            debug!("State corrected during health check, skipping health check");
            return;
        }

        // Also sync kill switch state periodically
        self.sync_killswitch_state();

        // Only run health checks when in Connected or Degraded state
        let server = match &self.machine.state {
            VpnState::Connected { server } => server.clone(),
            VpnState::Degraded { server } => server.clone(),
            _ => return,
        };

        debug!("Running health check for {}", server);

        let result = self.health_checker.check().await;

        match result {
            HealthResult::Healthy => {
                // If we were degraded, transition back to connected
                if matches!(self.machine.state, VpnState::Degraded { .. }) {
                    info!("Health check passed, VPN recovered from degraded state");
                    self.dispatch(Event::HealthOk);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification("VPN Recovered", "Connection is healthy again");
                } else {
                    debug!("Health check passed");
                }
            }
            HealthResult::Degraded { latency_ms } => {
                if matches!(self.machine.state, VpnState::Connected { .. }) {
                    warn!("Health check degraded: {}ms latency", latency_ms);
                    self.dispatch(Event::HealthDegraded);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification(
                        "VPN Degraded",
                        &format!("High latency: {}ms", latency_ms),
                    );
                }
            }
            HealthResult::Dead { reason } => {
                error!("Health check failed: {}", reason);
                let auto_reconnect = self.shared_state.read().await.auto_reconnect;

                if auto_reconnect {
                    self.dispatch(Event::HealthDead);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification("VPN Dead", "Connection lost, reconnecting...");
                    self.attempt_reconnect(&server).await;
                } else {
                    // Auto-reconnect disabled: go directly to Disconnected, not Reconnecting
                    self.machine
                        .set_state(VpnState::Disconnected, TransitionReason::HealthCheckDead);
                    self.sync_shared_state().await;
                    self.update_tray();
                    self.show_notification("VPN Dead", &reason);
                }
            }
        }
    }

    /// Handle user request to connect to a server
    pub(crate) async fn handle_connect(&mut self, connection_name: &str) {
        info!("Connect requested: {}", connection_name);

        // CRITICAL: Set switching flag to prevent D-Bus events from interfering
        self.switching_in_progress = true;
        self.switching_target = Some(connection_name.to_string());

        // Track the VPN we're switching FROM (to ignore late D-Bus events)
        if let Some(current) = self.machine.state.server_name() {
            if current != connection_name {
                self.switching_from = Some(current.to_string());
            }
        }

        // Set grace period immediately to block any D-Bus deactivation events
        self.last_disconnect_time = Some(Instant::now());

        // NOTE: We do NOT disable kill switch during VPN switch anymore.
        // The kill switch rules already whitelist all VPN server IPs from NetworkManager,
        // so VPN connections should work even with kill switch enabled.
        if self.app_config.kill_switch_enabled && !self.kill_switch.is_enabled() {
            info!("Pre-enabling kill switch before connection");
            if let Err(e) = self.kill_switch.enable().await {
                warn!("Failed to pre-enable kill switch: {}", e);
            } else {
                let mut state = self.shared_state.write().await;
                state.kill_switch = true;
            }
        }

        // STEP 1: ALWAYS check NM for active VPNs first (don't trust our state machine)
        // This catches VPNs that NM still has active even if our state is wrong
        let all_active = nm_get_all_active_vpns().await;
        info!(
            "NM reports {} active VPN(s): {:?}",
            all_active.len(),
            all_active.iter().map(|v| &v.name).collect::<Vec<_>>()
        );

        // Also track any active VPNs as "switching from" to ignore their deactivation events
        for vpn in &all_active {
            if vpn.name != connection_name && self.switching_from.is_none() {
                self.switching_from = Some(vpn.name.clone());
            }
        }

        // Disconnect ALL VPNs that aren't the one we're connecting to
        for vpn in &all_active {
            if vpn.name != connection_name {
                info!("Disconnecting VPN before switch: {}", vpn.name);
                if let Err(e) = nm_disconnect(&vpn.name).await {
                    warn!("Failed to disconnect {}: {}", vpn.name, e);
                }
            }
        }

        // STEP 2: Wait for ALL disconnects to complete (with verification)
        if all_active.iter().any(|v| v.name != connection_name) {
            info!("Waiting for VPN disconnection(s) to complete...");
            for attempt in 1..=DISCONNECT_VERIFY_MAX_ATTEMPTS {
                sleep(Duration::from_millis(DISCONNECT_VERIFY_INTERVAL_MS)).await;
                let remaining = nm_get_all_active_vpns().await;
                let others: Vec<_> = remaining
                    .iter()
                    .filter(|v| v.name != connection_name)
                    .collect();
                if others.is_empty() {
                    info!("All other VPNs disconnected after {} attempts", attempt);
                    break;
                }
                if attempt == DISCONNECT_VERIFY_MAX_ATTEMPTS {
                    warn!(
                        "Disconnect verification timed out after {} attempts",
                        attempt
                    );
                    // Force cleanup
                    for other in &others {
                        warn!("Forcing disconnect of stuck VPN: {}", other.name);
                        let _ = nm_disconnect(&other.name).await;
                    }
                }
                debug!(
                    "Still have {} other active VPN(s), attempt {}",
                    others.len(),
                    attempt
                );
            }

            kill_orphan_openvpn_processes().await;
            sleep(Duration::from_secs(POST_DISCONNECT_SETTLE_SECS)).await;
        }

        // Final verification before connect
        let final_check = nm_get_all_active_vpns().await;
        let other_vpns: Vec<_> = final_check
            .iter()
            .filter(|v| v.name != connection_name)
            .collect();
        if !other_vpns.is_empty() {
            error!(
                "CRITICAL: Still have {} other VPN(s) active before connect: {:?}",
                other_vpns.len(),
                other_vpns.iter().map(|v| &v.name).collect::<Vec<_>>()
            );
        }

        // Dispatch connecting event for new server
        self.dispatch(Event::UserEnable {
            server: connection_name.to_string(),
        });
        self.sync_shared_state().await;
        self.update_tray();

        self.show_notification("VPN", &format!("Connecting to {}...", connection_name));

        // Attempt connection with retries
        let mut connection_succeeded = false;
        for attempt in 1..=MAX_CONNECT_ATTEMPTS {
            debug!(
                "Connection attempt {} of {} for {}",
                attempt, MAX_CONNECT_ATTEMPTS, connection_name
            );

            match nm_connect(connection_name).await {
                Ok(_) => {
                    // Monitor connection state
                    for _ in 1..=CONNECTION_MONITOR_MAX_ATTEMPTS {
                        sleep(Duration::from_millis(CONNECTION_MONITOR_INTERVAL_MS)).await;

                        match nm_get_vpn_state(connection_name).await {
                            Some(NmVpnState::Activated) => {
                                info!("VPN '{}' successfully activated", connection_name);
                                self.dispatch(Event::NmVpnUp {
                                    server: connection_name.to_string(),
                                });
                                self.sync_shared_state().await;
                                self.update_tray();
                                self.show_notification(
                                    "VPN Connected",
                                    &format!("Connected to {}", connection_name),
                                );
                                connection_succeeded = true;
                                break;
                            }
                            Some(NmVpnState::Activating) => {
                                // Still connecting
                            }
                            Some(NmVpnState::Deactivating) | Some(NmVpnState::Inactive) | None => {
                                break;
                            }
                        }
                    }

                    if connection_succeeded {
                        break;
                    }
                    warn!("Connection monitoring timed out");
                }
                Err(e) => {
                    warn!("Connection attempt {} failed: {}", attempt, e);
                }
            }

            if attempt < MAX_CONNECT_ATTEMPTS {
                sleep(Duration::from_secs(2)).await;
            }
        }

        // NOTE: Kill switch stays enabled throughout - no need to re-enable
        // VPN server IPs are already whitelisted in the rules

        // CRITICAL: Clear switching flags - we're done with the switch
        // BUT keep switching_from and set switch_completed_time to ignore late D-Bus events
        self.switching_in_progress = false;
        self.switching_target = None;
        self.last_disconnect_time = None;
        // Set completion time so late D-Bus events for the old VPN are ignored
        self.switch_completed_time = Some(Instant::now());

        if !connection_succeeded {
            // All attempts failed - also clear switching_from since there's nothing to ignore
            self.switching_from = None;
            self.switch_completed_time = None;
            error!(
                "Failed to connect to {} after {} attempts",
                connection_name, MAX_CONNECT_ATTEMPTS
            );
            // Use ConnectionFailed to transition directly to Disconnected
            // (not Timeout, which would go to Reconnecting)
            self.dispatch(Event::ConnectionFailed {
                reason: format!("Failed to connect after {} attempts", MAX_CONNECT_ATTEMPTS),
            });
            self.sync_shared_state().await;
            self.update_tray();
            self.show_notification(
                "VPN Failed",
                &format!("Could not connect to {}", connection_name),
            );
        }
    }

    /// Handle user request to disconnect
    pub(crate) async fn handle_disconnect(&mut self) {
        info!("Disconnect requested");

        // Cancel any ongoing reconnection attempts
        self.reconnect_cancelled = true;

        let connection_name = match self.machine.state.server_name() {
            Some(name) => name.to_string(),
            None => {
                info!("Not connected, nothing to disconnect");
                return;
            }
        };

        self.last_disconnect_time = Some(Instant::now());

        match nm_disconnect(&connection_name).await {
            Ok(_) => {
                info!("Disconnected successfully");

                // CRITICAL: Disable kill switch on intentional disconnect
                // Otherwise user loses all network access
                if self.kill_switch.is_enabled() {
                    info!("Disabling kill switch on user disconnect");
                    if let Err(e) = self.kill_switch.disable().await {
                        warn!("Failed to disable kill switch: {}", e);
                    }
                    // Update config to reflect kill switch is now off
                    self.app_config.kill_switch_enabled = false;
                    if let Err(e) = self.config_manager.save(&self.app_config) {
                        warn!("Failed to save config: {}", e);
                    }
                    // Update shared state
                    {
                        let mut state = self.shared_state.write().await;
                        state.kill_switch = false;
                    }
                }

                self.dispatch(Event::UserDisable);
                self.sync_shared_state().await;
                self.update_tray();
                self.show_notification("VPN Disconnected", "VPN connection closed");
            }
            Err(e) => {
                error!("Failed to disconnect: {}", e);
            }
        }
    }

    /// Restart the application by re-executing the binary
    pub(crate) async fn handle_restart(&mut self) {
        use std::os::unix::process::CommandExt;

        info!("Restart requested");
        self.show_notification("VPN Manager", "Restarting...");

        if self.kill_switch.is_enabled() {
            info!("Disabling kill switch before restart");
            if let Err(e) = self.kill_switch.disable().await {
                error!("Failed to disable kill switch: {}", e);
            }
        }

        let exe_path = match resolve_restart_path() {
            Ok(path) => path,
            Err(message) => {
                error!("{}", message);
                self.show_notification("Restart Failed", &message);
                return;
            }
        };

        info!("Spawning new daemon instance: {:?}", exe_path);

        // Clean up resources BEFORE spawning to avoid conflicts
        release_instance_lock();
        let socket_path = crate::ipc::protocol::socket_path();
        let _ = std::fs::remove_file(&socket_path);

        // Give time for socket to be released
        sleep(Duration::from_millis(100)).await;

        // Spawn detached process that will outlive us
        let mut cmd = std::process::Command::new(&exe_path);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        // CRITICAL: Create new session to fully detach from parent
        // This prevents the child from dying when we exit
        unsafe {
            cmd.pre_exec(|| {
                // Create new session (detach from controlling terminal)
                libc::setsid();
                Ok(())
            });
        }

        match cmd.spawn() {
            Ok(child) => {
                info!("Spawned new daemon (PID: {})", child.id());

                // Give child time to start and bind socket
                sleep(Duration::from_millis(500)).await;

                // Now exit
                info!("Old daemon exiting for restart");
                std::process::exit(0);
            }
            Err(e) => {
                error!("Failed to spawn new instance: {}", e);
                self.show_notification("Restart Failed", &format!("Error: {}", e));
                // Re-acquire lock since spawn failed
                // Note: We can't easily re-acquire, so just warn
                warn!("Lock and socket were released but spawn failed - may need manual restart");
            }
        }
    }

    /// Handle quit command - clean shutdown
    pub(crate) async fn handle_quit(&mut self) {
        info!("Quit requested, cleaning up...");

        // Non-blocking kill switch cleanup with timeout
        if self.kill_switch.is_enabled() {
            info!("Cleaning up kill switch before shutdown");
            match crate::killswitch::cleanup_with_fallback() {
                crate::killswitch::CleanupResult::Cleaned => {
                    info!("Kill switch cleanup successful");
                }
                crate::killswitch::CleanupResult::NothingToClean => {
                    debug!("No kill switch rules to clean");
                }
                crate::killswitch::CleanupResult::Failed(_) => {
                    self.show_notification(
                        "Cleanup Failed",
                        "Firewall rules may need manual cleanup. See logs.",
                    );
                }
            }
            self.kill_switch.sync_state();
        }

        // Show notification
        self.show_notification("Shroud", "Shutting down...");

        // Give notification time to show
        sleep(Duration::from_millis(300)).await;

        info!("Shutdown complete");

        // Clean up and exit the process
        release_instance_lock();
        let socket_path = crate::ipc::protocol::socket_path();
        let _ = std::fs::remove_file(&socket_path);
        std::process::exit(0);
    }

    pub(crate) async fn graceful_shutdown(&mut self) {
        info!("Performing graceful shutdown");

        if self.kill_switch.is_enabled() {
            info!("Cleaning up kill switch before shutdown");
            match crate::killswitch::cleanup_with_fallback() {
                crate::killswitch::CleanupResult::Cleaned => {
                    info!("Kill switch cleanup successful");
                }
                crate::killswitch::CleanupResult::NothingToClean => {
                    debug!("No kill switch rules to clean");
                }
                crate::killswitch::CleanupResult::Failed(_) => {
                    self.show_notification(
                        "Cleanup Failed",
                        "Firewall rules may need manual cleanup. See logs.",
                    );
                }
            }
            self.kill_switch.sync_state();
        }

        release_instance_lock();

        let socket_path = crate::ipc::protocol::socket_path();
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }

        info!("Graceful shutdown complete");
    }

    /// Toggle auto-reconnect setting
    pub(crate) async fn toggle_auto_reconnect(&mut self) {
        info!("toggle_auto_reconnect called");
        let new_value = {
            let mut state = self.shared_state.write().await;
            state.auto_reconnect = !state.auto_reconnect;
            info!(
                "Auto-reconnect toggled in shared_state to: {}",
                state.auto_reconnect
            );
            state.auto_reconnect
        };

        // Save to persistent config
        self.app_config.auto_reconnect = new_value;
        if let Err(e) = self.config_manager.save(&self.app_config) {
            warn!("Failed to save config: {}", e);
        }

        info!("Auto-reconnect toggled to: {}", new_value);
        self.update_tray();
        self.show_notification(
            "Auto-Reconnect",
            if new_value { "Enabled" } else { "Disabled" },
        );
    }

    /// Toggle kill switch (iptables firewall rules that block non-VPN traffic)
    pub(crate) async fn toggle_kill_switch(&mut self) {
        let current_enabled = self.app_config.kill_switch_enabled;
        let new_enabled = !current_enabled;

        // Optimistically update shared state immediately so tray shows new state
        // This prevents the "flicker" where tray briefly shows old state
        {
            let mut state = self.shared_state.write().await;
            state.kill_switch = new_enabled;
        }
        self.update_tray();

        let result = if new_enabled {
            self.kill_switch.enable().await
        } else {
            self.kill_switch.disable().await
        };

        match result {
            Ok(()) => {
                // Save to persistent config
                self.app_config.kill_switch_enabled = new_enabled;
                if let Err(e) = self.config_manager.save(&self.app_config) {
                    warn!("Failed to save config: {}", e);
                }

                info!("Kill switch toggled to: {}", new_enabled);
                self.show_notification(
                    "Kill Switch",
                    if new_enabled {
                        "Enabled - Non-VPN traffic blocked"
                    } else {
                        "Disabled"
                    },
                );
            }
            Err(e) => {
                // Rollback optimistic state update on failure
                {
                    let mut state = self.shared_state.write().await;
                    state.kill_switch = current_enabled; // Revert to original
                }
                self.update_tray();

                if !new_enabled {
                    if let crate::killswitch::firewall::KillSwitchError::Command(msg) = &e {
                        let msg_lower = msg.to_lowercase();
                        if msg_lower.contains("cache initialization failed")
                            || msg_lower.contains("netlink: error")
                            || msg_lower.contains("ip_tables")
                            || msg_lower.contains("can't initialize iptables table")
                            || msg_lower.contains("table does not exist")
                        {
                            warn!(
                                "Kill switch disable encountered iptables error; treating as best-effort: {}",
                                e
                            );

                            {
                                let mut state = self.shared_state.write().await;
                                state.kill_switch = false;
                            }

                            self.app_config.kill_switch_enabled = false;
                            if let Err(save_err) = self.config_manager.save(&self.app_config) {
                                warn!("Failed to save config: {}", save_err);
                            }

                            self.update_tray();
                            self.show_notification("Kill Switch", "Disabled");
                            return;
                        }
                    }
                }

                error!("Failed to toggle kill switch: {}", e);
                self.show_notification("Kill Switch Error", &format!("Failed: {}", e));
            }
        }
    }

    /// Toggle autostart on login
    pub(crate) async fn toggle_autostart(&mut self) {
        match crate::autostart::Autostart::toggle() {
            Ok(enabled) => {
                self.update_tray();
                self.show_notification("Autostart", if enabled { "Enabled" } else { "Disabled" });
            }
            Err(e) => {
                error!("Failed to toggle autostart: {}", e);
                self.show_notification("Autostart Error", &e);
            }
        }
    }

    /// Toggle debug logging to file
    pub(crate) async fn toggle_debug_logging(&mut self) {
        let currently_enabled = logging::is_debug_logging_enabled();

        if currently_enabled {
            logging::disable_debug_logging();
            {
                let mut state = self.shared_state.write().await;
                state.debug_logging = false;
            }
            info!("Debug logging disabled");
            self.update_tray();
            self.show_notification("Debug Logging", "Disabled");
        } else {
            match logging::enable_debug_logging() {
                Ok(path) => {
                    {
                        let mut state = self.shared_state.write().await;
                        state.debug_logging = true;
                    }
                    info!("Debug logging enabled to {:?}", path);
                    self.update_tray();
                    self.show_notification(
                        "Debug Logging",
                        &format!("Enabled. Logs: {}", path.display()),
                    );
                }
                Err(e) => {
                    error!("Failed to enable debug logging: {}", e);
                    self.show_notification("Debug Logging Error", &e);
                }
            }
        }
    }

    /// Open the log file in the default viewer
    pub(crate) fn open_log_file(&self) {
        match logging::open_log_file() {
            Ok(()) => {
                debug!("Opened log file");
            }
            Err(e) => {
                warn!("Failed to open log file: {}", e);
                self.show_notification("Log File", &e);
            }
        }
    }

    /// Refresh the list of available VPN connections
    pub(crate) async fn refresh_connections(&mut self) {
        info!("Refreshing VPN connections");
        let connections = nm_list_vpn_connections().await;
        {
            let mut state = self.shared_state.write().await;
            state.connections = connections;
        }
        self.update_tray();
    }

    async fn reload_configuration(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Reloading configuration from disk");
        let new_config = self.config_manager.load_validated();

        self.app_config = new_config.clone();
        self.kill_switch.set_config(
            new_config.dns_mode,
            new_config.ipv6_mode,
            new_config.block_doh,
            new_config.custom_doh_blocklist.clone(),
        );

        {
            let mut state = self.shared_state.write().await;
            state.auto_reconnect = new_config.auto_reconnect;
            state.kill_switch = new_config.kill_switch_enabled;
        }

        self.sync_shared_state().await;
        self.update_tray();

        info!("Configuration reloaded successfully");
        Ok(())
    }

    /// Handle IPC command
    pub(crate) async fn handle_ipc_command(
        &mut self,
        cmd: crate::ipc::IpcCommand,
        response_tx: tokio::sync::mpsc::Sender<crate::ipc::IpcResponse>,
    ) {
        use crate::ipc::{IpcCommand, IpcResponse};

        let response = match cmd {
            IpcCommand::Status => {
                let state = self.shared_state.read().await;
                let vpn_type = if let Some(name) = state.state.server_name() {
                    Some(nm_get_vpn_type(name).await.to_string())
                } else {
                    None
                };
                IpcResponse::Status {
                    connected: state.state.server_name().is_some(),
                    vpn_name: state.state.server_name().map(|s| s.to_string()),
                    vpn_type,
                    state: state.state.name().to_string(),
                    kill_switch_enabled: state.kill_switch,
                }
            }
            IpcCommand::List { vpn_type } => {
                let active = nm_get_all_active_vpns().await;
                let connections = nm_list_vpn_connections_with_types().await;
                let mut entries = Vec::new();

                for conn in connections {
                    let type_str = conn.vpn_type.to_string();
                    if let Some(filter) = vpn_type.as_deref() {
                        if filter != type_str {
                            continue;
                        }
                    }

                    let status = if active.iter().any(|a| a.name == conn.name) {
                        "connected"
                    } else {
                        "available"
                    };

                    entries.push(crate::ipc::protocol::VpnConnectionInfo {
                        name: conn.name,
                        vpn_type: type_str,
                        status: status.to_string(),
                    });
                }

                IpcResponse::Connections {
                    connections: entries,
                }
            }
            IpcCommand::Connect { name } => {
                self.handle_connect(&name).await;
                // Since connect is async and we don't wait for completion here (state machine does),
                // we return OK. The client can poll status.
                // Ideally we might want to wait, but the architecture seems fire-and-forget for commands
                IpcResponse::Ok // or maybe return "Connecting to X"
            }
            IpcCommand::Disconnect => {
                self.handle_disconnect().await;
                IpcResponse::Ok
            }
            IpcCommand::Switch { name } => {
                // Logic closer to handle_connect but ensuring switch logic
                self.handle_connect(&name).await;
                IpcResponse::Ok
            }
            IpcCommand::Reconnect => {
                // Check if we have a last server
                let can_reconnect = {
                    let state = self.shared_state.read().await;
                    state.state.server_name().is_some()
                };

                // If connected, we disconnect then connect (handle_connect does this if we pass same server?)
                // Reconnect usually means "reconnect to LAST active server if disconnected" or "restart current".
                // Current logic:
                if can_reconnect {
                    // Get current server
                    let server = self
                        .shared_state
                        .read()
                        .await
                        .state
                        .server_name()
                        .unwrap()
                        .to_string();
                    self.handle_disconnect().await;
                    sleep(Duration::from_secs(2)).await;
                    self.handle_connect(&server).await;
                    IpcResponse::Ok
                } else {
                    // Check history?
                    // app_config has last_server
                    let last_server = self.app_config.last_server.clone();
                    if let Some(server) = last_server {
                        self.handle_connect(&server).await;
                        IpcResponse::Ok
                    } else {
                        IpcResponse::Error {
                            message: "No VPN to reconnect to".to_string(),
                        }
                    }
                }
            }
            IpcCommand::KillSwitch { enable } => {
                let result = if enable {
                    self.kill_switch.enable().await
                } else {
                    self.kill_switch.disable().await
                };

                match result {
                    Ok(()) => {
                        // Update shared state
                        {
                            let mut state = self.shared_state.write().await;
                            state.kill_switch = enable;
                        }
                        self.sync_shared_state().await;
                        IpcResponse::OkMessage {
                            message: format!(
                                "Kill switch {}",
                                if enable { "enabled" } else { "disabled" }
                            ),
                        }
                    }
                    Err(e) => IpcResponse::Error {
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::KillSwitchToggle => {
                self.toggle_kill_switch().await;
                // toggle_kill_switch updates state
                let state = self.shared_state.read().await;
                IpcResponse::OkMessage {
                    message: format!(
                        "Kill switch {}",
                        if state.kill_switch {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    ),
                }
            }
            IpcCommand::KillSwitchStatus => {
                let state = self.shared_state.read().await;
                IpcResponse::KillSwitchStatus {
                    enabled: state.kill_switch,
                }
            }
            IpcCommand::AutoReconnect { enable } => {
                self.app_config.auto_reconnect = enable;
                let _ = self.config_manager.save(&self.app_config.clone());
                self.sync_shared_state().await;
                IpcResponse::Ok
            }
            IpcCommand::AutoReconnectToggle => {
                self.toggle_auto_reconnect().await;
                let state = self.shared_state.read().await;
                IpcResponse::OkMessage {
                    message: format!(
                        "Auto-reconnect {}",
                        if state.auto_reconnect {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    ),
                }
            }
            IpcCommand::AutoReconnectStatus => {
                let state = self.shared_state.read().await;
                IpcResponse::AutoReconnectStatus {
                    enabled: state.auto_reconnect,
                }
            }
            IpcCommand::Debug { enable } => {
                let success = if enable {
                    match crate::logging::enable_debug_logging() {
                        Ok(_) => true,
                        Err(e) => {
                            return {
                                let _ = response_tx.send(IpcResponse::Error { message: e }).await;
                            }
                        }
                    }
                } else {
                    crate::logging::disable_debug_logging();
                    true
                };

                if success {
                    let mut state = self.shared_state.write().await;
                    state.debug_logging = enable;
                    drop(state);
                    self.update_tray();
                }
                self.sync_shared_state().await;
                IpcResponse::Ok
            }
            IpcCommand::Ping => IpcResponse::Ok,
            IpcCommand::Refresh => {
                self.refresh_connections().await;
                IpcResponse::Ok
            }
            IpcCommand::Quit => {
                // handle_quit exits process, so we won't return...
                // But we should try to send response first?
                let _ = response_tx.send(IpcResponse::Ok).await;
                self.handle_quit().await;
                return;
            }
            IpcCommand::Restart => {
                info!("Restart requested via IPC");

                if self.kill_switch.is_enabled() {
                    info!("Disabling kill switch before restart");
                    if let Err(e) = self.kill_switch.disable().await {
                        error!("Failed to disable kill switch: {}", e);
                    }
                }

                let exe_path = match resolve_restart_path() {
                    Ok(path) => path,
                    Err(message) => {
                        let _ = response_tx.send(IpcResponse::Error { message }).await;
                        return;
                    }
                };

                info!("Spawning new daemon instance: {:?}", exe_path);

                let spawn_result = std::process::Command::new(&exe_path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();

                match spawn_result {
                    Ok(_) => {
                        self.should_exit = true;
                        self.exit_reason = Some("restart".to_string());
                        IpcResponse::OkMessage {
                            message: "Restarting daemon...".to_string(),
                        }
                    }
                    Err(e) => IpcResponse::Error {
                        message: format!("Failed to spawn new instance: {}", e),
                    },
                }
            }
            IpcCommand::Reload => {
                info!("Configuration reload requested via IPC");
                match self.reload_configuration().await {
                    Ok(()) => IpcResponse::OkMessage {
                        message: "Configuration reloaded successfully".to_string(),
                    },
                    Err(e) => IpcResponse::Error {
                        message: format!("Failed to reload configuration: {}", e),
                    },
                }
            }
            _ => IpcResponse::Error {
                message: "Command not implemented".to_string(),
            },
        };

        let _ = response_tx.send(response).await;
    }
}

fn resolve_restart_path() -> Result<std::path::PathBuf, String> {
    let exe_path =
        std::env::current_exe().map_err(|e| format!("Failed to get executable path: {}", e))?;

    let exe_display = exe_path.to_string_lossy();
    if exe_path.exists() && !exe_display.contains(" (deleted)") {
        return Ok(exe_path);
    }

    let mut candidates = Vec::new();

    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/shroud"));
        candidates.push(home.join(".cargo/bin/shroud"));
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for entry in path_var.split(':') {
            if entry.is_empty() {
                continue;
            }
            candidates.push(std::path::Path::new(entry).join("shroud"));
        }
    }

    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err("Failed to locate shroud executable to restart".to_string())
}
