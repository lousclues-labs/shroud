//! CLI command handlers.
//!
//! Implements the client-side execution of CLI commands by communicating
//! with the daemon over IPC.

use log::{error, info};

use crate::ipc::client::{send_command, ClientError};
use crate::ipc::protocol::{IpcCommand, IpcResponse};
use crate::logging;
use super::args::{Args, ParsedCommand, ToggleAction, DebugAction};
use super::help;

/// Run the CLI in client mode.
///
/// Parses the command from arguments and sends it to the daemon.
///
/// # Returns
///
/// Exit code: 0 for success, non-zero for errors.
pub async fn run_client_mode(args: &Args) -> i32 {
    let command = match &args.command {
        Some(cmd) => cmd,
        None => return 0, // Should likely not happen if parsing enforced it, but explicit check safe
    };

    // Handle local commands that don't need the daemon
    match command {
        ParsedCommand::Help { command: Some(cmd) } => {
            help::print_command_help(cmd);
            return 0;
        }
        ParsedCommand::Help { command: None } => {
            help::print_main_help();
            return 0;
        }
        ParsedCommand::Debug {
            action: DebugAction::Tail,
        } => {
            // Tail is a local command
            // We assume logging module is available via crate root
            let log_path = logging::log_directory().join("debug.log");
            let status = std::process::Command::new("tail")
                .arg("-f")
                .arg(&log_path)
                .status();
            match status {
                Ok(s) => return s.code().unwrap_or(1),
                Err(e) => {
                    eprintln!("Failed to run tail: {}", e);
                    return 1;
                }
            }
        }
        _ => {}
    }

    // Convert CLI args to IPC command
    let ipc_command = match args_to_command(command) {
        Some(cmd) => cmd,
        None => {
            error!("Invalid command");
            return 1;
        }
    };

    info!("Sending command: {:?}", ipc_command);

    match send_command(ipc_command).await {
        Ok(response) => handle_response(response, args),
        Err(e) => {
            match e {
                ClientError::DaemonNotRunning => {
                    eprintln!("Error: Shroud daemon is not running.");
                    eprintln!("Start it with: shroud --daemon");
                    // Special exit code for daemon not running?
                    // Standard practice is 1, but maybe another is better. Sticking to 1.
                }
                _ => {
                    eprintln!("Error: {}", e);
                }
            }
            1
        }
    }
}

/// Convert CLI arguments to an IPC command.
fn args_to_command(cmd: &ParsedCommand) -> Option<IpcCommand> {
    match cmd {
        ParsedCommand::Connect { name } => Some(IpcCommand::Connect { name: name.clone() }),
        ParsedCommand::Disconnect => Some(IpcCommand::Disconnect),
        ParsedCommand::Reconnect => Some(IpcCommand::Reconnect),
        ParsedCommand::Switch { name } => Some(IpcCommand::Switch { name: name.clone() }),
        ParsedCommand::Status => Some(IpcCommand::Status),
        ParsedCommand::List => Some(IpcCommand::List),
        
        ParsedCommand::KillSwitch { action } => match action {
            ToggleAction::On => Some(IpcCommand::KillSwitch { enable: true }),
            ToggleAction::Off => Some(IpcCommand::KillSwitch { enable: false }),
            ToggleAction::Toggle => Some(IpcCommand::KillSwitchToggle),
            ToggleAction::Status => Some(IpcCommand::KillSwitchStatus),
        },
        
        ParsedCommand::AutoReconnect { action } => match action {
            ToggleAction::On => Some(IpcCommand::AutoReconnect { enable: true }),
            ToggleAction::Off => Some(IpcCommand::AutoReconnect { enable: false }),
            ToggleAction::Toggle => Some(IpcCommand::AutoReconnectToggle),
            ToggleAction::Status => Some(IpcCommand::AutoReconnectStatus),
        },
        
        ParsedCommand::Debug { action } => match action {
            DebugAction::On => Some(IpcCommand::Debug { enable: true }),
            DebugAction::Off => Some(IpcCommand::Debug { enable: false }),
            DebugAction::Dump => Some(IpcCommand::DebugDump),
            DebugAction::LogPath => Some(IpcCommand::DebugLogPath),
            DebugAction::Tail => None, // Handled locally
        },

        ParsedCommand::Ping => Some(IpcCommand::Ping),
        ParsedCommand::Refresh => Some(IpcCommand::Refresh),
        ParsedCommand::Quit => Some(IpcCommand::Quit),
        ParsedCommand::Restart => Some(IpcCommand::Restart),
        
        ParsedCommand::Help { .. } => None, // Handled locally
    }
}

/// Handle and display a response from the daemon.
///
/// Returns exit code: 0 for success, 1 for errors.
fn handle_response(response: IpcResponse, args: &Args) -> i32 {
    let json = args.json_output;

    if json {
        // Just dump the JSON structure of the response
        match serde_json::to_string_pretty(&response) {
            Ok(s) => println!("{}", s),
            Err(e) => {
                eprintln!("Error serializing response: {}", e);
                return 1;
            }
        }
        return if response.is_ok() { 0 } else { 1 };
    }

    match response {
        IpcResponse::Ok => {
            if !args.quiet {
                println!("OK");
            }
            0
        }
        IpcResponse::Error { message } => {
            eprintln!("Error: {}", message);
            1
        }
        IpcResponse::Status { connected, vpn_name, state, kill_switch_enabled } => {
            println!("Status: {}", state);
            if connected {
                println!("Connected to: {}", vpn_name.unwrap_or_else(|| "unknown".to_string()));
            } else {
                println!("Not connected");
            }
            println!("Kill switch: {}", if kill_switch_enabled { "enabled" } else { "disabled" });
            0
        }
        IpcResponse::Connections { names } => {
            println!("Available VPN connections:");
            for name in names {
                println!("  - {}", name);
            }
            0
        }
        IpcResponse::Value(v) => {
            // Generic fallback
            if let Some(obj) = v.as_object() {
                if let Some(msg) = obj.get("message") {
                     if let Some(s) = msg.as_str() {
                         println!("{}", s);
                         return 0;
                     }
                }
            }
            // Fallback print
            println!("{}", v);
            0
        }
    }
}
