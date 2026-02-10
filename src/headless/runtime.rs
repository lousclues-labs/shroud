//! Headless runtime implementation.

use crate::config::Config;
use crate::dbus::NmMonitor;
use crate::headless::systemd;
use crate::ipc::server::IpcServer;
use crate::killswitch::boot::{disable_boot_killswitch, enable_boot_killswitch};
use crate::killswitch::cleanup;
use crate::nm;
use crate::supervisor::VpnSupervisor;
use crate::tray::SharedState;
use log::{debug, error, info, warn};
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::{mpsc, RwLock};

/// Run Shroud in headless mode.
///
/// This is the main entry point for server operation.
/// It sets up:
/// 1. Boot kill switch (before VPN connects)
/// 2. IPC server (for CLI commands)
/// 3. Supervisor (VPN management)
/// 4. Signal handlers (graceful shutdown)
/// 5. Auto-connect (if configured)
pub async fn run_headless(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting Shroud in headless mode");

    // Notify systemd we're starting
    systemd::notify_status("Initializing...");

    // Step 1: Enable boot kill switch if configured
    if config.headless.kill_switch_on_boot {
        info!("Enabling boot kill switch");
        match enable_boot_killswitch(config.killswitch.allow_lan) {
            Ok(()) => info!("Boot kill switch enabled"),
            Err(e) => {
                error!("Failed to enable boot kill switch: {}", e);
                if config.headless.require_kill_switch {
                    return Err(e.into());
                }
                warn!("Continuing without boot kill switch (require_kill_switch = false)");
            }
        }
    }

    // Step 2: Set up channels (similar to desktop mode, but no tray)
    let shared_state = Arc::new(RwLock::new(SharedState::default()));

    // Channels
    let (_tray_tx, tray_rx) = mpsc::channel(16); // Unused in headless but needed for supervisor
    let (dbus_tx, dbus_rx) = mpsc::channel(32); // NM events
    let (ipc_tx, ipc_rx) = mpsc::channel(32); // IPC commands

    // No tray handle in headless mode
    let tray_handle = Arc::new(std::sync::Mutex::new(None));

    // Step 3: Start IPC server
    let ipc_server = IpcServer::new(ipc_tx);
    let ipc_handle = tokio::spawn(async move {
        if let Err(e) = ipc_server.run().await {
            error!("IPC server failed: {}", e);
        }
    });

    // Step 4: Start D-Bus monitor for NetworkManager events
    let nm_monitor = NmMonitor::new(dbus_tx);
    let dbus_handle = tokio::spawn(async move {
        if let Err(e) = nm_monitor.run().await {
            error!("D-Bus monitor failed: {}. Falling back to polling only.", e);
        }
    });

    // Step 5: Create and start supervisor
    let supervisor = VpnSupervisor::new(
        shared_state.clone(),
        tray_rx,
        ipc_rx,
        dbus_rx,
        tray_handle.clone(),
    );
    let supervisor_handle = tokio::spawn(supervisor.run());

    // Step 6: Auto-connect if configured
    if config.headless.auto_connect {
        let server_name = config
            .headless
            .startup_server
            .clone()
            .or_else(|| config.last_server.clone());

        if let Some(server) = server_name {
            info!("Auto-connecting to: {}", server);
            systemd::notify_status(&format!("Connecting to {}...", server));

            // Send connect command via IPC channel pattern
            // In headless mode, we can use nmcli directly for initial connect
            match auto_connect_nmcli(&server, &config.headless).await {
                Ok(()) => {
                    info!("Auto-connect successful");
                    systemd::notify_status(&format!("Connected to {}", server));

                    // Transition from boot kill switch to runtime kill switch
                    if config.headless.kill_switch_on_boot {
                        let _ = disable_boot_killswitch();
                    }
                }
                Err(e) => {
                    error!("Auto-connect failed: {}", e);
                    if config.headless.require_kill_switch {
                        // Keep boot kill switch active
                        systemd::notify_status("Auto-connect failed, kill switch active");
                    }
                }
            }
        } else {
            warn!("auto_connect enabled but no startup_server or last_server configured");
        }
    }

    // Step 7: Notify systemd we're ready
    systemd::notify_ready();
    info!("Shroud headless mode ready");

    // Step 8: Set up signal handlers
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sighup = signal(SignalKind::hangup())?;
    let mut sigusr1 = signal(SignalKind::user_defined1())?;
    let mut sigusr2 = signal(SignalKind::user_defined2())?;

    // Step 9: Watchdog loop
    let shared_state_watchdog = Arc::clone(&shared_state);
    let watchdog_handle = tokio::spawn(async move {
        let interval = systemd::watchdog_interval();
        if let Some(interval) = interval {
            let mut ticker = tokio::time::interval(interval / 2);
            loop {
                ticker.tick().await;

                // Check current state
                let state = shared_state_watchdog.read().await;
                let status = format!("State: {:?}", state.state);
                drop(state);

                // Update systemd status
                systemd::notify_status(&status);
                systemd::notify_watchdog();
            }
        }
    });

    // Step 10: Wait for shutdown signal (ignoring non-fatal signals)
    let shutdown_reason = loop {
        tokio::select! {
            _ = sigterm.recv() => break "SIGTERM",
            _ = sigint.recv() => break "SIGINT",
            _ = sighup.recv() => {
                info!("Received SIGHUP, reloading config");
                warn!("Config reload not yet implemented");
                // Don't shutdown on SIGHUP, just log and continue
                continue;
            }
            _ = sigusr1.recv() => {
                debug!("Received SIGUSR1, ignoring");
                continue;
            }
            _ = sigusr2.recv() => {
                debug!("Received SIGUSR2, ignoring");
                continue;
            }
        }
    };

    info!("Received {}, shutting down", shutdown_reason);

    // Step 11: Graceful shutdown
    shutdown(
        &config,
        watchdog_handle,
        ipc_handle,
        dbus_handle,
        supervisor_handle,
    )
    .await;

    info!("Shroud headless shutdown complete");
    Ok(())
}

