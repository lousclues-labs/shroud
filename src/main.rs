//! # Shroud
//!
//! A provider-agnostic VPN connection manager for Linux.
//!
//! Shroud wraps around NetworkManager and OpenVPN like a protective shroud
//! around a lock mechanism — hardening security without replacing the tools
//! you already have.
//!
//! ## Architecture
//!
//! - `state/` - Formal state machine types and transitions
//! - `nm/` - NetworkManager interface (nmcli + D-Bus events)
//! - `tray/` - System tray UI (ksni/StatusNotifierItem)
//! - `killswitch/` - iptables-based traffic blocking
//! - `health/` - VPN tunnel connectivity verification
//! - `config/` - Persistent user settings
//!
//! ## State Machine
//!
//! The supervisor uses a formal state machine that processes events:
//! - User events: UserEnable, UserDisable
//! - NM events: NmVpnUp, NmVpnDown, NmVpnChanged
//! - Health events: HealthOk, HealthDegraded, HealthDead
//! - System events: Wake (from sleep)
//! - Internal events: Timeout
//!
//! All state transitions go through StateMachine::handle_event() which logs
//! every transition with its reason. State is sacred — if the state says
//! Disconnected, we are disconnected.

mod autostart;
mod cli;
mod config;
mod daemon;
mod dbus;
mod gateway;
mod headless;
mod health;
mod import;
mod ipc;
mod killswitch;
mod logging;
mod mode;
mod nm;
mod state;
mod supervisor;
mod tray;

use log::{error, info};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::config::ConfigManager;
use crate::daemon::{acquire_instance_lock, release_instance_lock};
use crate::dbus::NmMonitor;
#[cfg(test)]
use crate::state::VpnState;
use crate::supervisor::VpnSupervisor;
use crate::tray::{SharedState, VpnTray};

// Main
// ============================================================================

/// Run daemon mode - start the tray application
async fn run_daemon_mode(args: cli::Args) {
    // Print startup banner FIRST (before any async/logging setup)
    println!("Shroud daemon starting... (use Ctrl+C to stop)");
    
    // Convert CLI args to logging args format
    let log_args = logging::Args {
        verbose: args.verbose,
        log_level: args.log_level,
        log_file: args.log_file,
        ..Default::default()
    };

    // Initialize logging
    logging::init_logging(&log_args);

    if log::log_enabled!(log::Level::Debug) {
        killswitch::paths::log_detected_paths();
    }

    killswitch::validate_sudoers_on_startup();

    let _lock_file = match acquire_instance_lock() {
        Ok(file) => file,
        Err(msg) => {
            eprintln!("{}", msg);
            std::process::exit(1);
        }
    };

    // Clean up any stale kill switch rules from previous crash
    killswitch::cleanup_stale_on_startup();

    ctrlc::set_handler(move || {
        info!("Shutdown signal received, cleaning up...");
        // Non-blocking cleanup with timeout
        let _ = killswitch::cleanup_with_fallback();
        release_instance_lock();
        // Clean up CLI socket
        let socket_path = ipc::protocol::socket_path();
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }
        std::process::exit(0);
    })
    .expect("Error setting Ctrl-C handler");

    info!("Starting Shroud VPN Manager");

    let shared_state = Arc::new(RwLock::new(SharedState::default()));

    // Channels
    let (tx, rx) = mpsc::channel(16); // Tray commands
    let (dbus_tx, dbus_rx) = mpsc::channel(32); // NM events
    let (ipc_tx, ipc_rx) = mpsc::channel(32); // IPC commands

    let tray_handle = Arc::new(std::sync::Mutex::new(None));

    // Load config just to force init or validation if needed
    let config_manager = ConfigManager::new();
    let _ = config_manager.load_validated();

    // Start IPC Server
    let ipc_server = ipc::IpcServer::new(ipc_tx);
    tokio::spawn(async move {
        if let Err(e) = ipc_server.run().await {
            error!("IPC server failed: {}", e);
        }
    });

    // Start D-Bus monitor for real-time NetworkManager events
    let nm_monitor = NmMonitor::new(dbus_tx);
    tokio::spawn(async move {
        if let Err(e) = nm_monitor.run().await {
            error!("D-Bus monitor failed: {}. Falling back to polling only.", e);
        }
    });

    let supervisor = VpnSupervisor::new(
        shared_state.clone(),
        rx,
        ipc_rx,
        dbus_rx,
        tray_handle.clone(),
    );
    tokio::spawn(supervisor.run());

    let tray_service = VpnTray::new(tx);

    info!("Starting system tray");
    let tray_handle_clone = tray_handle.clone();
    std::thread::spawn(move || {
        use ksni::blocking::TrayMethods;
        match tray_service.spawn() {
            Ok(handle) => {
                if let Ok(mut guard) = tray_handle_clone.lock() {
                    *guard = Some(handle);
                }
            }
            Err(e) => {
                error!("Failed to spawn tray: {}", e);
                std::process::exit(1);
            }
        }
    });

    std::future::pending::<()>().await;
}

#[tokio::main]
async fn main() {
    // Parse command-line arguments using CLI module
    let args = match cli::args::parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    // Determine mode based on whether a command was provided
    match args.command {
        Some(_) => {
            // Client mode: send command to running daemon
            let code = cli::run_client_mode(&args).await;
            std::process::exit(code);
        }
        None => {
            // Check for headless mode
            let runtime_mode = mode::detect_mode(args.headless, args.desktop);

            match runtime_mode {
                mode::RuntimeMode::Headless => {
                    // Load config from system location for headless
                    let config = config::ConfigManager::new().load_validated();
                    match headless::run_headless(config).await {
                        Ok(()) => std::process::exit(0),
                        Err(e) => {
                            error!("Headless mode failed: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                mode::RuntimeMode::Desktop => {
                    // Desktop mode: start the tray application
                    run_daemon_mode(args).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vpn_state_server_name() {
        let state = VpnState::Connected {
            server: "test".to_string(),
        };
        assert_eq!(state.server_name(), Some("test"));

        let state = VpnState::Disconnected;
        assert_eq!(state.server_name(), None);
    }
}
