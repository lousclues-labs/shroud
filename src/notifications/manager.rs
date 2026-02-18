// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Notification manager with throttling and configuration.
//!
//! Convenience methods (`vpn_connected`, `vpn_disconnected`, etc.) are
//! available for future callers; the supervisor currently uses `show()`
//! via `TrayBridge::notify()`.

#![allow(dead_code)]

use super::types::{Notification, NotificationCategory};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// User-facing notification configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NotificationConfig {
    /// Master enable/disable for all notifications.
    pub enabled: bool,
    /// Show notifications for VPN connection events.
    pub connection_events: bool,
    /// Show notifications for VPN disconnection events.
    pub disconnection_events: bool,
    /// Show notifications for reconnection attempts/success.
    pub reconnection_events: bool,
    /// Show notifications for kill switch changes.
    pub kill_switch_events: bool,
    /// Show notifications for errors and failures.
    pub error_events: bool,
    /// Show notifications for health/degraded state.
    pub health_events: bool,
    /// Show notifications on first run with tips.
    pub first_run_tips: bool,
    /// Minimum time between similar notifications (seconds).
    pub throttle_seconds: u32,
    /// Notification timeout in milliseconds (0 = system default).
    pub timeout_ms: u32,
    /// Play sound for critical notifications.
    pub sound_critical: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            connection_events: true,
            disconnection_events: true,
            reconnection_events: true,
            kill_switch_events: true,
            error_events: true,
            health_events: true,
            first_run_tips: true,
            throttle_seconds: 5,
            timeout_ms: 5000,
            sound_critical: false,
        }
    }
}

/// Manages notification display with throttling and per-category configuration.
pub struct NotificationManager {
    config: NotificationConfig,
    last_notification: HashMap<NotificationCategory, Instant>,
    suppressed_count: u32,
}

impl NotificationManager {
    pub fn new(config: NotificationConfig) -> Self {
        Self {
            config,
            last_notification: HashMap::new(),
            suppressed_count: 0,
        }
    }

    /// Update configuration at runtime.
    pub fn update_config(&mut self, config: NotificationConfig) {
        self.config = config;
    }

    /// Check if notifications are enabled for a category.
    pub fn is_enabled(&self, category: NotificationCategory) -> bool {
        if !self.config.enabled {
            return false;
        }

        match category.config_key() {
            "connection_events" => self.config.connection_events,
            "disconnection_events" => self.config.disconnection_events,
            "reconnection_events" => self.config.reconnection_events,
            "kill_switch_events" => self.config.kill_switch_events,
            "error_events" => self.config.error_events,
            "health_events" => self.config.health_events,
            "first_run_tips" => self.config.first_run_tips,
            _ => true,
        }
    }

    /// Check if a notification should be throttled.
    pub fn should_throttle(&self, category: NotificationCategory) -> bool {
        let throttle_duration = Duration::from_secs(self.config.throttle_seconds as u64);
        if throttle_duration.is_zero() {
            return false;
        }
        if let Some(last_time) = self.last_notification.get(&category) {
            if last_time.elapsed() < throttle_duration {
                return true;
            }
        }
        false
    }

    /// Record that a notification was shown.
    pub fn record_shown(&mut self, category: NotificationCategory) {
        self.last_notification.insert(category, Instant::now());
    }

    /// Record a suppressed notification.
    pub fn record_suppressed(&mut self) {
        self.suppressed_count += 1;
    }

    /// Get the number of suppressed notifications.
    pub fn suppressed_count(&self) -> u32 {
        self.suppressed_count
    }

    /// Get the configured timeout in milliseconds.
    pub fn timeout_ms(&self) -> u32 {
        self.config.timeout_ms
    }

    /// Whether critical-sound is enabled.
    pub fn sound_critical(&self) -> bool {
        self.config.sound_critical
    }

    /// Decide whether to display a notification, applying filters and throttle.
    ///
    /// Returns `true` if the notification should be displayed.
    pub fn should_display(&mut self, notification: &Notification) -> bool {
        if !self.is_enabled(notification.category) {
            debug!(
                "Notification suppressed (disabled): {:?}",
                notification.category
            );
            return false;
        }

        if self.should_throttle(notification.category) {
            debug!("Notification throttled: {:?}", notification.category);
            self.record_suppressed();
            return false;
        }

        self.record_shown(notification.category);
        true
    }