/// Auto-connect using nmcli directly with exponential backoff.
async fn auto_connect_nmcli(
    server: &str,
    config: &crate::config::HeadlessConfig,
) -> Result<(), String> {
    use crate::util::backoff::{jitter_millis, linear_backoff_secs};
    use std::time::Duration;

    let max_attempts = if config.max_reconnect_attempts == 0 {
        u32::MAX // Infinite retries
    } else {
        config.max_reconnect_attempts
    };
    let base_delay = Duration::from_secs(config.reconnect_delay_secs);
    let max_delay = Duration::from_secs(300); // Cap at 5 minutes

    for attempt in 1..=max_attempts {
        info!(
            "Auto-connect attempt {}/{} for {}",
            attempt,
            if max_attempts == u32::MAX {
                "∞".to_string()
            } else {
                max_attempts.to_string()
            },
            server
        );

        match nm::connect(server).await {
            Ok(()) => {
                // Wait a bit for connection to stabilize
                tokio::time::sleep(Duration::from_secs(2)).await;

                // Verify connection
                match nm::get_active_vpn().await {
                    Some(active) if active == server => {
                        info!("VPN connection verified: {}", server);
                        return Ok(());
                    }
                    Some(active) => {
                        debug!("Connected to different VPN: {}", active);
                        return Ok(());
                    }
                    None => {
                        warn!("VPN connection not active after connect command");
                    }
                }
            }
            Err(e) => {
                warn!("Connect attempt {} failed: {}", attempt, e);
            }
        }

        if attempt < max_attempts {
            // Calculate delay with linear backoff and jitter
            let delay = linear_backoff_secs(base_delay.as_secs(), max_delay.as_secs(), attempt);
            let total_delay = delay + jitter_millis(1000);

            info!("Retrying in {:?}", total_delay);
            tokio::time::sleep(total_delay).await;
        }
    }

    Err(format!(
        "Failed to connect to {} after {} attempts",
        server, max_attempts
    ))
}

/// Perform graceful shutdown.
async fn shutdown(
    config: &Config,
    watchdog_handle: tokio::task::JoinHandle<()>,
    ipc_handle: tokio::task::JoinHandle<()>,
    dbus_handle: tokio::task::JoinHandle<()>,
    supervisor_handle: tokio::task::JoinHandle<()>,
) {
    systemd::notify_stopping();
    info!("Initiating graceful shutdown");

    // Disable kill switch (if not configured to persist)
    if !config.headless.persist_kill_switch {
        info!("Disabling kill switch");
        if let Err(e) = cleanup::cleanup_all() {
            warn!("Error disabling kill switch: {}", e);
        }
    } else {
        info!("Persisting kill switch (persist_kill_switch = true)");
    }

    // Cancel background tasks
    watchdog_handle.abort();
    ipc_handle.abort();
    dbus_handle.abort();
    supervisor_handle.abort();

    // Await task termination with timeout
    let timeout = tokio::time::Duration::from_secs(5);
    let _ = tokio::time::timeout(timeout, async {
        let _ = watchdog_handle.await;
        let _ = ipc_handle.await;
        let _ = dbus_handle.await;
        let _ = supervisor_handle.await;
    })
    .await;

    // Clean up IPC socket
    let socket_path = crate::ipc::protocol::socket_path();
    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }
}
