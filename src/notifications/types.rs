// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! Notification types and categories — pure data, easily testable.
//!
//! Some variants and methods are part of the public API surface but not
//! yet called by the supervisor (which routes through `TrayBridge::notify`).

#![allow(dead_code)]

use std::time::Duration;

/// Notification urgency level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    /// Low priority — informational.
    Low,
    /// Normal priority — status changes.
    Normal,
    /// Critical — requires attention (errors, unexpected disconnects).
    Critical,
}

/// Notification category for filtering and configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationCategory {
    /// VPN connected successfully.
    Connected,
    /// VPN disconnected (user-initiated).
    Disconnected,
    /// VPN connection dropped unexpectedly.
    ConnectionLost,
    /// Reconnection attempt started.
    Reconnecting,
    /// Reconnection successful.
    Reconnected,
    /// Reconnection failed after max attempts.
    ReconnectionFailed,
    /// Kill switch enabled.
    KillSwitchEnabled,
    /// Kill switch disabled.
    KillSwitchDisabled,
    /// Connection health degraded.
    HealthDegraded,
    /// Connection health restored.
    HealthRestored,
    /// VPN connection failed.
    ConnectionFailed,
    /// Generic error.
    Error,
    /// First-run tips.
    FirstRun,
}

impl NotificationCategory {
    /// Icon name for this category.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Connected | Self::Reconnected => "network-vpn-symbolic",
            Self::Reconnecting => "network-vpn-acquiring-symbolic",
            Self::Disconnected => "network-vpn-disconnected-symbolic",
            Self::ConnectionLost | Self::ReconnectionFailed | Self::ConnectionFailed => {
                "network-vpn-error-symbolic"
            }
            Self::KillSwitchEnabled => "security-high-symbolic",
            Self::KillSwitchDisabled => "security-low-symbolic",
            Self::HealthDegraded => "dialog-warning-symbolic",
            Self::HealthRestored => "emblem-ok-symbolic",
            Self::Error => "dialog-error-symbolic",
            Self::FirstRun => "dialog-information-symbolic",
        }
    }

    /// Default urgency for this category.
    pub fn urgency(&self) -> Urgency {
        match self {
            Self::ConnectionLost
            | Self::ReconnectionFailed
            | Self::ConnectionFailed
            | Self::Error => Urgency::Critical,

            Self::Connected
            | Self::Disconnected
            | Self::Reconnected
            | Self::KillSwitchEnabled
            | Self::KillSwitchDisabled
            | Self::HealthDegraded
            | Self::HealthRestored => Urgency::Normal,

            Self::Reconnecting | Self::FirstRun => Urgency::Low,
        }
    }

    /// Default display timeout.
    pub fn default_timeout(&self) -> Duration {
        match self.urgency() {
            Urgency::Critical => Duration::from_secs(10),
            Urgency::Normal => Duration::from_secs(5),
            Urgency::Low => Duration::from_secs(3),
        }
    }

    /// Whether this category should play a sound.
    pub fn should_play_sound(&self) -> bool {
        matches!(self.urgency(), Urgency::Critical)
    }

    /// Whether this category supports action buttons.
    pub fn supports_actions(&self) -> bool {
        matches!(
            self,
            Self::ConnectionLost | Self::ReconnectionFailed | Self::Disconnected
        )
    }

    /// Configuration key controlling this category.
    pub fn config_key(&self) -> &'static str {
        match self {
            Self::Connected => "connection_events",
            Self::Disconnected => "disconnection_events",
            Self::ConnectionLost
            | Self::Reconnecting
            | Self::Reconnected
            | Self::ReconnectionFailed => "reconnection_events",
            Self::KillSwitchEnabled | Self::KillSwitchDisabled => "kill_switch_events",
            Self::HealthDegraded | Self::HealthRestored => "health_events",
            Self::ConnectionFailed | Self::Error => "error_events",
            Self::FirstRun => "first_run_tips",
        }
    }
}

/// A notification to be displayed.
#[derive(Debug, Clone)]
pub struct Notification {
    pub category: NotificationCategory,
    pub title: String,
    pub body: String,
    pub urgency: Urgency,
    pub timeout: Option<Duration>,
    pub actions: Vec<NotificationAction>,
}

impl Notification {
    pub fn new(
        category: NotificationCategory,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            urgency: category.urgency(),
            timeout: Some(category.default_timeout()),
            category,
            title: title.into(),
            body: body.into(),
            actions: Vec::new(),
        }
    }

    pub fn with_urgency(mut self, urgency: Urgency) -> Self {
        self.urgency = urgency;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_action(mut self, action: NotificationAction) -> Self {
        self.actions.push(action);
        self
    }
}

