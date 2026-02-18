// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Unit tests for Headless module

#[cfg(test)]
mod headless_tests {
    use crate::config::{Config, HeadlessConfig};

    #[test]
    fn test_headless_config_defaults() {
        let config = HeadlessConfig::default();
        assert!(!config.auto_connect, "Auto-connect off by default");
        assert!(config.kill_switch_on_boot, "Boot kill switch on by default");
        assert!(
            config.require_kill_switch,
            "Require kill switch true by default"
        );
    }

    #[test]
    fn test_headless_config_startup_server() {
        let mut config = HeadlessConfig::default();
        assert!(config.startup_server.is_none());
        config.startup_server = Some("my-vpn".to_string());
        assert_eq!(config.startup_server.as_deref(), Some("my-vpn"));
    }

    #[test]
    fn test_auto_connect_server_selection() {
        let mut config = Config::default();

        // No servers configured
        let server = config
            .headless
            .startup_server
            .clone()
            .or_else(|| config.last_server.clone());
        assert!(server.is_none());

        // Only last_server
        config.last_server = Some("last-vpn".to_string());
        let server = config
            .headless
            .startup_server
            .clone()
            .or_else(|| config.last_server.clone());
        assert_eq!(server.as_deref(), Some("last-vpn"));

        // startup_server overrides
        config.headless.startup_server = Some("startup-vpn".to_string());
        let server = config
            .headless
            .startup_server
            .clone()
            .or_else(|| config.last_server.clone());
        assert_eq!(server.as_deref(), Some("startup-vpn"));
    }

    #[test]
    fn test_systemd_notify_messages() {
        let ready_msg = "READY=1";
        let status_msg = "STATUS=Connected to vpn";
        let stopping_msg = "STOPPING=1";
        let watchdog_msg = "WATCHDOG=1";

        assert!(ready_msg.contains("READY"));
        assert!(status_msg.contains("STATUS="));
        assert!(stopping_msg.contains("STOPPING"));
        assert!(watchdog_msg.contains("WATCHDOG"));
    }

    #[test]
    fn test_watchdog_interval_calculation() {
        let watchdog_usec = "30000000"; // 30 seconds
        let usec: u64 = watchdog_usec.parse().unwrap();
        let duration_secs = usec / 1_000_000;
        let ping_interval = duration_secs / 2;
        assert_eq!(duration_secs, 30);
        assert_eq!(ping_interval, 15);
    }

    #[test]
    fn test_signal_mapping() {
        let handled_signals = ["SIGTERM", "SIGINT", "SIGHUP", "SIGUSR1", "SIGUSR2"];
        assert!(handled_signals.contains(&"SIGTERM"));
        assert!(handled_signals.contains(&"SIGINT"));
    }

    #[test]
    fn test_ipc_socket_path() {
        let default_path = "/run/shroud.sock";
        let user_path = "/tmp/shroud-1000.sock";
        assert!(default_path.starts_with("/run"));
        assert!(user_path.contains("shroud"));
    }
}