    /// Show a notification using notify-rust (does I/O — not tested in unit tests).
    pub fn show(&mut self, notification: Notification) {
        if !self.should_display(&notification) {
            return;
        }

        let title = notification.title;
        let body = notification.body;
        let icon = notification.category.icon().to_string();
        let timeout_ms = notification
            .timeout
            .map(|t| t.as_millis() as i32)
            .unwrap_or(self.config.timeout_ms as i32);

        std::thread::spawn(move || {
            let result = notify_rust::Notification::new()
                .summary(&title)
                .body(&body)
                .icon(&icon)
                .timeout(timeout_ms)
                .appname("Shroud VPN")
                .show();

            if let Err(e) = result {
                warn!("Failed to show notification: {}", e);
            }
        });
    }

    // ---- Convenience methods ----

    /// Show VPN connected notification.
    pub fn vpn_connected(&mut self, vpn_name: &str) {
        self.show(Notification::new(
            NotificationCategory::Connected,
            "VPN Connected",
            format!("Connected to {}", vpn_name),
        ));
    }

    /// Show VPN disconnected notification.
    pub fn vpn_disconnected(&mut self, vpn_name: &str) {
        self.show(Notification::new(
            NotificationCategory::Disconnected,
            "VPN Disconnected",
            format!("Disconnected from {}", vpn_name),
        ));
    }

    /// Show VPN connection lost notification.
    pub fn vpn_connection_lost(&mut self, vpn_name: &str, reconnecting: bool) {
        let body = if reconnecting {
            format!("Connection to {} lost. Reconnecting…", vpn_name)
        } else {
            format!("Connection to {} lost", vpn_name)
        };
        self.show(Notification::new(
            NotificationCategory::ConnectionLost,
            "VPN Connection Lost",
            body,
        ));
    }

    /// Show VPN reconnected notification.
    pub fn vpn_reconnected(&mut self, vpn_name: &str, attempts: u32) {
        let body = if attempts > 1 {
            format!("Reconnected to {} after {} attempts", vpn_name, attempts)
        } else {
            format!("Reconnected to {}", vpn_name)
        };
        self.show(Notification::new(
            NotificationCategory::Reconnected,
            "VPN Reconnected",
            body,
        ));
    }

    /// Show reconnection failed notification.
    pub fn reconnection_failed(&mut self, vpn_name: &str, attempts: u32) {
        self.show(Notification::new(
            NotificationCategory::ReconnectionFailed,
            "Reconnection Failed",
            format!(
                "Failed to reconnect to {} after {} attempts",
                vpn_name, attempts
            ),
        ));
    }

    /// Show connection failed notification.
    pub fn connection_failed(&mut self, vpn_name: &str, reason: &str) {
        self.show(Notification::new(
            NotificationCategory::ConnectionFailed,
            "Connection Failed",
            format!("Failed to connect to {}: {}", vpn_name, reason),
        ));
    }

    /// Show kill switch change notification.
    pub fn kill_switch_changed(&mut self, enabled: bool) {
        let (cat, title, body) = if enabled {
            (
                NotificationCategory::KillSwitchEnabled,
                "Kill Switch Enabled",
                "Non-VPN traffic is now blocked",
            )
        } else {
            (
                NotificationCategory::KillSwitchDisabled,
                "Kill Switch Disabled",
                "All traffic is now allowed",
            )
        };
        self.show(Notification::new(cat, title, body));
    }

    /// Show health status notification.
    pub fn health_changed(&mut self, degraded: bool, vpn_name: &str) {
        if degraded {
            self.show(Notification::new(
                NotificationCategory::HealthDegraded,
                "VPN Health Warning",
                format!("Connection to {} is degraded", vpn_name),
            ));
        } else {
            self.show(Notification::new(
                NotificationCategory::HealthRestored,
                "VPN Health Restored",
                format!("Connection to {} is stable", vpn_name),
            ));
        }
    }

    /// Show error notification.
    pub fn error(&mut self, title: &str, message: &str) {
        self.show(Notification::new(
            NotificationCategory::Error,
            title,
            message,
        ));
    }

    /// Show first-run tip.
    pub fn first_run_tip(&mut self, message: &str) {
        self.show(Notification::new(
            NotificationCategory::FirstRun,
            "Shroud VPN",
            message,
        ));
    }
}

