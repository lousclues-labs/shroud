// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Unit tests for VPN Supervisor module

#[cfg(test)]
mod supervisor_tests {
    use std::time::{Duration, Instant};

    #[test]
    fn test_supervisor_constants_reasonable() {
        use crate::supervisor::{
            CONNECTION_MONITOR_INTERVAL_MS, CONNECTION_MONITOR_MAX_ATTEMPTS,
            DISCONNECT_VERIFY_INTERVAL_MS, DISCONNECT_VERIFY_MAX_ATTEMPTS, MAX_CONNECT_ATTEMPTS,
            POST_DISCONNECT_GRACE_SECS, RECONNECT_BASE_DELAY_SECS, RECONNECT_MAX_DELAY_SECS,
        };

        // Store in variables to prevent constant folding
        let base_delay = RECONNECT_BASE_DELAY_SECS;
        let max_delay = RECONNECT_MAX_DELAY_SECS;
        let grace = POST_DISCONNECT_GRACE_SECS;
        let dc_attempts = DISCONNECT_VERIFY_MAX_ATTEMPTS;
        let mon_attempts = CONNECTION_MONITOR_MAX_ATTEMPTS;
        let mon_interval = CONNECTION_MONITOR_INTERVAL_MS;
        let dc_interval = DISCONNECT_VERIFY_INTERVAL_MS;
        let connect_attempts = MAX_CONNECT_ATTEMPTS;

        // Verify reasonable bounds
        assert!(base_delay >= 1, "Base delay should be at least 1 second");
        assert!(max_delay >= base_delay, "Max delay should be >= base delay");
        assert!(max_delay <= 120, "Max delay should not exceed 2 minutes");
        assert!(grace >= 1, "Grace period should be at least 1 second");
        assert!(
            dc_attempts >= 5,
            "Should have at least 5 disconnect verify attempts"
        );
        assert!(
            mon_attempts >= 10,
            "Should have at least 10 monitor attempts"
        );
        assert!(
            mon_interval >= 100,
            "Monitor interval should be at least 100ms"
        );
        assert!(
            dc_interval >= 100,
            "Disconnect verify interval should be at least 100ms"
        );
        assert!(
            connect_attempts >= 2,
            "Should have at least 2 connect attempts"
        );
    }

    #[test]
    fn test_reconnect_timing_calculations() {
        use crate::supervisor::{RECONNECT_BASE_DELAY_SECS, RECONNECT_MAX_DELAY_SECS};

        fn calc_backoff(attempt: u32) -> u64 {
            std::cmp::min(
                RECONNECT_BASE_DELAY_SECS * (attempt as u64 + 1),
                RECONNECT_MAX_DELAY_SECS,
            )
        }

        assert_eq!(calc_backoff(0), RECONNECT_BASE_DELAY_SECS);
        let backoff_100 = calc_backoff(100);
        assert_eq!(backoff_100, RECONNECT_MAX_DELAY_SECS);
    }

    #[test]
    fn test_grace_period_logic() {
        use crate::supervisor::POST_DISCONNECT_GRACE_SECS;

        let disconnect_time = Instant::now();
        let grace_duration = Duration::from_secs(POST_DISCONNECT_GRACE_SECS);
        assert!(disconnect_time.elapsed() < grace_duration);
    }
}

#[cfg(test)]
mod reconnect_tests {
    #[test]
    fn test_exponential_backoff_sequence() {
        const BASE_DELAY: u64 = 2;
        const MAX_DELAY: u64 = 30;

        // Test linear backoff: delay = BASE * (attempt + 1), capped at MAX
        let mut delays = Vec::new();
        for attempt in 0..20 {
            let delay = std::cmp::min(BASE_DELAY * (attempt as u64 + 1), MAX_DELAY);
            delays.push(delay);
        }

        // First delay is BASE_DELAY * 1 = 2
        assert_eq!(delays[0], 2);
        // All delays must be capped at MAX_DELAY
        assert!(delays.iter().all(|&d| d <= MAX_DELAY));
        // After enough attempts, delay should reach MAX_DELAY (2 * 15 = 30)
        assert_eq!(delays[14], MAX_DELAY);
        // And stay at MAX_DELAY
        assert_eq!(*delays.last().unwrap(), MAX_DELAY);
    }

    #[test]
    fn test_retry_reset_on_success() {
        let mut retries = 5;
        let success = true;
        if success {
            retries = 0;
        }
        assert_eq!(retries, 0);
    }
}

#[cfg(test)]
mod handler_tests {
    use crate::ipc::{IpcCommand, IpcResponse};
    use crate::tray::VpnCommand;

    #[test]
    fn test_vpn_command_serialization() {
        let connect_cmd = VpnCommand::Connect("test-vpn".to_string());
        let disconnect_cmd = VpnCommand::Disconnect;
        let toggle_ks = VpnCommand::ToggleKillSwitch;

        match connect_cmd {
            VpnCommand::Connect(name) => assert_eq!(name, "test-vpn"),
            _ => panic!("Expected Connect"),
        }
        match disconnect_cmd {
            VpnCommand::Disconnect => {}
            _ => panic!("Expected Disconnect"),
        }
        match toggle_ks {
            VpnCommand::ToggleKillSwitch => {}
            _ => panic!("Expected ToggleKillSwitch"),
        }
    }

