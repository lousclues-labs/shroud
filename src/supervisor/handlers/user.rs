// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! User-initiated command handlers

use std::time::Instant;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, instrument, warn};

use crate::config::{DnsMode, Ipv6Mode};
use crate::daemon::lock::release_instance_lock;
use crate::logging;
use crate::state::{Event, NmVpnState};

use super::super::{
    CONNECTION_MONITOR_INTERVAL_MS, CONNECTION_MONITOR_MAX_ATTEMPTS, CONNECT_RETRY_DELAY_SECS,
    DISCONNECT_VERIFY_INTERVAL_MS, DISCONNECT_VERIFY_MAX_ATTEMPTS, MAX_CONNECT_ATTEMPTS,
    POST_DISCONNECT_SETTLE_SECS,
};
use super::system::resolve_restart_path;

impl super::super::VpnSupervisor {
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
        // Kill-switch ordering (see KillSwitchConfig::pre_connect):
        // - pre_connect (default): fail-closed — enable BEFORE connecting so no
        //   traffic leaks during the handshake.
        // - connect-first: usability-first — suspend the kill switch during the
        //   handshake so DNS can resolve a hostname endpoint; it is re-enabled
        //   after a successful connect by the post-connect block below.
        if self.config_store.config.kill_switch_enabled {
            if self.config_store.config.killswitch.pre_connect {
                if !self.kill_switch.is_enabled() {
                    info!("Pre-enabling kill switch before connection");
                    if let Err(e) = self.kill_switch.enable().await {
                        warn!("Failed to pre-enable kill switch: {}", e);
                    } else {
                        let mut state = self.shared_state.write().await;
                        state.kill_switch = true;
                    }
                }
            } else if self.kill_switch.is_enabled() {
                info!("connect-first: suspending kill switch during connect");
                if let Err(e) = self.kill_switch.disable().await {
                    warn!("Failed to suspend kill switch for connect-first: {}", e);
                } else {
                    let mut state = self.shared_state.write().await;
                    state.kill_switch = false;
                }
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
                sleep(Duration::from_secs(CONNECT_RETRY_DELAY_SECS)).await;
            }
        }

        // With pre_connect ordering the kill switch stays enabled throughout.
        // With connect-first ordering we (re-)enable it now that the tunnel is
        // up and the live server IP is whitelistable.
        if connection_succeeded
            && self.config_store.config.kill_switch_enabled
            && !self.kill_switch.is_enabled()
        {
            info!("connect-first: enabling kill switch after successful connect");
            if let Err(e) = self.kill_switch.enable().await {
                warn!("Failed to enable kill switch after connect: {}", e);
            } else {
                let mut state = self.shared_state.write().await;
                state.kill_switch = true;
            }
        } else if !connection_succeeded
            && self.config_store.config.kill_switch_enabled
            && !self.config_store.config.killswitch.pre_connect
        {
            warn!(
                "connect-first: connection failed; kill switch remains suspended \
                 (usability-first fail-open). Traffic is UNPROTECTED until reconnect."
            );
        }

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
    ///
    /// Also couples `auto_connect` with autostart: "start on login" means
    /// "start AND connect on login".
    pub(crate) async fn toggle_autostart(&mut self) {
        match crate::autostart::Autostart::toggle() {
            Ok(enabled) => {
                info!("Autostart toggled to: {}", enabled);

                // Couple auto_connect with autostart
                self.config_store.config.auto_connect = enabled;
                self.config_store.save();

                self.tray.update(&self.shared_state);
                self.tray.notify(
                    "Autostart",
                    if enabled {
                        "VPN Shroud will start and auto-connect on login"
                    } else {
                        "Autostart and auto-connect disabled"
                    },
                );
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

    pub(super) async fn reload_configuration(
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
}