impl Default for NotificationManager {
    fn default() -> Self {
        Self::new(NotificationConfig::default())
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn no_throttle_config() -> NotificationConfig {
        NotificationConfig {
            throttle_seconds: 0,
            ..Default::default()
        }
    }

    // --- is_enabled ---

    mod is_enabled_tests {
        use super::*;

        #[test]
        fn test_enabled_by_default() {
            let mgr = NotificationManager::new(NotificationConfig::default());
            assert!(mgr.is_enabled(NotificationCategory::Connected));
            assert!(mgr.is_enabled(NotificationCategory::Error));
            assert!(mgr.is_enabled(NotificationCategory::FirstRun));
        }

        #[test]
        fn test_master_disable() {
            let cfg = NotificationConfig {
                enabled: false,
                ..Default::default()
            };
            let mgr = NotificationManager::new(cfg);
            assert!(!mgr.is_enabled(NotificationCategory::Connected));
            assert!(!mgr.is_enabled(NotificationCategory::Error));
        }

        #[test]
        fn test_per_category_disable() {
            let cfg = NotificationConfig {
                connection_events: false,
                ..Default::default()
            };
            let mgr = NotificationManager::new(cfg);
            assert!(!mgr.is_enabled(NotificationCategory::Connected));
            assert!(mgr.is_enabled(NotificationCategory::Error));
        }

        #[test]
        fn test_disconnection_disable() {
            let cfg = NotificationConfig {
                disconnection_events: false,
                ..Default::default()
            };
            let mgr = NotificationManager::new(cfg);
            assert!(!mgr.is_enabled(NotificationCategory::Disconnected));
            assert!(mgr.is_enabled(NotificationCategory::Connected));
        }

        #[test]
        fn test_reconnection_disable() {
            let cfg = NotificationConfig {
                reconnection_events: false,
                ..Default::default()
            };
            let mgr = NotificationManager::new(cfg);
            assert!(!mgr.is_enabled(NotificationCategory::ConnectionLost));
            assert!(!mgr.is_enabled(NotificationCategory::Reconnecting));
            assert!(!mgr.is_enabled(NotificationCategory::Reconnected));
            assert!(!mgr.is_enabled(NotificationCategory::ReconnectionFailed));
        }

        #[test]
        fn test_killswitch_disable() {
            let cfg = NotificationConfig {
                kill_switch_events: false,
                ..Default::default()
            };
            let mgr = NotificationManager::new(cfg);
            assert!(!mgr.is_enabled(NotificationCategory::KillSwitchEnabled));
            assert!(!mgr.is_enabled(NotificationCategory::KillSwitchDisabled));
        }

        #[test]
        fn test_health_disable() {
            let cfg = NotificationConfig {
                health_events: false,
                ..Default::default()
            };
            let mgr = NotificationManager::new(cfg);
            assert!(!mgr.is_enabled(NotificationCategory::HealthDegraded));
            assert!(!mgr.is_enabled(NotificationCategory::HealthRestored));
        }

        #[test]
        fn test_error_disable() {
            let cfg = NotificationConfig {
                error_events: false,
                ..Default::default()
            };
            let mgr = NotificationManager::new(cfg);
            assert!(!mgr.is_enabled(NotificationCategory::Error));
            assert!(!mgr.is_enabled(NotificationCategory::ConnectionFailed));
        }

        #[test]
        fn test_first_run_disable() {
            let cfg = NotificationConfig {
                first_run_tips: false,
                ..Default::default()
            };
            let mgr = NotificationManager::new(cfg);
            assert!(!mgr.is_enabled(NotificationCategory::FirstRun));
        }
    }

    // --- throttling ---

    mod throttle_tests {
        use super::*;

        #[test]
        fn test_no_throttle_when_disabled() {
            let mgr = NotificationManager::new(no_throttle_config());
            assert!(!mgr.should_throttle(NotificationCategory::Connected));
        }

        #[test]
        fn test_first_event_not_throttled() {
            let cfg = NotificationConfig {
                throttle_seconds: 60,
                ..Default::default()
            };
            let mgr = NotificationManager::new(cfg);
            assert!(!mgr.should_throttle(NotificationCategory::Connected));
        }

        #[test]
        fn test_same_category_throttled() {
            let cfg = NotificationConfig {
                throttle_seconds: 60,
                ..Default::default()
            };
            let mut mgr = NotificationManager::new(cfg);
            mgr.record_shown(NotificationCategory::Connected);
            assert!(mgr.should_throttle(NotificationCategory::Connected));
        }

        #[test]
        fn test_different_category_not_throttled() {
            let cfg = NotificationConfig {
                throttle_seconds: 60,
                ..Default::default()
            };
            let mut mgr = NotificationManager::new(cfg);
            mgr.record_shown(NotificationCategory::Connected);
            assert!(!mgr.should_throttle(NotificationCategory::Disconnected));
        }

        #[test]
        fn test_after_window_not_throttled() {
            let cfg = NotificationConfig {
                throttle_seconds: 1,
                ..Default::default()
            };
            let mut mgr = NotificationManager::new(cfg);
            mgr.last_notification.insert(
                NotificationCategory::Connected,
                Instant::now() - Duration::from_secs(2),
            );
            assert!(!mgr.should_throttle(NotificationCategory::Connected));
        }
    }

    // --- should_display ---

    mod should_display_tests {
        use super::*;

        #[test]
        fn test_enabled_and_not_throttled() {
            let mut mgr = NotificationManager::new(no_throttle_config());
            let n = Notification::new(NotificationCategory::Connected, "T", "B");
            assert!(mgr.should_display(&n));
        }

        #[test]
        fn test_disabled_category() {
            let cfg = NotificationConfig {
                connection_events: false,
                ..Default::default()
            };
            let mut mgr = NotificationManager::new(cfg);
            let n = Notification::new(NotificationCategory::Connected, "T", "B");
            assert!(!mgr.should_display(&n));
        }

        #[test]
        fn test_throttled() {
            let cfg = NotificationConfig {
                throttle_seconds: 60,
                ..Default::default()
            };
            let mut mgr = NotificationManager::new(cfg);
            let n = Notification::new(NotificationCategory::Connected, "T", "B");
            assert!(mgr.should_display(&n));
            // Second time → throttled
            assert!(!mgr.should_display(&n));
        }

        #[test]
        fn test_throttled_increments_suppressed() {
            let cfg = NotificationConfig {
                throttle_seconds: 60,
                ..Default::default()
            };
            let mut mgr = NotificationManager::new(cfg);
            let n = Notification::new(NotificationCategory::Connected, "T", "B");
            mgr.should_display(&n);
            mgr.should_display(&n);
            mgr.should_display(&n);
            assert_eq!(mgr.suppressed_count(), 2);
        }
    }

    // --- update_config ---

    mod config_update_tests {
        use super::*;

        #[test]
        fn test_update_config() {
            let mut mgr = NotificationManager::new(NotificationConfig::default());
            assert!(mgr.is_enabled(NotificationCategory::Connected));

            mgr.update_config(NotificationConfig {
                enabled: false,
                ..Default::default()
            });
            assert!(!mgr.is_enabled(NotificationCategory::Connected));
        }

        #[test]
        fn test_update_throttle() {
            let mut mgr = NotificationManager::new(NotificationConfig::default());
            mgr.update_config(NotificationConfig {
                throttle_seconds: 0,
                ..Default::default()
            });
            mgr.record_shown(NotificationCategory::Connected);
            assert!(!mgr.should_throttle(NotificationCategory::Connected));
        }
    }

    // --- accessors ---

    mod accessor_tests {
        use super::*;

        #[test]
        fn test_timeout_ms() {
            let mgr = NotificationManager::new(NotificationConfig {
                timeout_ms: 3000,
                ..Default::default()
            });
            assert_eq!(mgr.timeout_ms(), 3000);
        }

        #[test]
        fn test_sound_critical() {
            let mgr = NotificationManager::new(NotificationConfig {
                sound_critical: true,
                ..Default::default()
            });
            assert!(mgr.sound_critical());
        }

        #[test]
        fn test_sound_critical_default_false() {
            let mgr = NotificationManager::default();
            assert!(!mgr.sound_critical());
        }

        #[test]
        fn test_suppressed_count_initial() {
            let mgr = NotificationManager::default();
            assert_eq!(mgr.suppressed_count(), 0);
        }
    }

    // --- NotificationConfig defaults ---

    mod config_tests {
        use super::*;

        #[test]
        fn test_defaults() {
            let cfg = NotificationConfig::default();
            assert!(cfg.enabled);
            assert!(cfg.connection_events);
            assert!(cfg.disconnection_events);
            assert!(cfg.reconnection_events);
            assert!(cfg.kill_switch_events);
            assert!(cfg.error_events);
            assert!(cfg.health_events);
            assert!(cfg.first_run_tips);
            assert_eq!(cfg.throttle_seconds, 5);
            assert_eq!(cfg.timeout_ms, 5000);
            assert!(!cfg.sound_critical);
        }
    }
}
