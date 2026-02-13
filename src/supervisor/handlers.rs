//! Supervisor event handlers

use std::time::Instant;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, instrument, warn};

use crate::config::{DnsMode, Ipv6Mode};
use crate::daemon::lock::release_instance_lock;
use crate::dbus::NmEvent;
use crate::health::HealthResult;
use crate::logging;
use crate::nm::{
    get_vpn_type as nm_get_vpn_type,
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
    #[instrument(skip(self), fields(event = ?event))]
    pub(crate) async fn handle_dbus_event(&mut self, event: NmEvent) {
        debug!("Received D-Bus event: {:?}", event);

        // CRITICAL: Ignore ALL D-Bus events while a VPN switch is in progress
        // handle_connect manages everything during a switch - D-Bus events only cause interference
        if self.switch_ctx.in_progress {
            debug!("Ignoring D-Bus event during VPN switch: {:?}", event);
            return;
        }

        // CRITICAL: Ignore late deactivation events from VPN we recently switched FROM
        // D-Bus events can arrive after we've already connected to the new VPN
        if let Some(ref from_server) = self.switch_ctx.from {
            if let NmEvent::VpnDeactivated { ref name } = event {
                if name == from_server {
                    // Check if we're within the grace window after switch completed
                    if let Some(completed) = self.switch_ctx.completed_time {
                        if completed.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                            info!(
                                "Ignoring late deactivation event for switched-from VPN: {}",
                                name
                            );
                            return;
                        }
                    }
                    // Clear the switching_from after processing
                    self.switch_ctx.from = None;
                    self.switch_ctx.completed_time = None;
                }
            }
        }

        // Check if we're in grace period after intentional disconnect
        if let Some(disconnect_time) = self.timing.last_disconnect_time {
            if disconnect_time.elapsed().as_secs() < POST_DISCONNECT_GRACE_SECS {
                debug!("Ignoring D-Bus event during grace period");
                return;
            } else {
                self.timing.last_disconnect_time = None;
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
                        self.tray.update(&self.shared_state);
                        // Then disconnect the old one
                        if let Err(e) = self.nm.disconnect(&old_vpn).await {
                            warn!("Failed to disconnect old VPN '{}': {}", old_vpn, e);
                        }
                        self.tray
                            .notify("VPN Switched", &format!("Now connected to {}", name));
                        return;
                    }
                }

                // Also check for any other active VPNs in NetworkManager
                let all_active = self.nm.get_all_active_vpns().await;
                if all_active.len() > 1 {
                    info!(
                        "Multiple VPNs detected ({}) - cleaning up extras",
                        all_active.len()
                    );
                    for vpn in &all_active {
                        if vpn.name != name {
                            info!("Disconnecting extra VPN: {}", vpn.name);
                            let _ = self.nm.disconnect(&vpn.name).await;
                        }
                    }
                }

                self.dispatch(Event::NmVpnUp { server: name });
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
            }
            NmEvent::VpnActivating { name } => {
                // Only update if we're not already connecting/connected to this VPN
                let dominated = matches!(
                    &self.machine.state,
                    VpnState::Connecting { server } | VpnState::Connected { server }
                        if server == &name
                );
                if !dominated {
                    info!("D-Bus: VPN '{}' activating (external)", name);
                    self.dispatch(Event::UserEnable { server: name });
                    self.sync_shared_state().await;
                    self.tray.update(&self.shared_state);
                } else {
                    debug!(
                        "D-Bus: ignoring activating event for '{}' (already {})",
                        name,
                        self.machine.state.name()
                    );
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
                            self.tray.update(&self.shared_state);
                            self.tray
                                .notify("VPN Disconnected", "Connection dropped, reconnecting...");
                            self.attempt_reconnect(&server).await;
                        } else {
                            // Auto-reconnect disabled: go directly to Disconnected, not Reconnecting
                            self.machine
                                .set_state(VpnState::Disconnected, TransitionReason::VpnLost);
                            self.sync_shared_state().await;
                            self.tray.update(&self.shared_state);
                            self.tray
                                .notify("VPN Disconnected", &format!("Disconnected from {}", name));
                        }
                    }
                }
            }
            NmEvent::VpnFailed { name, reason } => {
                warn!("D-Bus: VPN '{}' failed: {}", name, reason);

                if auto_reconnect {
                    self.dispatch(Event::NmVpnDown);
                    self.sync_shared_state().await;
                    self.tray.update(&self.shared_state);
                    self.tray
                        .notify("VPN Failed", &format!("{}: {}", name, reason));
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
                    self.tray.update(&self.shared_state);
                }
            }
            NmEvent::ConnectivityChanged { connected } => {
                debug!("D-Bus: Connectivity changed: {}", connected);
                // Could trigger health check here
            }
        }
    }

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
        // Sync kill switch state with iptables reality on every poll cycle.
        // This catches desync even when VPN is disconnected (health checks
        // only run when connected).
        self.sync_killswitch_state();

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
                    self.tray.update(&self.shared_state);
                    self.tray
                        .notify("VPN Recovered", "Connection is healthy again");
                } else {
                    debug!("Health check passed");
                }
            }
            HealthResult::Degraded { latency_ms } => {
                if matches!(self.machine.state, VpnState::Connected { .. }) {
                    warn!("Health check degraded: {}ms latency", latency_ms);
                    self.dispatch(Event::HealthDegraded);
                    self.sync_shared_state().await;
                    self.tray.update(&self.shared_state);
                    self.tray
                        .notify("VPN Degraded", &format!("High latency: {}ms", latency_ms));
                }
            }
            HealthResult::Dead { reason } => {
                error!("Health check failed: {}", reason);
                let auto_reconnect = self.shared_state.read().await.auto_reconnect;

                if auto_reconnect {
                    self.dispatch(Event::HealthDead);
                    self.sync_shared_state().await;
                    self.tray.update(&self.shared_state);
                    self.tray
                        .notify("VPN Dead", "Connection lost, reconnecting...");
                    self.attempt_reconnect(&server).await;
                } else {
                    // Auto-reconnect disabled: go directly to Disconnected, not Reconnecting
                    self.machine
                        .set_state(VpnState::Disconnected, TransitionReason::HealthCheckDead);
                    self.sync_shared_state().await;
                    self.tray.update(&self.shared_state);
                    self.tray.notify("VPN Dead", &reason);
                }
            }
            HealthResult::Suspended => {
                // Health checks are suspended (e.g., system wake).
                // Leave state unchanged — don't affirm health or declare failure.
                debug!("Health check suspended, skipping state update");
            }
        }
    }

    /// Handle user request to connect to a server
    #[instrument(skip(self), fields(connection = %connection_name))]
    pub(crate) async fn handle_connect(&mut self, connection_name: &str) {
        info!("Connect requested: {}", connection_name);

        // CRITICAL: Set switching flag to prevent D-Bus events from interfering
        self.switch_ctx.in_progress = true;
        self.switch_ctx.target = Some(connection_name.to_string());

        // Track the VPN we're switching FROM (to ignore late D-Bus events)
        if let Some(current) = self.machine.state.server_name() {
            if current != connection_name {
                self.switch_ctx.from = Some(current.to_string());
            }
        }

        // Set grace period immediately to block any D-Bus deactivation events
        self.timing.last_disconnect_time = Some(Instant::now());

        // NOTE: We do NOT disable kill switch during VPN switch anymore.
        // The kill switch rules already whitelist all VPN server IPs from NetworkManager,
        // so VPN connections should work even with kill switch enabled.
        // NOTE: enable() calls detect_all_vpn_server_ips() which reads server IPs
        // from NM profiles. If the user just imported a config and NM hasn't fully
        // registered the profile, the server IP may not be detected — the connection
        // would be blocked by the kill switch. This is an unlikely edge case
        // (import + connect in rapid succession).
        if self.config_store.config.kill_switch_enabled && !self.kill_switch.is_enabled() {
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
        let all_active = self.nm.get_all_active_vpns().await;
        info!(
            "NM reports {} active VPN(s): {:?}",
            all_active.len(),
            all_active.iter().map(|v| &v.name).collect::<Vec<_>>()
        );

        // Also track any active VPNs as "switching from" to ignore their deactivation events
        for vpn in &all_active {
            if vpn.name != connection_name && self.switch_ctx.from.is_none() {
                self.switch_ctx.from = Some(vpn.name.clone());
            }
        }

        // Disconnect ALL VPNs that aren't the one we're connecting to
        for vpn in &all_active {
            if vpn.name != connection_name {
                info!("Disconnecting VPN before switch: {}", vpn.name);
                if let Err(e) = self.nm.disconnect(&vpn.name).await {
                    warn!("Failed to disconnect {}: {}", vpn.name, e);
                }
            }
        }

        // STEP 2: Wait for ALL disconnects to complete (with verification)
        if all_active.iter().any(|v| v.name != connection_name) {
            info!("Waiting for VPN disconnection(s) to complete...");
            for attempt in 1..=DISCONNECT_VERIFY_MAX_ATTEMPTS {
                sleep(Duration::from_millis(DISCONNECT_VERIFY_INTERVAL_MS)).await;
                let remaining = self.nm.get_all_active_vpns().await;
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
                        let _ = self.nm.disconnect(&other.name).await;
                    }
                }
                debug!(
                    "Still have {} other active VPN(s), attempt {}",
                    others.len(),
                    attempt
                );
            }

            self.nm.kill_orphan_openvpn_processes().await;
            sleep(Duration::from_secs(POST_DISCONNECT_SETTLE_SECS)).await;
        }

        // Final verification before connect
        let final_check = self.nm.get_all_active_vpns().await;
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
        self.tray.update(&self.shared_state);

        self.tray
            .notify("VPN", &format!("Connecting to {}...", connection_name));

        // Attempt connection with retries
        let mut connection_succeeded = false;
        for attempt in 1..=MAX_CONNECT_ATTEMPTS {
            debug!(
                "Connection attempt {} of {} for {}",
                attempt, MAX_CONNECT_ATTEMPTS, connection_name
            );

            match self.nm.connect(connection_name).await {
                Ok(_) => {
                    // Monitor connection state
                    for _ in 1..=CONNECTION_MONITOR_MAX_ATTEMPTS {
                        sleep(Duration::from_millis(CONNECTION_MONITOR_INTERVAL_MS)).await;

                        match self.nm.get_vpn_state(connection_name).await {
                            Some(NmVpnState::Activated) => {
                                info!("VPN '{}' successfully activated", connection_name);
                                self.dispatch(Event::NmVpnUp {
                                    server: connection_name.to_string(),
                                });
                                self.sync_shared_state().await;
                                self.tray.update(&self.shared_state);
                                self.tray.notify(
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
        self.switch_ctx.in_progress = false;
        self.switch_ctx.target = None;
        self.timing.last_disconnect_time = None;
        // Set completion time so late D-Bus events for the old VPN are ignored
        self.switch_ctx.completed_time = Some(Instant::now());

        if !connection_succeeded {
            // All attempts failed - also clear switching_from since there's nothing to ignore
            self.switch_ctx.from = None;
            self.switch_ctx.completed_time = None;
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
            self.tray.update(&self.shared_state);
            self.tray.notify(
                "VPN Failed",
                &format!("Could not connect to {}", connection_name),
            );
        }
    }

    /// Handle user request to disconnect
    #[instrument(skip(self))]
    pub(crate) async fn handle_disconnect(&mut self) {
        info!("Disconnect requested");

        // Cancel any ongoing reconnection attempts
        self.timing.reconnect_cancelled = true;

        let connection_name = match self.machine.state.server_name() {
            Some(name) => name.to_string(),
            None => {
                info!("Not connected, nothing to disconnect");
                return;
            }
        };

        self.timing.last_disconnect_time = Some(Instant::now());

        match self.nm.disconnect(&connection_name).await {
            Ok(_) => {
                info!("Disconnected successfully");

                // CRITICAL: Disable kill switch on intentional disconnect
                // Otherwise user loses all network access
                if self.kill_switch.is_enabled() {
                    info!("Disabling kill switch on user disconnect");
                    if let Err(e) = self.kill_switch.disable().await {
                        warn!("Failed to disable kill switch: {}", e);
                    }
                    // SECURITY: Do NOT persist kill_switch_enabled = false to config.
                    // The kill switch is suspended for this session only — it will
                    // re-enable on next VPN connect if config still says enabled.
                    // This prevents a single IPC Disconnect command from permanently
                    // stripping kill switch protection (SHROUD-VULN-015).
                    {
                        let mut state = self.shared_state.write().await;
                        state.kill_switch = false;
                    }
                }

                self.dispatch(Event::UserDisable);
                self.sync_shared_state().await;
                self.tray.update(&self.shared_state);
                self.tray
                    .notify("VPN Disconnected", "VPN connection closed");
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
        self.tray.notify("VPN Manager", "Restarting...");

        // NOTE: We intentionally do NOT disable the kill switch here.
        // The new daemon instance will detect existing iptables rules via
        // sync_state() in its constructor and adopt them. Tearing down rules
        // creates a window where traffic leaks unprotected — a security hole
        // if the new instance takes time to start or fails to restore them.

        let exe_path = match resolve_restart_path() {
            Ok(path) => path,
            Err(message) => {
                error!("{}", message);
                self.tray.notify("Restart Failed", &message);
                return;
            }
        };

        info!("Spawning new daemon instance: {:?}", exe_path);

        // Spawn detached process that will outlive us
        // SECURITY: Spawn FIRST, then release lock. The child will block on
        // acquiring the lock until we exit. This eliminates the hijack window
        // where both lock and socket are released before the child starts
        // (SHROUD-VULN-031).
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

                // Now release resources so child can acquire them
                release_instance_lock();
                let socket_path = crate::ipc::protocol::socket_path();
                let _ = std::fs::remove_file(&socket_path);

                // Give child time to acquire lock and bind socket
                sleep(Duration::from_millis(500)).await;

                // Now exit
                info!("Old daemon exiting for restart");
                self.exit_state.request("Restart");
            }
            Err(e) => {
                error!("Failed to spawn new instance: {}", e);
                self.tray.notify("Restart Failed", &format!("Error: {}", e));
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
                    self.tray.notify(
                        "Cleanup Failed",
                        "Firewall rules may need manual cleanup. See logs.",
                    );
                }
            }
            self.kill_switch.sync_state();
        }

        // Show notification
        self.tray.notify("Shroud", "Shutting down...");

        // Give notification time to show
        sleep(Duration::from_millis(300)).await;

        info!("Shutdown complete");

        // Clean up and signal exit
        release_instance_lock();
        let socket_path = crate::ipc::protocol::socket_path();
        let _ = std::fs::remove_file(&socket_path);
        self.exit_state.request("User quit");
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
                    self.tray.notify(
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
        self.config_store.config.auto_reconnect = new_value;
        self.config_store.save();

        info!("Auto-reconnect toggled to: {}", new_value);
        self.tray.update(&self.shared_state);
        self.tray.notify(
            "Auto-Reconnect",
            if new_value { "Enabled" } else { "Disabled" },
        );
    }

    /// Toggle kill switch (iptables firewall rules that block non-VPN traffic)
    pub(crate) async fn toggle_kill_switch(&mut self) {
        let current_enabled = self.config_store.config.kill_switch_enabled;
        let new_enabled = !current_enabled;
        info!(
            "Kill switch toggle requested: {} → {}",
            current_enabled, new_enabled
        );

        let result = if new_enabled {
            self.kill_switch.enable().await
        } else {
            self.kill_switch.disable().await
        };

        // Read ACTUAL state after the operation — don't trust Ok(()) alone.
        // enable()/disable() can return Ok(()) without acting (cooldown, toggle guard).
        let actual_enabled = self.kill_switch.is_enabled();

        match result {
            Ok(()) => {
                // Sync shared state to actual kill switch state
                {
                    let mut state = self.shared_state.write().await;
                    state.kill_switch = actual_enabled;
                }
                self.tray.update(&self.shared_state);

                if actual_enabled == new_enabled {
                    // Operation achieved the desired state
                    self.config_store.config.kill_switch_enabled = new_enabled;
                    self.config_store.save();

                    info!("Kill switch toggled to: {}", new_enabled);
                    self.tray.notify(
                        "Kill Switch",
                        if new_enabled {
                            "Enabled - Non-VPN traffic blocked"
                        } else {
                            "Disabled"
                        },
                    );
                } else {
                    // Operation returned Ok but didn't change state (cooldown/guard)
                    warn!(
                        "Kill switch toggle returned Ok but state unchanged (wanted={}, actual={})",
                        new_enabled, actual_enabled
                    );
                }
            }
            Err(e) => {
                // Sync shared state to actual kill switch state on error too
                {
                    let mut state = self.shared_state.write().await;
                    state.kill_switch = actual_enabled;
                }
                self.tray.update(&self.shared_state);

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

                            // SECURITY: Update runtime state but do NOT persist to config.
                            // If the table/chain doesn't exist, the rules are effectively
                            // gone — but config should retain the user's intent to have
                            // the kill switch enabled (SHROUD-VULN-035).
                            {
                                let mut state = self.shared_state.write().await;
                                state.kill_switch = false;
                            }

                            self.tray.update(&self.shared_state);
                            self.tray.notify("Kill Switch", "Disabled");
                            return;
                        }
                    }
                }

                error!("Failed to toggle kill switch: {}", e);
                self.tray
                    .notify("Kill Switch Error", &format!("Failed: {}", e));
            }
        }
    }

    /// Toggle autostart on login
    pub(crate) async fn toggle_autostart(&mut self) {
        match crate::autostart::Autostart::toggle() {
            Ok(enabled) => {
                self.tray.update(&self.shared_state);
                self.tray
                    .notify("Autostart", if enabled { "Enabled" } else { "Disabled" });
            }
            Err(e) => {
                error!("Failed to toggle autostart: {}", e);
                self.tray.notify("Autostart Error", &e);
            }
        }
    }

    /// Toggle debug logging to file
    #[instrument(skip(self))]
    pub(crate) async fn toggle_debug_logging(&mut self) {
        let currently_enabled = logging::is_debug_logging_enabled();

        if currently_enabled {
            logging::disable_debug_logging();
            {
                let mut state = self.shared_state.write().await;
                state.debug_logging = false;
            }
            info!("Debug logging disabled");
            self.tray.update(&self.shared_state);
            self.tray.notify("Debug Logging", "Disabled");
        } else {
            match logging::enable_debug_logging() {
                Ok(path) => {
                    {
                        let mut state = self.shared_state.write().await;
                        state.debug_logging = true;
                    }
                    info!("Debug logging enabled to {:?}", path);
                    self.tray.update(&self.shared_state);
                    self.tray.notify(
                        "Debug Logging",
                        &format!("Enabled. Logs: {}", path.display()),
                    );
                }
                Err(e) => {
                    error!("Failed to enable debug logging: {}", e);
                    self.tray.notify("Debug Logging Error", &e);
                }
            }
        }
    }

    /// Open the log file in the default viewer
    pub(crate) fn open_log_file(&mut self) {
        match logging::open_log_file() {
            Ok(()) => {
                debug!("Opened log file");
            }
            Err(e) => {
                warn!("Failed to open log file: {}", e);
                self.tray.notify("Log File", &e);
            }
        }
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

    async fn reload_configuration(
        &mut self,
        source: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!(
            "Reloading configuration from disk (triggered by: {})",
            source
        );
        let new_config = self.config_store.reload();
        let old_config = &self.config_store.config;

        // SECURITY: Refuse security-critical downgrades via config file reload.
        // Explicit IPC commands (KillSwitch, AutoReconnect, etc.) still work.
        let mut refused = Vec::new();

        let apply_kill_switch = if old_config.kill_switch_enabled && !new_config.kill_switch_enabled
        {
            refused.push("kill_switch_enabled: true → false");
            false
        } else {
            true
        };

        let apply_auto_reconnect = if old_config.auto_reconnect && !new_config.auto_reconnect {
            refused.push("auto_reconnect: true → false");
            false
        } else {
            true
        };

        let apply_dns_mode = {
            let old_secure = matches!(old_config.dns_mode, DnsMode::Tunnel | DnsMode::Strict);
            let new_insecure = matches!(new_config.dns_mode, DnsMode::Localhost | DnsMode::Any);
            if old_secure && new_insecure {
                refused.push("dns_mode: secure → less secure");
                false
            } else {
                true
            }
        };

        let apply_ipv6_mode = {
            let old_block = matches!(old_config.ipv6_mode, Ipv6Mode::Block);
            let new_weaker = matches!(new_config.ipv6_mode, Ipv6Mode::Tunnel | Ipv6Mode::Off);
            if old_block && new_weaker {
                refused.push("ipv6_mode: block → weaker");
                false
            } else {
                true
            }
        };

        let apply_block_doh = if old_config.block_doh && !new_config.block_doh {
            refused.push("block_doh: true → false");
            false
        } else {
            true
        };

        for msg in &refused {
            warn!(
                "Security downgrade refused via config reload: {}. Use explicit IPC command to change security settings.",
                msg
            );
        }

        // Apply firewall config — only the fields not refused
        let dns = if apply_dns_mode {
            new_config.dns_mode
        } else {
            old_config.dns_mode
        };
        let ipv6 = if apply_ipv6_mode {
            new_config.ipv6_mode
        } else {
            old_config.ipv6_mode
        };
        let doh = if apply_block_doh {
            new_config.block_doh
        } else {
            old_config.block_doh
        };

        self.kill_switch
            .set_config(dns, ipv6, doh, new_config.custom_doh_blocklist.clone());

        {
            let mut state = self.shared_state.write().await;
            state.auto_reconnect = if apply_auto_reconnect {
                new_config.auto_reconnect
            } else {
                old_config.auto_reconnect
            };
            state.kill_switch = if apply_kill_switch {
                new_config.kill_switch_enabled
            } else {
                old_config.kill_switch_enabled
            };
        }

        self.sync_shared_state().await;
        self.tray.update(&self.shared_state);

        info!("Configuration reloaded successfully");
        Ok(())
    }

    /// Handle IPC command
    pub(crate) async fn handle_ipc_command(
        &mut self,
        cmd: crate::ipc::IpcCommand,
        response_tx: tokio::sync::mpsc::Sender<crate::ipc::IpcResponse>,
    ) {
        use crate::ipc::{IpcCommand, IpcResponse, PROTOCOL_VERSION};

        let response = match cmd {
            IpcCommand::Hello { .. } => IpcResponse::Error {
                message: "Hello handshake handled by IPC server".to_string(),
            },
            IpcCommand::Version => IpcResponse::VersionInfo {
                binary_version: env!("CARGO_PKG_VERSION").to_string(),
                protocol_version: PROTOCOL_VERSION,
            },
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
                let active = self.nm.get_all_active_vpns().await;
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
                // SECURITY: Use handle_connect() directly — it handles disconnecting
                // the old VPN internally while preserving the kill switch.
                // The old disconnect-sleep-connect pattern disabled the kill switch
                // during the 2-second gap (SHROUD-VULN-046).
                let can_reconnect = {
                    let state = self.shared_state.read().await;
                    state.state.server_name().is_some()
                };

                if can_reconnect {
                    let server = self
                        .shared_state
                        .read()
                        .await
                        .state
                        .server_name()
                        .unwrap()
                        .to_string();
                    self.handle_connect(&server).await;
                    IpcResponse::Ok
                } else {
                    let last_server = self.config_store.config.last_server.clone();
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
                // Skip if already in the desired state
                if self.kill_switch.is_enabled() == enable {
                    debug!(
                        "Kill switch already {}, skipping",
                        if enable { "enabled" } else { "disabled" }
                    );
                    let _ = response_tx
                        .send(IpcResponse::OkMessage {
                            message: format!(
                                "Kill switch already {}",
                                if enable { "enabled" } else { "disabled" }
                            ),
                        })
                        .await;
                    return;
                }

                let result = if enable {
                    self.kill_switch.enable().await
                } else {
                    self.kill_switch.disable().await
                };

                // Read ACTUAL state — don't trust Ok(()) alone
                let actual_enabled = self.kill_switch.is_enabled();

                match result {
                    Ok(()) => {
                        // Sync shared state to actual kill switch state
                        {
                            let mut state = self.shared_state.write().await;
                            state.kill_switch = actual_enabled;
                        }
                        self.config_store.config.kill_switch_enabled = actual_enabled;
                        self.config_store.save();
                        self.sync_shared_state().await;
                        IpcResponse::OkMessage {
                            message: format!(
                                "Kill switch {}",
                                if actual_enabled {
                                    "enabled"
                                } else {
                                    "disabled"
                                }
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
                self.config_store.config.auto_reconnect = enable;
                self.config_store.save();
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
                    self.tray.update(&self.shared_state);
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
                use std::os::unix::process::CommandExt;
                info!("Restart requested via IPC");

                // NOTE: Do NOT disable kill switch before restart.
                // The new instance will adopt existing rules via sync_state().

                let exe_path = match resolve_restart_path() {
                    Ok(path) => path,
                    Err(message) => {
                        let _ = response_tx.send(IpcResponse::Error { message }).await;
                        return;
                    }
                };

                info!("Spawning new daemon instance: {:?}", exe_path);

                // SECURITY: Spawn with setsid, matching the tray restart path.
                // Spawn BEFORE releasing lock (SHROUD-VULN-031).
                let mut cmd = std::process::Command::new(&exe_path);
                cmd.stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());

                unsafe {
                    cmd.pre_exec(|| {
                        libc::setsid();
                        Ok(())
                    });
                }

                match cmd.spawn() {
                    Ok(child) => {
                        info!("Spawned new daemon (PID: {})", child.id());
                        // Release lock and socket so child can acquire them
                        release_instance_lock();
                        let sock = crate::ipc::protocol::socket_path();
                        let _ = std::fs::remove_file(&sock);
                        self.exit_state.request("restart");
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
                match self.reload_configuration("IPC").await {
                    Ok(()) => IpcResponse::OkMessage {
                        message: "Configuration reloaded successfully".to_string(),
                    },
                    Err(e) => IpcResponse::Error {
                        message: format!("Failed to reload configuration: {}", e),
                    },
                }
            }
            IpcCommand::DebugLogPath => {
                let path = crate::logging::default_log_path();
                IpcResponse::DebugInfo {
                    log_path: Some(path.to_string_lossy().to_string()),
                    debug_enabled: crate::logging::is_debug_logging_enabled(),
                }
            }
            IpcCommand::DebugDump => {
                let state = self.shared_state.read().await;
                let machine_state = self.machine.state.name();
                let server = self.machine.state.server_name().map(|s| s.to_string());
                let kill_switch = self.kill_switch.is_enabled();
                let auto_reconnect = state.auto_reconnect;
                let debug_logging = crate::logging::is_debug_logging_enabled();
                let connections = state.connections.clone();
                let switching = self.switch_ctx.in_progress;
                let retries = self.machine.retries();
                drop(state);

                let dump = serde_json::json!({
                    "state": machine_state,
                    "server": server,
                    "kill_switch_enabled": kill_switch,
                    "auto_reconnect": auto_reconnect,
                    "debug_logging": debug_logging,
                    "connections": connections,
                    "switching_in_progress": switching,
                    "reconnect_retries": retries,
                    "reconnect_cancelled": self.timing.reconnect_cancelled,
                    "is_first_run": self.config_store.is_first_run,
                    "config": {
                        "max_reconnect_attempts": self.config_store.config.max_reconnect_attempts,
                        "health_check_interval_secs": self.config_store.config.health_check_interval_secs,
                        "health_degraded_threshold_ms": self.config_store.config.health_degraded_threshold_ms,
                        "health_check_endpoints": self.config_store.config.health_check_endpoints.clone(),
                        "dns_mode": format!("{}", self.config_store.config.dns_mode),
                        "ipv6_mode": format!("{:?}", self.config_store.config.ipv6_mode),
                        "block_doh": self.config_store.config.block_doh,
                    },
                });

                IpcResponse::OkMessage {
                    message: serde_json::to_string_pretty(&dump)
                        .unwrap_or_else(|_| "{}".to_string()),
                }
            }
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

    // Handle the update scenario: binary was replaced at the same path.
    // On Linux, /proc/self/exe shows "/path/to/shroud (deleted)" when the
    // original inode is removed, even if a new binary exists at the same path.
    // This is the normal flow during `scripts/update.sh` (rm + cp).
    if exe_display.contains(" (deleted)") {
        let original_path = exe_display.trim_end_matches(" (deleted)");
        let original = std::path::PathBuf::from(original_path);
        if original.exists() {
            info!(
                "Running binary was replaced (update). Using new binary at: {}",
                original.display()
            );
            return Ok(original);
        }
    }

    // SECURITY: Do NOT fall back to arbitrary user-writable paths.
    // If the running binary is deleted and no replacement exists at the
    // same path, refuse to restart (SHROUD-VULN-036).
    Err("Running binary has been deleted. Cannot safely restart. \
         Please restart manually: shroud"
        .to_string())
}
