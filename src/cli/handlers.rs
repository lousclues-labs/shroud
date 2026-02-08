//! CLI command handlers.
//!
//! Implements the client-side execution of CLI commands by communicating
//! with the daemon over IPC.

use log::{error, info};

use super::args::{Args, DebugAction, GatewayAction, ParsedCommand, ToggleAction};
use super::help;
use super::import as import_command;
use crate::ipc::client::{send_command, ClientError};
use crate::ipc::protocol::{IpcCommand, IpcResponse};
use crate::killswitch::verify;
use crate::logging;

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
        ParsedCommand::Autostart { action } => {
            return handle_autostart_command(*action, args);
        }
        ParsedCommand::Cleanup => {
            return handle_cleanup_command(args).await;
        }
        ParsedCommand::Version { check } => {
            return handle_version_command(*check).await;
        }
        ParsedCommand::Update => {
            return handle_update_command();
        }
        ParsedCommand::Doctor => {
            return handle_doctor_command();
        }
        ParsedCommand::VerifyKillswitch { json, verbose } => {
            let json_flag = *json || args.json_output;
            return handle_verify_killswitch_command(json_flag, *verbose || args.verbose >= 2)
                .await;
        }
        ParsedCommand::Gateway { action } => {
            return handle_gateway_command(*action).await;
        }
        ParsedCommand::Import { options } => {
            let mut merged = options.clone();
            if args.json_output {
                merged.json = true;
            }
            if args.quiet {
                merged.quiet = true;
            }
            return import_command::execute(merged).await;
        }
        ParsedCommand::Help { command: Some(cmd) } => {
            help::print_command_help(cmd);
            return 0;
        }
        ParsedCommand::Help { command: None } => {
            help::print_main_help();
            return 0;
        }
        ParsedCommand::Debug {
            action: DebugAction::Tail { verbose },
        } => {
            // Track whether we enabled logging (so we can disable on exit)
            let log_path = logging::default_log_path();
            let we_enabled = if !logging::is_debug_logging_enabled() {
                match send_command(IpcCommand::Debug { enable: true }).await {
                    Ok(_) => {
                        eprintln!("Debug logging enabled: {}", log_path.display());
                        true
                    }
                    Err(_) => {
                        eprintln!("Note: daemon not running, tailing existing log file");
                        false
                    }
                }
            } else {
                false
            };

            if *verbose {
                eprintln!(
                    "Tailing {} (all levels, Ctrl+C to stop)",
                    log_path.display()
                );
            } else {
                eprintln!(
                    "Tailing {} (INFO+, use -v for DEBUG, Ctrl+C to stop)",
                    log_path.display()
                );
            }

            // Ensure the file exists
            if !log_path.exists() {
                if let Some(parent) = log_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::File::create(&log_path);
            }

            let status = if *verbose {
                std::process::Command::new("tail")
                    .args(["-f", "-n", "50"])
                    .arg(&log_path)
                    .status()
            } else {
                std::process::Command::new("bash")
                    .arg("-c")
                    .arg(format!(
                        "tail -f -n 50 '{}' | grep --line-buffered -v '\\[DEBUG\\]'",
                        log_path.display()
                    ))
                    .status()
            };

            // Auto-disable debug logging if we enabled it
            if we_enabled {
                let _ = send_command(IpcCommand::Debug { enable: false }).await;
                eprintln!("Debug logging disabled");
            }

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

    let mut args_override = args.clone();
    if let ParsedCommand::List { json: true, .. } = command {
        args_override.json_output = true;
    }

    match command {
        ParsedCommand::Restart => match send_command(ipc_command).await {
            Ok(IpcResponse::OkMessage { message }) => {
                if !args.quiet {
                    println!("{}", message);
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
                if !args.quiet {
                    println!("Daemon restarted successfully");
                }
                0
            }
            Ok(IpcResponse::Ok) => {
                if !args.quiet {
                    println!("Daemon restarting...");
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
                if !args.quiet {
                    println!("Daemon restarted successfully");
                }
                0
            }
            Ok(IpcResponse::Error { message }) => {
                eprintln!("Error: {}", message);
                1
            }
            Ok(other) => {
                eprintln!("Unexpected response: {:?}", other);
                1
            }
            Err(e) => {
                match e {
                    ClientError::DaemonNotRunning => {
                        eprintln!("Error: Shroud daemon is not running.");
                        eprintln!("Start it with: shroud");
                    }
                    _ => {
                        eprintln!("Error: {}", e);
                    }
                }
                1
            }
        },
        _ => match send_command(ipc_command).await {
            Ok(response) => handle_response(response, &args_override),
            Err(e) => {
                match e {
                    ClientError::DaemonNotRunning => {
                        eprintln!("Error: Shroud daemon is not running.");
                        eprintln!("Start it with: shroud");
                        // Special exit code for daemon not running?
                        // Standard practice is 1, but maybe another is better. Sticking to 1.
                    }
                    _ => {
                        eprintln!("Error: {}", e);
                    }
                }
                1
            }
        },
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
        ParsedCommand::List { vpn_type, .. } => Some(IpcCommand::List {
            vpn_type: vpn_type.clone(),
        }),

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

        ParsedCommand::Autostart { .. } => None,
        ParsedCommand::Cleanup => None,

        ParsedCommand::Debug { action } => match action {
            DebugAction::On => Some(IpcCommand::Debug { enable: true }),
            DebugAction::Off => Some(IpcCommand::Debug { enable: false }),
            DebugAction::Dump => Some(IpcCommand::DebugDump),
            DebugAction::LogPath => Some(IpcCommand::DebugLogPath),
            DebugAction::Tail { .. } => None, // Handled locally
        },

        ParsedCommand::Ping => Some(IpcCommand::Ping),
        ParsedCommand::Refresh => Some(IpcCommand::Refresh),
        ParsedCommand::Quit => Some(IpcCommand::Quit),
        ParsedCommand::Restart => Some(IpcCommand::Restart),
        ParsedCommand::Reload => Some(IpcCommand::Reload),
        ParsedCommand::Version { .. } => None,
        ParsedCommand::Update => None,
        ParsedCommand::Doctor => None,
        ParsedCommand::VerifyKillswitch { .. } => None,
        ParsedCommand::Gateway { .. } => None,
        ParsedCommand::Import { .. } => None,

        ParsedCommand::Help { .. } => None, // Handled locally
    }
}

fn handle_doctor_command() -> i32 {
    use crate::killswitch::paths;
    use crate::killswitch::sudo_check::{check_sudo_access, SudoAccessStatus};

    println!("🔍 Shroud Doctor - Checking configuration...\n");

    let mut issues = 0;

    println!("=== Firewall Binaries ===");
    let ipt = paths::iptables();
    let ip6 = paths::ip6tables();
    let nft = paths::nft();

    if std::path::Path::new(ipt).exists() {
        println!("  ✓ iptables:  {}", ipt);
    } else {
        println!("  ✗ iptables:  {} (NOT FOUND)", ipt);
        issues += 1;
    }

    if std::path::Path::new(ip6).exists() {
        println!("  ✓ ip6tables: {}", ip6);
    } else {
        println!("  ✗ ip6tables: {} (NOT FOUND)", ip6);
        issues += 1;
    }

    if std::path::Path::new(nft).exists() {
        println!("  ✓ nft:       {}", nft);
    } else {
        println!("  ⚠ nft:       {} (not found, optional)", nft);
    }

    println!();

    println!("=== Sudo Access ===");
    match check_sudo_access() {
        SudoAccessStatus::Ok => {
            println!("  ✓ Passwordless sudo configured correctly");
        }
        SudoAccessStatus::RequiresPassword => {
            println!("  ✗ Sudo requires password for iptables");
            println!("\n    Fix with: ./setup.sh --install-sudoers");
            issues += 1;
        }
        SudoAccessStatus::SudoNotFound => {
            println!("  ✗ sudo command not found");
            issues += 1;
        }
        SudoAccessStatus::BinaryNotFound(path) => {
            println!("  ✗ Binary not found: {}", path);
            issues += 1;
        }
    }

    println!();

    println!("=== Sudoers File ===");
    let sudoers_path = "/etc/sudoers.d/shroud";
    let sudoers_exists = std::process::Command::new("sudo")
        .args(["-n", "test", "-f", sudoers_path])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if sudoers_exists {
        println!("  ✓ {} exists", sudoers_path);
    } else {
        println!("  ✗ {} not found", sudoers_path);
        println!("    Run: ./setup.sh --install-sudoers");
        issues += 1;
    }

    println!();

    println!("=== User Groups ===");
    if let Ok(output) = std::process::Command::new("groups").output() {
        let groups = String::from_utf8_lossy(&output.stdout);
        let in_wheel = groups.contains("wheel");
        let in_sudo = groups.contains("sudo");

        if in_wheel {
            println!("  ✓ User is in 'wheel' group");
        }
        if in_sudo {
            println!("  ✓ User is in 'sudo' group");
        }
        if !in_wheel && !in_sudo {
            println!("  ✗ User is not in 'wheel' or 'sudo' group");
            println!("    Add yourself with: sudo usermod -aG wheel $USER");
            issues += 1;
        }
    }

    println!();

    println!("=== Summary ===");
    if issues == 0 {
        println!("  ✓ All checks passed! Kill switch should work correctly.");
        0
    } else {
        println!("  ✗ Found {} issue(s) that need attention.", issues);
        println!("\n  Quick fix: ./setup.sh --install-sudoers");
        1
    }
}

async fn handle_verify_killswitch_command(json: bool, verbose: bool) -> i32 {
    match verify::run_verification(verbose).await {
        Ok(report) => {
            if json {
                match serde_json::to_string_pretty(&report) {
                    Ok(body) => println!("{}", body),
                    Err(e) => {
                        eprintln!("Failed to serialize report: {}", e);
                        return 2;
                    }
                }
            } else {
                print_human_verification_report(&report, verbose);
            }

            match report.overall {
                verify::Verdict::Pass | verify::Verdict::Warn => 0,
                verify::Verdict::Fail => 1,
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            2
        }
    }
}

fn print_human_verification_report(report: &verify::VerificationReport, verbose: bool) {
    const GREEN: &str = "\x1b[32m";
    const RED: &str = "\x1b[31m";
    const YELLOW: &str = "\x1b[33m";
    const RESET: &str = "\x1b[0m";

    println!("=== Kill Switch Verification ===\n");
    println!("Backend: {}\n", report.backend);

    for check in &report.checks {
        let (sym, color) = match check.verdict {
            verify::Verdict::Pass => ("✓", GREEN),
            verify::Verdict::Warn => ("⚠", YELLOW),
            verify::Verdict::Fail => ("✗", RED),
        };
        println!(
            "{color}{sym}{RESET} {:<45} {}",
            check.description,
            check.detail,
            color = color,
            sym = sym,
            RESET = RESET
        );
        if verbose {
            if let Some(raw) = &check.raw {
                for line in raw.lines() {
                    println!("│ {}", line);
                }
            }
        }
    }

    println!(
        "\nResult: {}{}{} ({})",
        match report.overall {
            verify::Verdict::Pass => GREEN,
            verify::Verdict::Warn => YELLOW,
            verify::Verdict::Fail => RED,
        },
        match report.overall {
            verify::Verdict::Pass => "PASS",
            verify::Verdict::Warn => "WARN",
            verify::Verdict::Fail => "FAIL",
        },
        RESET,
        report.summary
    );

    // Friendly tip if kill switch appears off
    let chain_missing = report
        .checks
        .iter()
        .find(|c| c.name == "chain_exists")
        .map_or(false, |c| matches!(c.verdict, verify::Verdict::Fail));
    let jump_missing = report
        .checks
        .iter()
        .find(|c| c.name == "jump_rule_exists")
        .map_or(false, |c| matches!(c.verdict, verify::Verdict::Fail));

    match report.overall {
        verify::Verdict::Pass => println!("\nYour kill switch is working. Non-VPN traffic is blocked."),
        verify::Verdict::Warn => println!("\nKill switch protections are in place, but there are warnings."),
        verify::Verdict::Fail => println!("\n⚠ WARNING: Your kill switch is NOT protecting you.\nTraffic may be leaking outside the VPN tunnel.\n\nTo fix: shroud killswitch on"),
    }

    if chain_missing && jump_missing {
        println!("\n💡 Tip: Kill switch appears OFF. Enable it with: shroud killswitch on");
    }
}

/// Handle gateway commands.
async fn handle_gateway_command(action: GatewayAction) -> i32 {
    use crate::config::ConfigManager;
    use crate::gateway;
    use crate::gateway::status::GatewayStatus;

    match action {
        GatewayAction::On => {
            // Load config
            let config = ConfigManager::new().load_validated();

            // Check if VPN is connected
            let vpn_interface = gateway::detect::detect_vpn_interface();
            if vpn_interface.is_none() {
                eprintln!("Error: VPN not connected. Connect to VPN first, then enable gateway.");
                eprintln!();
                eprintln!("  shroud connect <server-name>");
                eprintln!("  shroud gateway on");
                return 1;
            }

            println!("Enabling VPN gateway...");

            match gateway::enable(&config.gateway).await {
                Ok(()) => {
                    println!();
                    println!("✓ Gateway enabled");
                    println!();

                    let status = GatewayStatus::collect();
                    if let Some(ref lan) = status.lan_interface {
                        if let Some(ref ip) = status.lan_ip {
                            println!("  LAN interface: {} ({})", lan, ip);
                        }
                    }
                    if let Some(ref vpn) = status.vpn_interface {
                        if let Some(ref ip) = status.vpn_ip {
                            println!("  VPN interface: {} ({})", vpn, ip);
                        }
                    }
                    println!();
                    println!("Clients can now route traffic through this gateway.");
                    println!("Set their default gateway to this machine's LAN IP.");

                    0
                }
                Err(e) => {
                    eprintln!("Error: Failed to enable gateway: {}", e);
                    1
                }
            }
        }
        GatewayAction::Off => {
            if !gateway::is_enabled() {
                println!("Gateway is not enabled.");
                return 0;
            }

            println!("Disabling VPN gateway...");

            match gateway::disable().await {
                Ok(()) => {
                    println!("✓ Gateway disabled");
                    0
                }
                Err(e) => {
                    eprintln!("Error: Failed to disable gateway: {}", e);
                    1
                }
            }
        }
        GatewayAction::Status => {
            let status = GatewayStatus::collect();
            println!("{}", status);
            0
        }
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
        IpcResponse::Status {
            connected,
            vpn_name,
            vpn_type,
            state,
            kill_switch_enabled,
        } => {
            println!("Status: {}", state);
            if connected {
                println!(
                    "Connected to: {}",
                    vpn_name.unwrap_or_else(|| "unknown".to_string())
                );
                if let Some(vpn_type) = vpn_type {
                    println!("Type: {}", vpn_type);
                }
            } else {
                println!("Not connected");
            }
            println!(
                "Kill switch: {}",
                if kill_switch_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            0
        }
        IpcResponse::Connections { connections } => {
            println!("Available VPN connections:");
            println!("  {:<20} {:<10} {:<10}", "NAME", "TYPE", "STATUS");
            for entry in connections {
                println!(
                    "  {:<20} {:<10} {:<10}",
                    entry.name, entry.vpn_type, entry.status
                );
            }
            0
        }
        IpcResponse::OkMessage { message } => {
            println!("{}", message);
            0
        }
        IpcResponse::KillSwitchStatus { enabled } => {
            println!(
                "Kill Switch: {}",
                if enabled { "enabled" } else { "disabled" }
            );
            0
        }
        IpcResponse::AutoReconnectStatus { enabled } => {
            println!(
                "Auto-Reconnect: {}",
                if enabled { "enabled" } else { "disabled" }
            );
            0
        }
        IpcResponse::DebugInfo {
            log_path,
            debug_enabled,
        } => {
            println!("Debug Mode: {}", if debug_enabled { "on" } else { "off" });
            if let Some(path) = log_path {
                println!("Log Path: {}", path);
            }
            0
        }
        IpcResponse::Pong => {
            println!("Pong");
            0
        }
    }
}

fn handle_autostart_command(action: ToggleAction, args: &Args) -> i32 {
    use crate::autostart::Autostart;

    match action {
        ToggleAction::On => {
            if let Ok(Some(path)) = Autostart::cleanup_old_systemd() {
                println!("Cleaned up old systemd service: {}", path);
            }

            match Autostart::enable() {
                Ok(()) => {
                    let status = Autostart::status();
                    println!("✓ Autostart enabled");
                    if let Some(ref path) = status.binary_path {
                        println!("  Binary: {}", path.display());
                    }
                    if let Some(ref path) = status.desktop_file {
                        println!("  Desktop file: {}", path.display());
                    }
                    println!("\nShroud will start automatically on next login.");
                    0
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    1
                }
            }
        }
        ToggleAction::Off => match Autostart::disable() {
            Ok(()) => {
                println!("✓ Autostart disabled");
                0
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                1
            }
        },
        ToggleAction::Toggle => match Autostart::toggle() {
            Ok(true) => {
                println!("✓ Autostart enabled");
                0
            }
            Ok(false) => {
                println!("✓ Autostart disabled");
                0
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                1
            }
        },
        ToggleAction::Status => {
            let status = Autostart::status();

            if args.json_output {
                println!(
                    r#"{{"enabled": {}, "binary_exists": {}, "has_old_systemd": {}}}"#,
                    status.enabled, status.binary_exists, status.has_old_systemd
                );
                return 0;
            }

            println!(
                "Autostart: {}",
                if status.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );

            if let Some(ref path) = status.binary_path {
                println!(
                    "Binary: {} {}",
                    path.display(),
                    if status.binary_exists {
                        "✓"
                    } else {
                        "✗ NOT FOUND"
                    }
                );
            }

            if status.enabled {
                if let Some(ref path) = status.desktop_file {
                    println!("Desktop file: {}", path.display());
                }
            }

            if status.has_old_systemd {
                println!();
                println!("⚠ Warning: Old systemd service found");
                if let Some(ref path) = status.systemd_service_path {
                    println!("  {}", path.display());
                }
                println!("  Run 'shroud cleanup' to remove it");
            }
            0
        }
    }
}

async fn handle_cleanup_command(args: &Args) -> i32 {
    use crate::autostart::Autostart;

    if !args.quiet {
        println!("Cleaning up old configurations...\n");
    }

    let mut cleaned = false;

    match Autostart::cleanup_old_systemd() {
        Ok(Some(path)) => {
            println!("✓ Removed old systemd service: {}", path);
            cleaned = true;
        }
        Ok(None) => {
            if !args.quiet {
                println!("  No old systemd service found");
            }
        }
        Err(e) => {
            eprintln!("✗ Failed to clean systemd service: {}", e);
        }
    }

    if let Some(runtime) = dirs::runtime_dir() {
        let socket = runtime.join("shroud.sock");
        let lock = runtime.join("shroud.lock");

        if !is_daemon_running().await {
            if socket.exists() && std::fs::remove_file(&socket).is_ok() {
                println!("✓ Removed stale socket: {}", socket.display());
                cleaned = true;
            }
            if lock.exists() && std::fs::remove_file(&lock).is_ok() {
                println!("✓ Removed stale lock: {}", lock.display());
                cleaned = true;
            }
        }
    }

    if !cleaned {
        if !args.quiet {
            println!("\nNothing to clean up.");
        }
    } else if !args.quiet {
        println!("\n✓ Cleanup complete");
    }

    0
}

async fn is_daemon_running() -> bool {
    send_command(IpcCommand::Ping).await.is_ok()
}

async fn handle_version_command(check: bool) -> i32 {
    let version = env!("CARGO_PKG_VERSION");
    println!("shroud {}", version);

    if check {
        // Quick staleness check: compare binary mtime vs Cargo.toml + src/main.rs
        if let Ok(exe_path) = std::env::current_exe() {
            if let Ok(exe_meta) = std::fs::metadata(&exe_path) {
                if let Ok(exe_mtime) = exe_meta.modified() {
                    let check_files = ["Cargo.toml", "src/main.rs", "Cargo.lock"];
                    let mut newer = false;
                    for file in &check_files {
                        let path = std::path::Path::new(file);
                        if let Ok(meta) = std::fs::metadata(path) {
                            if let Ok(mtime) = meta.modified() {
                                if mtime > exe_mtime {
                                    newer = true;
                                    break;
                                }
                            }
                        }
                    }
                    if newer {
                        println!("\n⚠ Source may be newer than binary");
                        println!("  Run 'shroud update' to rebuild and install");
                    } else {
                        println!("\n✓ Binary appears up to date");
                    }
                }
            }
        }
    }

    if let Ok(response) = send_command(IpcCommand::Ping).await {
        if let IpcResponse::Pong | IpcResponse::Ok = response {
            println!("Daemon is running");
        }
    } else {
        println!("Daemon is not running");
    }

    0
}

fn handle_update_command() -> i32 {
    // Locate the update script relative to the binary or project
    let script_candidates = [
        // From project directory (most common during dev)
        std::path::PathBuf::from("scripts/update.sh"),
        // Relative to binary location
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("../scripts/update.sh")))
            .unwrap_or_default(),
        // Home directory project
        dirs::home_dir()
            .map(|h| h.join("src/shroud/scripts/update.sh"))
            .unwrap_or_default(),
    ];

    for candidate in &script_candidates {
        if candidate.exists() {
            println!("Running {}...\n", candidate.display());
            let status = std::process::Command::new("bash").arg(candidate).status();

            return match status {
                Ok(s) => s.code().unwrap_or(1),
                Err(e) => {
                    eprintln!("Failed to run update script: {}", e);
                    1
                }
            };
        }
    }

    // Fallback: run the commands inline
    eprintln!("Update script not found, running inline...\n");
    let status = std::process::Command::new("bash")
        .arg("-c")
        .arg(concat!(
            "set -e && ",
            "echo 'Building and installing...' && ",
            "cargo install --path . --force && ",
            "rm -f ~/.local/bin/shroud 2>/dev/null || true && ",
            "cp ~/.cargo/bin/shroud ~/.local/bin/shroud && ",
            "echo 'Restarting daemon...' && ",
            "shroud restart 2>/dev/null || echo 'Daemon not running' && ",
            "echo '' && shroud --version && echo '✓ Update complete'"
        ))
        .status();

    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("Failed to run update: {}", e);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::{Args, ToggleAction};
    use crate::cli::help;

    fn default_args() -> Args {
        Args::default()
    }

    fn args_with_json() -> Args {
        Args {
            json_output: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_handle_version_returns_zero() {
        let exit_code = handle_version_command(false).await;
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn test_help_main() {
        help::print_main_help();
    }

    #[test]
    fn test_help_connect() {
        help::print_command_help("connect");
    }

    #[test]
    fn test_help_invalid_command() {
        help::print_command_help("nonexistent");
    }

    #[test]
    fn test_handle_autostart_status() {
        let args = default_args();
        let exit_code = handle_autostart_command(ToggleAction::Status, &args);
        assert_eq!(exit_code, 0);
    }

    #[test]
    #[ignore = "requires XDG desktop environment - run with: cargo test -- --ignored"]
    fn test_handle_autostart_on_off() {
        let args = default_args();
        let exit_code = handle_autostart_command(ToggleAction::On, &args);
        assert_eq!(exit_code, 0);

        let exit_code = handle_autostart_command(ToggleAction::Off, &args);
        assert_eq!(exit_code, 0);
    }

    #[test]
    #[ignore = "requires XDG desktop environment - run with: cargo test -- --ignored"]
    fn test_handle_autostart_toggle() {
        let args = default_args();
        let initial = crate::autostart::Autostart::is_enabled();

        let exit_code = handle_autostart_command(ToggleAction::Toggle, &args);
        assert_eq!(exit_code, 0);

        assert_ne!(crate::autostart::Autostart::is_enabled(), initial);

        let _ = handle_autostart_command(ToggleAction::Toggle, &args);
    }

    #[test]
    fn test_handle_autostart_json_output() {
        let args = args_with_json();
        let exit_code = handle_autostart_command(ToggleAction::Status, &args);
        assert_eq!(exit_code, 0);
    }

    #[tokio::test]
    async fn test_handle_cleanup_returns_zero() {
        let args = default_args();
        let exit_code = handle_cleanup_command(&args).await;
        assert_eq!(exit_code, 0);
    }

    #[tokio::test]
    async fn test_is_daemon_running_returns_bool() {
        let _ = is_daemon_running().await;
    }

    // --- args_to_command mapping ---

    mod args_to_command_tests {
        use super::*;
        use crate::cli::args::*;

        #[test]
        fn test_connect() {
            let cmd = ParsedCommand::Connect {
                name: "vpn1".into(),
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert!(matches!(ipc, IpcCommand::Connect { name } if name == "vpn1"));
        }

        #[test]
        fn test_disconnect() {
            let ipc = args_to_command(&ParsedCommand::Disconnect).unwrap();
            assert_eq!(ipc, IpcCommand::Disconnect);
        }

        #[test]
        fn test_reconnect() {
            let ipc = args_to_command(&ParsedCommand::Reconnect).unwrap();
            assert_eq!(ipc, IpcCommand::Reconnect);
        }

        #[test]
        fn test_switch() {
            let cmd = ParsedCommand::Switch {
                name: "vpn2".into(),
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert!(matches!(ipc, IpcCommand::Switch { name } if name == "vpn2"));
        }

        #[test]
        fn test_status() {
            let ipc = args_to_command(&ParsedCommand::Status).unwrap();
            assert_eq!(ipc, IpcCommand::Status);
        }

        #[test]
        fn test_list_no_filter() {
            let cmd = ParsedCommand::List {
                vpn_type: None,
                json: false,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert!(matches!(ipc, IpcCommand::List { vpn_type: None }));
        }

        #[test]
        fn test_list_with_filter() {
            let cmd = ParsedCommand::List {
                vpn_type: Some("wireguard".into()),
                json: false,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert!(matches!(
                ipc,
                IpcCommand::List {
                    vpn_type: Some(t)
                } if t == "wireguard"
            ));
        }

        #[test]
        fn test_killswitch_on() {
            let cmd = ParsedCommand::KillSwitch {
                action: ToggleAction::On,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::KillSwitch { enable: true });
        }

        #[test]
        fn test_killswitch_off() {
            let cmd = ParsedCommand::KillSwitch {
                action: ToggleAction::Off,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::KillSwitch { enable: false });
        }

        #[test]
        fn test_killswitch_toggle() {
            let cmd = ParsedCommand::KillSwitch {
                action: ToggleAction::Toggle,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::KillSwitchToggle);
        }

        #[test]
        fn test_killswitch_status() {
            let cmd = ParsedCommand::KillSwitch {
                action: ToggleAction::Status,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::KillSwitchStatus);
        }

        #[test]
        fn test_auto_reconnect_on() {
            let cmd = ParsedCommand::AutoReconnect {
                action: ToggleAction::On,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::AutoReconnect { enable: true });
        }

        #[test]
        fn test_auto_reconnect_off() {
            let cmd = ParsedCommand::AutoReconnect {
                action: ToggleAction::Off,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::AutoReconnect { enable: false });
        }

        #[test]
        fn test_auto_reconnect_toggle() {
            let cmd = ParsedCommand::AutoReconnect {
                action: ToggleAction::Toggle,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::AutoReconnectToggle);
        }

        #[test]
        fn test_auto_reconnect_status() {
            let cmd = ParsedCommand::AutoReconnect {
                action: ToggleAction::Status,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::AutoReconnectStatus);
        }

        #[test]
        fn test_debug_on() {
            let cmd = ParsedCommand::Debug {
                action: DebugAction::On,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::Debug { enable: true });
        }

        #[test]
        fn test_debug_off() {
            let cmd = ParsedCommand::Debug {
                action: DebugAction::Off,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::Debug { enable: false });
        }

        #[test]
        fn test_debug_dump() {
            let cmd = ParsedCommand::Debug {
                action: DebugAction::Dump,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::DebugDump);
        }

        #[test]
        fn test_debug_log_path() {
            let cmd = ParsedCommand::Debug {
                action: DebugAction::LogPath,
            };
            let ipc = args_to_command(&cmd).unwrap();
            assert_eq!(ipc, IpcCommand::DebugLogPath);
        }

        #[test]
        fn test_debug_tail_is_local() {
            let cmd = ParsedCommand::Debug {
                action: DebugAction::Tail { verbose: false },
            };
            assert!(args_to_command(&cmd).is_none());
        }

        #[test]
        fn test_ping() {
            assert_eq!(
                args_to_command(&ParsedCommand::Ping).unwrap(),
                IpcCommand::Ping
            );
        }

        #[test]
        fn test_refresh() {
            assert_eq!(
                args_to_command(&ParsedCommand::Refresh).unwrap(),
                IpcCommand::Refresh
            );
        }

        #[test]
        fn test_quit() {
            assert_eq!(
                args_to_command(&ParsedCommand::Quit).unwrap(),
                IpcCommand::Quit
            );
        }

        #[test]
        fn test_restart() {
            assert_eq!(
                args_to_command(&ParsedCommand::Restart).unwrap(),
                IpcCommand::Restart
            );
        }

        #[test]
        fn test_reload() {
            assert_eq!(
                args_to_command(&ParsedCommand::Reload).unwrap(),
                IpcCommand::Reload
            );
        }

        #[test]
        fn test_local_commands_return_none() {
            assert!(args_to_command(&ParsedCommand::Autostart {
                action: ToggleAction::On
            })
            .is_none());
            assert!(args_to_command(&ParsedCommand::Cleanup).is_none());
            assert!(args_to_command(&ParsedCommand::Version { check: false }).is_none());
            assert!(args_to_command(&ParsedCommand::Update).is_none());
            assert!(args_to_command(&ParsedCommand::Doctor).is_none());
            assert!(args_to_command(&ParsedCommand::Gateway {
                action: GatewayAction::Status
            })
            .is_none());
            assert!(args_to_command(&ParsedCommand::Help { command: None }).is_none());
        }
    }

    // --- handle_response formatting ---

    mod handle_response_tests {
        use super::*;

        #[test]
        fn test_ok_response() {
            let args = default_args();
            assert_eq!(handle_response(IpcResponse::Ok, &args), 0);
        }

        #[test]
        fn test_error_response() {
            let args = default_args();
            let resp = IpcResponse::Error {
                message: "fail".into(),
            };
            assert_eq!(handle_response(resp, &args), 1);
        }

        #[test]
        fn test_pong_response() {
            let args = default_args();
            assert_eq!(handle_response(IpcResponse::Pong, &args), 0);
        }

        #[test]
        fn test_status_response_disconnected() {
            let args = default_args();
            let resp = IpcResponse::Status {
                connected: false,
                vpn_name: None,
                vpn_type: None,
                state: "Disconnected".into(),
                kill_switch_enabled: false,
            };
            assert_eq!(handle_response(resp, &args), 0);
        }

        #[test]
        fn test_status_response_connected() {
            let args = default_args();
            let resp = IpcResponse::Status {
                connected: true,
                vpn_name: Some("my-vpn".into()),
                vpn_type: Some("wireguard".into()),
                state: "Connected".into(),
                kill_switch_enabled: true,
            };
            assert_eq!(handle_response(resp, &args), 0);
        }

        #[test]
        fn test_connections_response() {
            let args = default_args();
            let resp = IpcResponse::Connections {
                connections: vec![
                    crate::ipc::protocol::VpnConnectionInfo {
                        name: "vpn1".into(),
                        vpn_type: "wireguard".into(),
                        status: "active".into(),
                    },
                    crate::ipc::protocol::VpnConnectionInfo {
                        name: "vpn2".into(),
                        vpn_type: "openvpn".into(),
                        status: "available".into(),
                    },
                ],
            };
            assert_eq!(handle_response(resp, &args), 0);
        }

        #[test]
        fn test_ok_message_response() {
            let args = default_args();
            let resp = IpcResponse::OkMessage {
                message: "done".into(),
            };
            assert_eq!(handle_response(resp, &args), 0);
        }

        #[test]
        fn test_ks_status_response() {
            let args = default_args();
            assert_eq!(
                handle_response(IpcResponse::KillSwitchStatus { enabled: true }, &args),
                0
            );
            assert_eq!(
                handle_response(IpcResponse::KillSwitchStatus { enabled: false }, &args),
                0
            );
        }

        #[test]
        fn test_ar_status_response() {
            let args = default_args();
            assert_eq!(
                handle_response(IpcResponse::AutoReconnectStatus { enabled: true }, &args),
                0
            );
        }

        #[test]
        fn test_debug_info_response() {
            let args = default_args();
            let resp = IpcResponse::DebugInfo {
                log_path: Some("/tmp/debug.log".into()),
                debug_enabled: true,
            };
            assert_eq!(handle_response(resp, &args), 0);
        }

        #[test]
        fn test_json_output_ok() {
            let args = args_with_json();
            assert_eq!(handle_response(IpcResponse::Ok, &args), 0);
        }

        #[test]
        fn test_json_output_error() {
            let args = args_with_json();
            let resp = IpcResponse::Error {
                message: "fail".into(),
            };
            assert_eq!(handle_response(resp, &args), 1);
        }

        #[test]
        fn test_quiet_ok() {
            let args = Args {
                quiet: true,
                ..Default::default()
            };
            assert_eq!(handle_response(IpcResponse::Ok, &args), 0);
        }
    }
}
