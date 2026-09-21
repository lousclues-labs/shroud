// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Health check event handler

use tracing::{debug, error, info, warn};

use crate::health::checker::{HealthConfig, ProbeOutcome};
use crate::health::HealthResult;
use crate::state::{Event, TransitionReason, VpnState};

impl super::super::VpnSupervisor {
    /// Pre-flight for a health check: reconcile local state, then decide
    /// whether a network probe is warranted.
    ///
    /// Returns the config the probe should use, or `None` to skip this cycle.
    /// Everything here is local and fast — the slow network probe is run by the
    /// caller on a detached task so it never holds the event loop. Previously
    /// the probe was awaited inline inside `tokio::select!`, so a single
    /// unreachable endpoint stalled tray, IPC, D-Bus and NM polling for the
    /// full connect timeout.
    pub(crate) async fn begin_health_check(&mut self) -> Option<HealthConfig> {
        // CRITICAL: First sync with NetworkManager state
        // This catches external VPN changes before we do health checks
        if self.sync_state_from_nm().await {
            debug!("State corrected during health check, skipping health check");
            return None;
        }

        // Also sync kill switch state periodically.
        // This is the sole periodic firewall reality-check; the per-poll
        // call previously run from `poll_nm_state` was removed in v2.4.1
        // because it generated tens of thousands of sudo invocations
        // per day with no operational benefit at sub-30s granularity.
        self.sync_killswitch_state().await;

        // Only run health checks when in Connected or Degraded state
        let server = match &self.machine.state {
            VpnState::Connected { server } => server.clone(),
            VpnState::Degraded { server } => server.clone(),
            _ => return None,
        };

        if self.health_checker.is_suspended() {
            debug!("Health check skipped - suspended");
            return None;
        }

        debug!("Running health check for {}", server);
        Some(self.health_checker.config_snapshot())
    }

    /// Apply a completed probe to the state machine.
    ///
    /// Pure local work: counter updates plus at most a `resolv.conf` read.
    pub(crate) async fn finish_health_check(&mut self, outcome: ProbeOutcome) {
        // The probe ran detached, so state may have moved on while it was in
        // flight (user disconnected, VPN dropped, a reconnect started). Re-read
        // it rather than trusting the value captured at launch.
        let server = match &self.machine.state {
            VpnState::Connected { server } => server.clone(),
            VpnState::Degraded { server } => server.clone(),
            _ => {
                debug!("State changed while health probe was in flight, discarding result");
                return;
            }
        };

        let result = self.health_checker.evaluate(outcome);

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
}
