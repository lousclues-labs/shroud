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
mod notifications;
mod state;
mod supervisor;
mod tray;
mod util;

use log::{error, info};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::daemon::{acquire_instance_lock, release_instance_lock};
use crate::dbus::NmMonitor;
#[cfg(test)]
use crate::state::VpnState;
use crate::supervisor::VpnSupervisor;
use crate::tray::{SharedState, VpnCommand, VpnTray};

// Main
// ============================================================================

/// Install a panic hook that performs emergency cleanup
///
/// This ensures that kill switch rules are cleaned up even if Shroud panics.
/// Without this, a panic could leave iptables rules in place, locking out
/// the user from all network access.
fn install_panic_hook() {
    use killswitch::CleanupResult;

    let default_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        // Best-effort cleanup - don't panic again if this fails
        eprintln!("\n!!! SHROUD PANIC - attempting emergency cleanup !!!");

        // Try to clean up kill switch rules
        match killswitch::cleanup_with_fallback() {
            CleanupResult::Cleaned => {
                eprintln!("Kill switch rules cleaned up successfully.");
            }
            CleanupResult::NothingToClean => {
                eprintln!("No kill switch rules to clean up.");
            }
            CleanupResult::Failed(msg) => {
                eprintln!("Emergency kill switch cleanup failed: {}", msg);
                eprintln!("If you're locked out, manually run:");
                eprintln!("  sudo iptables -F SHROUD_KILLSWITCH");
                eprintln!("  sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH");
                eprintln!("  sudo iptables -X SHROUD_KILLSWITCH");
            }
        }

        // Clean up socket file
        let socket_path = ipc::protocol::socket_path();
        let _ = std::fs::remove_file(&socket_path);

        // Release lock
        daemon::release_instance_lock();

        // Call the default panic hook for the actual panic output
        default_hook(info);
    }));
}

/// Run daemon mode - start the tray application
async fn run_daemon_mode(args: cli::Args) {
    // Print startup banner FIRST (before any async/logging setup)
    println!("Shroud daemon starting... (use Ctrl+C to stop)");

    // HARDENING: Install panic hook for emergency cleanup
    // This ensures kill switch rules are cleaned up even on panic
    install_panic_hook();

    // Convert CLI args to logging args format
    let log_args = logging::Args {
        verbose: args.verbose,
        log_level: args.log_level,
        log_file: args.log_file,
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
            return;
        }
    };

    // Clean up any stale kill switch rules from previous crash
    killswitch::cleanup_stale_on_startup();

    let shared_state = Arc::new(RwLock::new(SharedState::default()));

    // Channels
    let (tx, rx) = mpsc::channel(16); // Tray commands
    let shutdown_tx = tx.clone();
    let shutdown_tx_clone = shutdown_tx.clone();

    ctrlc::set_handler(move || {
        info!("Shutdown signal received");
        if shutdown_tx_clone.try_send(VpnCommand::Quit).is_err() {
            info!("Supervisor not running, performing fallback cleanup");
            let _ = killswitch::cleanup_with_fallback();
            release_instance_lock();
            let socket_path = ipc::protocol::socket_path();
            if socket_path.exists() {
                let _ = std::fs::remove_file(&socket_path);
            }
            // Let process exit naturally
        }
    })
    .expect("Error setting Ctrl-C handler");

    info!("Starting Shroud VPN Manager");
    let (dbus_tx, dbus_rx) = mpsc::channel(32); // NM events
    let (ipc_tx, ipc_rx) = mpsc::channel(32); // IPC commands

    let tray_handle = Arc::new(std::sync::Mutex::new(None));

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
    let supervisor_handle = tokio::spawn(supervisor.run());

    let tray_service = VpnTray::new(tx);
    let tray_shutdown_tx = shutdown_tx.clone();

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
                let _ = tray_shutdown_tx.try_send(VpnCommand::Quit);
            }
        }
    });

    let _ = supervisor_handle.await;
    info!("Supervisor exited, shutting down");
}

#[tokio::main]
async fn main() -> ExitCode {
    // Parse command-line arguments using CLI module
    let args = match cli::args::parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("Error: {}", e);
            return ExitCode::from(1);
        }
    };

    // Determine mode based on whether a command was provided
    match args.command {
        Some(cli::args::ParsedCommand::Version { .. }) => {
            println!("shroud {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(_) => {
            // Client mode: send command to running daemon
            let code = cli::run_client_mode(&args).await;
            ExitCode::from(code as u8)
        }
        None => {
            // Check for headless mode
            let runtime_mode = mode::detect_mode(args.headless, args.desktop);

            match runtime_mode {
                mode::RuntimeMode::Headless => {
                    // Load config from system location for headless
                    let config = config::ConfigManager::new().load_validated();
                    match headless::run_headless(config).await {
                        Ok(()) => ExitCode::SUCCESS,
                        Err(e) => {
                            error!("Headless mode failed: {}", e);
                            ExitCode::FAILURE
                        }
                    }
                }
                mode::RuntimeMode::Desktop => {
                    // Desktop mode: start the tray application
                    run_daemon_mode(args).await;
                    ExitCode::SUCCESS
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