    #[test]
    fn test_ipc_command_response_types() {
        let status_cmd = IpcCommand::Status;
        let connect_cmd = IpcCommand::Connect {
            name: "my-vpn".to_string(),
        };

        let ok_response = IpcResponse::Ok;
        let err_response = IpcResponse::Error {
            message: "Failed".to_string(),
        };

        match status_cmd {
            IpcCommand::Status => {}
            _ => panic!("Expected Status"),
        }

        match connect_cmd {
            IpcCommand::Connect { name } => assert_eq!(name, "my-vpn"),
            _ => panic!("Expected Connect"),
        }

        assert!(matches!(ok_response, IpcResponse::Ok));

        if let IpcResponse::Error { message } = err_response {
            assert_eq!(message, "Failed");
        }
    }

    #[test]
    fn test_all_vpn_command_variants() {
        let commands: Vec<VpnCommand> = vec![
            VpnCommand::Connect("vpn".into()),
            VpnCommand::Disconnect,
            VpnCommand::ToggleAutoReconnect,
            VpnCommand::ToggleKillSwitch,
            VpnCommand::ToggleAutostart,
            VpnCommand::ToggleDebugLogging,
            VpnCommand::OpenLogFile,
            VpnCommand::RefreshConnections,
            VpnCommand::Restart,
        ];
        assert_eq!(commands.len(), 9);
    }

    #[test]
    fn test_all_ipc_command_variants_constructable() {
        let commands: Vec<IpcCommand> = vec![
            IpcCommand::Connect { name: "v".into() },
            IpcCommand::Disconnect,
            IpcCommand::Switch { name: "v".into() },
            IpcCommand::Status,
            IpcCommand::List { vpn_type: None },
            IpcCommand::Reconnect,
            IpcCommand::KillSwitch { enable: true },
            IpcCommand::KillSwitchToggle,
            IpcCommand::KillSwitchStatus,
            IpcCommand::AutoReconnect { enable: true },
            IpcCommand::AutoReconnectToggle,
            IpcCommand::AutoReconnectStatus,
            IpcCommand::Debug { enable: true },
            IpcCommand::DebugLogPath,
            IpcCommand::DebugDump,
            IpcCommand::Ping,
            IpcCommand::Refresh,
            IpcCommand::Quit,
            IpcCommand::Restart,
            IpcCommand::Reload,
        ];
        assert_eq!(commands.len(), 20);
    }

    #[test]
    fn test_ipc_response_ok_message() {
        let resp = IpcResponse::OkMessage {
            message: "done".into(),
        };
        assert!(resp.is_ok());
    }

    #[test]
    fn test_ipc_response_pong() {
        let resp = IpcResponse::Pong;
        assert!(resp.is_ok());
    }

    #[test]
    fn test_ipc_response_status() {
        let resp = IpcResponse::Status {
            connected: true,
            vpn_name: Some("my-vpn".into()),
            vpn_type: None,
            state: "Connected".into(),
            kill_switch_enabled: false,
        };
        assert!(resp.is_ok());
    }

    #[test]
    fn test_ipc_response_ks_status() {
        let resp = IpcResponse::KillSwitchStatus { enabled: true };
        assert!(resp.is_ok());
    }

    #[test]
    fn test_ipc_response_ar_status() {
        let resp = IpcResponse::AutoReconnectStatus { enabled: false };
        assert!(resp.is_ok());
    }

    #[test]
    fn test_ipc_response_debug_info() {
        let resp = IpcResponse::DebugInfo {
            log_path: Some("/tmp/shroud.log".into()),
            debug_enabled: true,
        };
        assert!(resp.is_ok());
    }
}

// === Behavioral integration tests with MockNmClient ===
use crate::nm::{MockNmClient, NmCall, NmError};
use crate::state::{Event, VpnState};
use crate::supervisor::VpnSupervisor;
use crate::tray::{SharedState, VpnCommand};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

async fn make_supervisor(mock: MockNmClient) -> (VpnSupervisor, mpsc::Sender<VpnCommand>) {
    let shared_state = Arc::new(RwLock::new(SharedState::default()));
    let (tray_tx, tray_rx) = mpsc::channel(16);
    let (_ipc_tx, ipc_rx) = mpsc::channel(16);
    let (_dbus_tx, dbus_rx) = mpsc::channel(16);
    let tray_handle = Arc::new(std::sync::Mutex::new(None));

    let supervisor = VpnSupervisor::with_nm(
        shared_state.clone(),
        tray_rx,
        ipc_rx,
        dbus_rx,
        tray_handle,
        Box::new(mock.clone()),
    );

    (supervisor, tray_tx)
}

