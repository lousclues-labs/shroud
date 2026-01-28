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
//! - `killswitch/` - nftables-based traffic blocking
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

mod cli;
mod config;
mod daemon;
mod dbus;
mod health;
mod killswitch;
mod logging;
mod nm;
mod state;
mod tray;
mod supervisor;

use log::{error, info, warn};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};

use crate::config::ConfigManager;
use crate::daemon::{acquire_instance_lock, release_instance_lock};
use crate::dbus::NmMonitor;
use crate::supervisor::VpnSupervisor;
#[cfg(test)]
use crate::state::VpnState;
use crate::tray::{SharedState, VpnTray};

// Main
// ============================================================================

/// Run client mode - send command to daemon and exit
fn run_client_mode(args: &cli::Args) -> ! {
    use cli::client::{print_response, send_command, OutputFormat};
    use cli::{DebugAction, ParsedCommand, ToggleAction};

    let command = args.command.as_ref().unwrap();

    // Handle local commands that don't need the daemon
    match command {
        ParsedCommand::Help { command: Some(cmd) } => {
            cli::help::print_command_help(cmd);
            std::process::exit(0);
        }
        ParsedCommand::Help { command: None } => {
            cli::help::print_main_help();
            std::process::exit(0);
        }
        ParsedCommand::Debug {
            action: DebugAction::Tail,
        } => {
            // Tail is a local command
            let log_path = logging::log_directory().join("debug.log");
            let status = std::process::Command::new("tail")
                .arg("-f")
                .arg(&log_path)
                .status();
            match status {
                Ok(s) => std::process::exit(s.code().unwrap_or(1)),
                Err(e) => {
                    eprintln!("Failed to run tail: {}", e);
                    std::process::exit(1);
                }
            }
        }
        _ => {}
    }

    // Convert ParsedCommand to CliCommand for IPC
    let cli_command = match command {
        ParsedCommand::Connect { name } => cli::CliCommand::Connect { name: name.clone() },
        ParsedCommand::Disconnect => cli::CliCommand::Disconnect,
        ParsedCommand::Reconnect => cli::CliCommand::Reconnect,
        ParsedCommand::Switch { name } => cli::CliCommand::Switch { name: name.clone() },
        ParsedCommand::Status => cli::CliCommand::Status,
        ParsedCommand::List => cli::CliCommand::List,
        ParsedCommand::KillSwitch { action } => match action {
            ToggleAction::On => cli::CliCommand::KillSwitchOn,
            ToggleAction::Off => cli::CliCommand::KillSwitchOff,
            ToggleAction::Toggle => cli::CliCommand::KillSwitchToggle,
            ToggleAction::Status => cli::CliCommand::KillSwitchStatus,
        },
        ParsedCommand::AutoReconnect { action } => match action {
            ToggleAction::On => cli::CliCommand::AutoReconnectOn,
            ToggleAction::Off => cli::CliCommand::AutoReconnectOff,
            ToggleAction::Toggle => cli::CliCommand::AutoReconnectToggle,
            ToggleAction::Status => cli::CliCommand::AutoReconnectStatus,
        },
        ParsedCommand::Debug { action } => match action {
            DebugAction::On => cli::CliCommand::DebugOn,
            DebugAction::Off => cli::CliCommand::DebugOff,
            DebugAction::LogPath => cli::CliCommand::DebugLogPath,
            DebugAction::Dump => cli::CliCommand::DebugDump,
            DebugAction::Tail => unreachable!(), // Handled above
        },
        ParsedCommand::Ping => cli::CliCommand::Ping,
        ParsedCommand::Refresh => cli::CliCommand::Refresh,
        ParsedCommand::Quit => cli::CliCommand::Quit,
        ParsedCommand::Restart => cli::CliCommand::Restart,
        ParsedCommand::Help { .. } => unreachable!(), // Handled above
    };

    // Send command to daemon
    let format = if args.json_output {
        OutputFormat::Json
    } else {
        OutputFormat::Human
    };

    match send_command(cli_command, args.timeout) {
        Ok(response) => {
            let exit_code = print_response(&response, format, args.quiet);
            std::process::exit(exit_code);
        }
        Err(e) => {
            if !args.quiet {
                eprintln!("{}", e);
            }
            std::process::exit(e.exit_code());
        }
    }
}

/// Run daemon mode - start the tray application
async fn run_daemon_mode(args: cli::Args) {
    // Convert CLI args to logging args format
    let log_args = logging::Args {
        verbose: args.verbose,
        log_level: args.log_level,
        log_file: args.log_file,
        ..Default::default()
    };

    // Initialize logging
    logging::init_logging(&log_args);

    let _lock_file = match acquire_instance_lock() {
        Ok(file) => file,
        Err(msg) => {
            eprintln!("{}", msg);
            std::process::exit(1);
        }
    };

    // Clean up any stale kill switch rules from previous crash
    if killswitch::rules_exist() {
        warn!("Found stale kill switch rules from previous run, cleaning up...");
        killswitch::cleanup_stale_rules();
    }

    // Track start time for uptime reporting
    let start_time = Instant::now();

    ctrlc::set_handler(move || {
        info!("Shutdown signal received, cleaning up...");
        // Clean up kill switch rules (sync version for signal handler)
        killswitch::cleanup_stale_rules();
        release_instance_lock();
        // Clean up CLI socket
        let socket_path = cli::server::get_socket_path();
        let _ = std::fs::remove_file(&socket_path);
        std::process::exit(0);
    })
    .expect("Error setting Ctrl-C handler");

    info!("Starting Shroud VPN Manager");

    let shared_state = Arc::new(RwLock::new(SharedState::default()));
    let (tx, rx) = mpsc::channel(16);
    let (dbus_tx, dbus_rx) = mpsc::channel(32);
    let tray_handle = Arc::new(std::sync::Mutex::new(None));

    // Load config for sharing with CLI server
    let config_manager = ConfigManager::new();
    let app_config = Arc::new(RwLock::new(config_manager.load()));

    // Start CLI server for receiving commands
    let cli_server = match cli::CliServer::new().await {
        Ok(server) => Some(server),
        Err(e) => {
            warn!(
                "Failed to start CLI server: {}. CLI commands will not work.",
                e
            );
            None
        }
    };

    // Start D-Bus monitor for real-time NetworkManager events
    let nm_monitor = NmMonitor::new(dbus_tx);
    tokio::spawn(async move {
        if let Err(e) = nm_monitor.run().await {
            error!("D-Bus monitor failed: {}. Falling back to polling only.", e);
        }
    });

    // Spawn CLI connection handler if server is running
    if let Some(server) = cli_server {
        let cli_tx = tx.clone();
        let cli_state = shared_state.clone();
        let cli_config = app_config.clone();
        tokio::spawn(async move {
            loop {
                match server.accept().await {
                    Ok(stream) => {
                        let cmd_tx = cli_tx.clone();
                        let state = cli_state.clone();
                        let config = cli_config.clone();
                        let start = start_time;
                        tokio::spawn(async move {
                            cli::server::handle_cli_connection(
                                stream, cmd_tx, state, config, start,
                            )
                            .await;
                        });
                    }
                    Err(e) => {
                        warn!("Failed to accept CLI connection: {}", e);
                    }
                }
            }
        });
    }

    let supervisor = VpnSupervisor::new(shared_state.clone(), rx, dbus_rx, tray_handle.clone());
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
    let args = match cli::parse_args() {
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
            run_client_mode(&args);
        }
        None => {
            // Daemon mode: start the tray application
            run_daemon_mode(args).await;
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