/// An action button on a notification.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

impl NotificationAction {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }

    pub fn reconnect() -> Self {
        Self::new("reconnect", "Reconnect")
    }

    pub fn dismiss() -> Self {
        Self::new("dismiss", "Dismiss")
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod category_tests {
        use super::*;

        #[test]
        fn test_icons_contain_expected_keywords() {
            assert!(NotificationCategory::Connected.icon().contains("vpn"));
            assert!(NotificationCategory::Disconnected.icon().contains("vpn"));
            assert!(NotificationCategory::Error.icon().contains("error"));
            assert!(NotificationCategory::KillSwitchEnabled
                .icon()
                .contains("security"));
            assert!(NotificationCategory::KillSwitchDisabled
                .icon()
                .contains("security"));
            assert!(NotificationCategory::HealthDegraded
                .icon()
                .contains("warning"));
            assert!(NotificationCategory::HealthRestored.icon().contains("ok"));
            assert!(NotificationCategory::FirstRun
                .icon()
                .contains("information"));
        }

        #[test]
        fn test_icons_are_symbolic() {
            let categories = [
                NotificationCategory::Connected,
                NotificationCategory::Disconnected,
                NotificationCategory::ConnectionLost,
                NotificationCategory::Reconnecting,
                NotificationCategory::Error,
                NotificationCategory::KillSwitchEnabled,
                NotificationCategory::HealthDegraded,
                NotificationCategory::FirstRun,
            ];
            for cat in &categories {
                assert!(
                    cat.icon().ends_with("-symbolic"),
                    "{:?} icon doesn't end with -symbolic: {}",
                    cat,
                    cat.icon()
                );
            }
        }

        #[test]
        fn test_urgency_critical() {
            assert_eq!(
                NotificationCategory::ConnectionLost.urgency(),
                Urgency::Critical
            );
            assert_eq!(
                NotificationCategory::ReconnectionFailed.urgency(),
                Urgency::Critical
            );
            assert_eq!(
                NotificationCategory::ConnectionFailed.urgency(),
                Urgency::Critical
            );
            assert_eq!(NotificationCategory::Error.urgency(), Urgency::Critical);
        }

        #[test]
        fn test_urgency_normal() {
            assert_eq!(NotificationCategory::Connected.urgency(), Urgency::Normal);
            assert_eq!(
                NotificationCategory::Disconnected.urgency(),
                Urgency::Normal
            );
            assert_eq!(NotificationCategory::Reconnected.urgency(), Urgency::Normal);
            assert_eq!(
                NotificationCategory::KillSwitchEnabled.urgency(),
                Urgency::Normal
            );
            assert_eq!(
                NotificationCategory::HealthDegraded.urgency(),
                Urgency::Normal
            );
        }

        #[test]
        fn test_urgency_low() {
            assert_eq!(NotificationCategory::Reconnecting.urgency(), Urgency::Low);
            assert_eq!(NotificationCategory::FirstRun.urgency(), Urgency::Low);
        }

        #[test]
        fn test_critical_timeout_longer() {
            let critical = NotificationCategory::ConnectionLost.default_timeout();
            let normal = NotificationCategory::Connected.default_timeout();
            let low = NotificationCategory::FirstRun.default_timeout();
            assert!(critical > normal);
            assert!(normal > low);
        }

        #[test]
        fn test_should_play_sound() {
            assert!(NotificationCategory::ConnectionLost.should_play_sound());
            assert!(NotificationCategory::Error.should_play_sound());
            assert!(!NotificationCategory::Connected.should_play_sound());
            assert!(!NotificationCategory::FirstRun.should_play_sound());
        }

        #[test]
        fn test_supports_actions() {
            assert!(NotificationCategory::ConnectionLost.supports_actions());
            assert!(NotificationCategory::ReconnectionFailed.supports_actions());
            assert!(NotificationCategory::Disconnected.supports_actions());
            assert!(!NotificationCategory::Connected.supports_actions());
            assert!(!NotificationCategory::Error.supports_actions());
        }

        #[test]
        fn test_config_keys() {
            assert_eq!(
                NotificationCategory::Connected.config_key(),
                "connection_events"
            );
            assert_eq!(
                NotificationCategory::Disconnected.config_key(),
                "disconnection_events"
            );
            assert_eq!(
                NotificationCategory::ConnectionLost.config_key(),
                "reconnection_events"
            );
            assert_eq!(
                NotificationCategory::Reconnecting.config_key(),
                "reconnection_events"
            );
            assert_eq!(
                NotificationCategory::KillSwitchEnabled.config_key(),
                "kill_switch_events"
            );
            assert_eq!(
                NotificationCategory::HealthDegraded.config_key(),
                "health_events"
            );
            assert_eq!(NotificationCategory::Error.config_key(), "error_events");
            assert_eq!(
                NotificationCategory::FirstRun.config_key(),
                "first_run_tips"
            );
        }

        #[test]
        fn test_all_categories_have_config_key() {
            let categories = [
                NotificationCategory::Connected,
                NotificationCategory::Disconnected,
                NotificationCategory::ConnectionLost,
                NotificationCategory::Reconnecting,
                NotificationCategory::Reconnected,
                NotificationCategory::ReconnectionFailed,
                NotificationCategory::KillSwitchEnabled,
                NotificationCategory::KillSwitchDisabled,
                NotificationCategory::HealthDegraded,
                NotificationCategory::HealthRestored,
                NotificationCategory::ConnectionFailed,
                NotificationCategory::Error,
                NotificationCategory::FirstRun,
            ];
            for cat in &categories {
                assert!(
                    !cat.config_key().is_empty(),
                    "{:?} has empty config key",
                    cat
                );
            }
            assert_eq!(categories.len(), 13);
        }
    }

    mod notification_tests {
        use super::*;

        #[test]
        fn test_new_inherits_category_defaults() {
            let n = Notification::new(NotificationCategory::Error, "Title", "Body");
            assert_eq!(n.urgency, Urgency::Critical);
            assert_eq!(n.timeout, Some(Duration::from_secs(10)));
            assert!(n.actions.is_empty());
        }

        #[test]
        fn test_with_urgency_override() {
            let n = Notification::new(NotificationCategory::Connected, "T", "B")
                .with_urgency(Urgency::Critical);
            assert_eq!(n.urgency, Urgency::Critical);
        }

        #[test]
        fn test_with_timeout_override() {
            let n = Notification::new(NotificationCategory::Connected, "T", "B")
                .with_timeout(Duration::from_secs(30));
            assert_eq!(n.timeout, Some(Duration::from_secs(30)));
        }

        #[test]
        fn test_with_action() {
            let n = Notification::new(NotificationCategory::ConnectionLost, "T", "B")
                .with_action(NotificationAction::reconnect())
                .with_action(NotificationAction::dismiss());
            assert_eq!(n.actions.len(), 2);
            assert_eq!(n.actions[0].id, "reconnect");
            assert_eq!(n.actions[1].id, "dismiss");
        }

        #[test]
        fn test_title_and_body() {
            let n = Notification::new(NotificationCategory::Connected, "VPN Connected", "To vpn1");
            assert_eq!(n.title, "VPN Connected");
            assert_eq!(n.body, "To vpn1");
        }

        #[test]
        fn test_category_preserved() {
            let n = Notification::new(NotificationCategory::FirstRun, "T", "B");
            assert_eq!(n.category, NotificationCategory::FirstRun);
        }
    }

    mod action_tests {
        use super::*;

        #[test]
        fn test_new_action() {
            let a = NotificationAction::new("test-id", "Test Label");
            assert_eq!(a.id, "test-id");
            assert_eq!(a.label, "Test Label");
        }

        #[test]
        fn test_reconnect_action() {
            let a = NotificationAction::reconnect();
            assert_eq!(a.id, "reconnect");
            assert_eq!(a.label, "Reconnect");
        }

        #[test]
        fn test_dismiss_action() {
            let a = NotificationAction::dismiss();
            assert_eq!(a.id, "dismiss");
            assert_eq!(a.label, "Dismiss");
        }

        #[test]
        fn test_action_equality() {
            let a1 = NotificationAction::reconnect();
            let a2 = NotificationAction::reconnect();
            assert_eq!(a1, a2);
        }

        #[test]
        fn test_action_inequality() {
            let a1 = NotificationAction::reconnect();
            let a2 = NotificationAction::dismiss();
            assert_ne!(a1, a2);
        }
    }

    mod urgency_tests {
        use super::*;

        #[test]
        fn test_urgency_equality() {
            assert_eq!(Urgency::Low, Urgency::Low);
            assert_eq!(Urgency::Normal, Urgency::Normal);
            assert_eq!(Urgency::Critical, Urgency::Critical);
        }

        #[test]
        fn test_urgency_inequality() {
            assert_ne!(Urgency::Low, Urgency::Normal);
            assert_ne!(Urgency::Normal, Urgency::Critical);
        }

        #[test]
        fn test_urgency_debug() {
            assert_eq!(format!("{:?}", Urgency::Critical), "Critical");
        }

        #[test]
        fn test_urgency_clone() {
            let u = Urgency::Normal;
            let cloned = u;
            assert_eq!(u, cloned);
        }
    }
}