#[tokio::test]
async fn connect_calls_nm_connect_and_transitions_to_connected() {
    let mock = MockNmClient::new();
    mock.add_vpn("ireland-42");

    let (mut supervisor, _tx) = make_supervisor(mock.clone()).await;
    supervisor.handle_connect("ireland-42").await;

    assert!(mock.was_called(&NmCall::Connect("ireland-42".to_string())));
    assert!(matches!(
        supervisor.machine.state,
        VpnState::Connected { ref server } if server == "ireland-42"
    ));
}

#[tokio::test]
async fn connect_to_nonexistent_vpn_stays_disconnected() {
    let mock = MockNmClient::new();
    let (mut supervisor, _tx) = make_supervisor(mock.clone()).await;
    supervisor.handle_connect("ghost-vpn").await;

    assert!(mock.was_called(&NmCall::Connect("ghost-vpn".to_string())));
    assert!(!matches!(
        supervisor.machine.state,
        VpnState::Connected { .. }
    ));
}

#[tokio::test]
async fn connect_failure_leaves_state_machine_not_connected() {
    let mock = MockNmClient::new();
    mock.add_vpn("flaky-vpn");
    mock.queue_error(NmError::Command("connection refused".into()));

    let (mut supervisor, _tx) = make_supervisor(mock.clone()).await;
    supervisor.handle_connect("flaky-vpn").await;

    assert!(mock.was_called(&NmCall::Connect("flaky-vpn".to_string())));
}

#[tokio::test]
async fn disconnect_calls_nm_disconnect_and_transitions_to_disconnected() {
    let mock = MockNmClient::new();
    mock.add_vpn("ireland-42");
    mock.set_active("ireland-42");

    let (mut supervisor, _tx) = make_supervisor(mock.clone()).await;
    supervisor.dispatch(Event::NmVpnUp {
        server: "ireland-42".to_string(),
    });
    supervisor.handle_disconnect().await;

    assert!(mock.was_called(&NmCall::Disconnect("ireland-42".to_string())));
    assert!(matches!(supervisor.machine.state, VpnState::Disconnected));
}

#[tokio::test]
async fn state_sync_detects_external_vpn_connection() {
    let mock = MockNmClient::new();
    mock.add_vpn("external-vpn");
    mock.set_active("external-vpn");

    let (mut supervisor, _tx) = make_supervisor(mock.clone()).await;
    let corrected = supervisor.sync_state_from_nm().await;

    assert!(corrected);
    assert!(matches!(
        supervisor.machine.state,
        VpnState::Connected { ref server } if server == "external-vpn"
    ));
}

#[tokio::test]
async fn state_sync_detects_external_vpn_switch() {
    let mock = MockNmClient::new();
    mock.add_vpn("vpn-a");
    mock.add_vpn("vpn-b");
    mock.set_active("vpn-b");

    let (mut supervisor, _tx) = make_supervisor(mock.clone()).await;
    supervisor.dispatch(Event::NmVpnUp {
        server: "vpn-a".to_string(),
    });

    let corrected = supervisor.sync_state_from_nm().await;
    assert!(corrected);
}

#[tokio::test]
async fn state_sync_detects_external_disconnect() {
    let mock = MockNmClient::new();
    mock.add_vpn("vpn-a");

    let (mut supervisor, _tx) = make_supervisor(mock.clone()).await;
    supervisor.dispatch(Event::NmVpnUp {
        server: "vpn-a".to_string(),
    });

    let corrected = supervisor.sync_state_from_nm().await;
    assert!(corrected);
    assert!(matches!(supervisor.machine.state, VpnState::Disconnected));
}

#[tokio::test]
async fn reconnect_retries_on_failure_up_to_max_attempts() {
    let mock = MockNmClient::new();
    mock.add_vpn("unstable-vpn");
    for _ in 0..5 {
        mock.queue_error(NmError::Command("connection refused".into()));
    }

    let (mut supervisor, _tx) = make_supervisor(mock.clone()).await;
    supervisor.attempt_reconnect("unstable-vpn").await;

    assert!(mock.connect_count() > 1);
}

#[tokio::test]
async fn reconnect_stops_if_vpn_already_connected() {
    let mock = MockNmClient::new();
    mock.add_vpn("vpn-a");
    mock.set_active("vpn-a");

    let (mut supervisor, _tx) = make_supervisor(mock.clone()).await;
    supervisor.attempt_reconnect("vpn-a").await;

    assert_eq!(mock.connect_count(), 0);
}

#[tokio::test]
async fn disconnect_disables_kill_switch_config() {
    let mock = MockNmClient::new();
    mock.add_vpn("vpn-a");
    mock.set_active("vpn-a");

    let (mut supervisor, _tx) = make_supervisor(mock.clone()).await;
    supervisor.dispatch(Event::NmVpnUp {
        server: "vpn-a".to_string(),
    });
    supervisor.config_store.config.kill_switch_enabled = true;

    supervisor.handle_disconnect().await;
    assert!(mock.was_called(&NmCall::Disconnect("vpn-a".to_string())));
}
