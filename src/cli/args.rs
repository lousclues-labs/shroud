//! Command-line argument parsing
//!
//! Lightweight argument parsing without external dependencies.
//! Supports both global options and subcommands for daemon vs client mode.

use std::path::PathBuf;

/// Parsed command-line arguments
#[derive(Debug, Clone)]
pub struct Args {
    // Global options
    /// Verbosity level (0=warn, 1=info, 2=debug, 3+=trace)
    pub verbose: u8,
    /// Explicit log level override
    pub log_level: Option<String>,
    /// Log to file
    pub log_file: Option<PathBuf>,
    /// Output in JSON format
    pub json_output: bool,
    /// Suppress output (exit code only)
    pub quiet: bool,
    /// Timeout for daemon communication in seconds
    pub timeout: u64,

    /// Command to execute (None = daemon mode)
    pub command: Option<ParsedCommand>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            verbose: 0,
            log_level: None,
            log_file: None,
            json_output: false,
            quiet: false,
            timeout: 5,
            command: None,
        }
    }
}

/// Parsed CLI command
#[derive(Debug, Clone)]
pub enum ParsedCommand {
    // Connection management
    Connect { name: String },
    Disconnect,
    Reconnect,
    Switch { name: String },

    // Status and information
    Status,
    List,

    // Kill switch
    KillSwitch { action: ToggleAction },

    // Auto-reconnect
    AutoReconnect { action: ToggleAction },

    // Autostart
    Autostart { action: ToggleAction },

    // Cleanup
    Cleanup,

    // Debug
    Debug { action: DebugAction },

    // Daemon control
    Ping,
    Refresh,
    Quit,
    Restart,
    Reload,
    Update { yes: bool, debug: bool },
    Version { check: bool },

    // Help
    Help { command: Option<String> },
}

/// Action for toggle commands (killswitch, auto-reconnect)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleAction {
    On,
    Off,
    Toggle,
    Status,
}

/// Action for debug commands
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugAction {
    On,
    Off,
    LogPath,
    Tail,
    Dump,
}

/// Parse command-line arguments
pub fn parse_args() -> Result<Args, String> {
    parse_args_from(&std::env::args().skip(1).collect::<Vec<_>>())
}

/// Parse arguments from a string slice (for testing)
pub fn parse_args_from(argv: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut i = 0;

    // Parse global options first
    while i < argv.len() {
        match argv[i].as_str() {
            "-v" | "--verbose" => args.verbose = args.verbose.saturating_add(1),
            "-vv" => args.verbose = args.verbose.saturating_add(2),
            "-vvv" => args.verbose = args.verbose.saturating_add(3),
            "--log-level" => {
                i += 1;
                args.log_level = Some(argv.get(i).ok_or("--log-level requires a value")?.clone());
            }
            "--log-file" => {
                i += 1;
                args.log_file = Some(PathBuf::from(
                    argv.get(i).ok_or("--log-file requires a value")?,
                ));
            }
            "--json" => args.json_output = true,
            "-q" | "--quiet" => args.quiet = true,
            "--timeout" => {
                i += 1;
                args.timeout = argv
                    .get(i)
                    .ok_or("--timeout requires a value")?
                    .parse()
                    .map_err(|_| "Invalid timeout value")?;
            }
            "-h" | "--help" => {
                // --help with no command shows main help
                args.command = Some(ParsedCommand::Help { command: None });
                return Ok(args);
            }
            "-V" | "--version" => {
                println!("shroud {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            arg if arg.starts_with('-') && !arg.starts_with("--") => {
                // Handle combined flags like -vvv
                for c in arg.chars().skip(1) {
                    match c {
                        'v' => args.verbose = args.verbose.saturating_add(1),
                        'q' => args.quiet = true,
                        'h' => {
                            args.command = Some(ParsedCommand::Help { command: None });
                            return Ok(args);
                        }
                        'V' => {
                            println!("shroud {}", env!("CARGO_PKG_VERSION"));
                            std::process::exit(0);
                        }
                        _ => return Err(format!("Unknown flag: -{}", c)),
                    }
                }
            }
            arg if arg.starts_with("--") => {
                return Err(format!("Unknown option: {}", arg));
            }
            _ => break, // Start of command
        }
        i += 1;
    }

    // Parse command
    if i < argv.len() {
        args.command = Some(parse_command(&argv[i..])?);
    }

    Ok(args)
}

/// Parse a command and its arguments
fn parse_command(argv: &[String]) -> Result<ParsedCommand, String> {
    if argv.is_empty() {
        return Err("No command provided".to_string());
    }

    // Check for --help after command
    if argv.len() > 1 && (argv[1] == "--help" || argv[1] == "-h") {
        return Ok(ParsedCommand::Help {
            command: Some(argv[0].clone()),
        });
    }

    match argv[0].as_str() {
        "connect" => {
            let name = argv.get(1).ok_or("connect requires a connection name")?;
            Ok(ParsedCommand::Connect { name: name.clone() })
        }
        "disconnect" => Ok(ParsedCommand::Disconnect),
        "reconnect" => Ok(ParsedCommand::Reconnect),
        "switch" => {
            let name = argv.get(1).ok_or("switch requires a connection name")?;
            Ok(ParsedCommand::Switch { name: name.clone() })
        }
        "status" => Ok(ParsedCommand::Status),
        "list" | "ls" => Ok(ParsedCommand::List),
        "killswitch" | "kill-switch" | "ks" => {
            let action = parse_toggle_action(argv.get(1).map(|s| s.as_str()))?;
            Ok(ParsedCommand::KillSwitch { action })
        }
        "auto-reconnect" | "autoreconnect" | "ar" => {
            let action = parse_toggle_action(argv.get(1).map(|s| s.as_str()))?;
            Ok(ParsedCommand::AutoReconnect { action })
        }
        "debug" => {
            let action = parse_debug_action(argv.get(1).map(|s| s.as_str()))?;
            Ok(ParsedCommand::Debug { action })
        }
        "autostart" | "startup" => {
            let action = parse_toggle_action(argv.get(1).map(|s| s.as_str()))?;
            Ok(ParsedCommand::Autostart { action })
        }
        "cleanup" => Ok(ParsedCommand::Cleanup),
        "ping" => Ok(ParsedCommand::Ping),
        "refresh" => Ok(ParsedCommand::Refresh),
        "quit" | "stop" | "exit" => Ok(ParsedCommand::Quit),
        "restart" => Ok(ParsedCommand::Restart),
        "reload" => Ok(ParsedCommand::Reload),
        "update" => parse_update_flags(&argv[1..]),
        "version" => parse_version_flags(&argv[1..]),
        "help" => Ok(ParsedCommand::Help {
            command: argv.get(1).cloned(),
        }),
        _ => Err(format!(
            "Unknown command: '{}'. Run 'shroud --help' for usage.",
            argv[0]
        )),
    }
}

fn parse_update_flags(argv: &[String]) -> Result<ParsedCommand, String> {
    let mut yes = false;
    let mut debug = false;

    for arg in argv {
        match arg.as_str() {
            "-y" | "--yes" => yes = true,
            "--debug" => debug = true,
            _ => {
                return Err(format!(
                    "Unknown update option: '{}'. Use --yes or --debug",
                    arg
                ))
            }
        }
    }

    Ok(ParsedCommand::Update { yes, debug })
}

fn parse_version_flags(argv: &[String]) -> Result<ParsedCommand, String> {
    let mut check = false;

    for arg in argv {
        match arg.as_str() {
            "--check" => check = true,
            _ => return Err(format!("Unknown version option: '{}'. Use --check", arg)),
        }
    }

    Ok(ParsedCommand::Version { check })
}

/// Parse a toggle action argument
fn parse_toggle_action(arg: Option<&str>) -> Result<ToggleAction, String> {
    match arg {
        None | Some("status") => Ok(ToggleAction::Status),
        Some("on") | Some("enable") | Some("1") | Some("true") => Ok(ToggleAction::On),
        Some("off") | Some("disable") | Some("0") | Some("false") => Ok(ToggleAction::Off),
        Some("toggle") => Ok(ToggleAction::Toggle),
        Some(other) => Err(format!(
            "Unknown action: '{}'. Use on/off/toggle/status",
            other
        )),
    }
}

/// Parse a debug action argument
fn parse_debug_action(arg: Option<&str>) -> Result<DebugAction, String> {
    match arg {
        Some("on") | Some("enable") => Ok(DebugAction::On),
        Some("off") | Some("disable") => Ok(DebugAction::Off),
        Some("log-path") | Some("logpath") | Some("path") => Ok(DebugAction::LogPath),
        Some("tail") | Some("follow") | Some("f") => Ok(DebugAction::Tail),
        Some("dump") | Some("state") => Ok(DebugAction::Dump),
        None => Err("debug requires an action: on/off/log-path/tail/dump".to_string()),
        Some(other) => Err(format!("Unknown debug action: '{}'", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn test_daemon_mode_no_command() {
        let result = parse_args_from(&args("")).unwrap();
        assert!(result.command.is_none());
    }

    #[test]
    fn test_connect_command() {
        let result = parse_args_from(&args("connect ireland-42")).unwrap();
        assert!(matches!(
            result.command,
            Some(ParsedCommand::Connect { name }) if name == "ireland-42"
        ));
    }

    #[test]
    fn test_status_command() {
        let result = parse_args_from(&args("status")).unwrap();
        assert!(matches!(result.command, Some(ParsedCommand::Status)));
    }

    #[test]
    fn test_status_json() {
        let result = parse_args_from(&args("--json status")).unwrap();
        assert!(result.json_output);
        assert!(matches!(result.command, Some(ParsedCommand::Status)));
    }

    #[test]
    fn test_killswitch_toggle() {
        let result = parse_args_from(&args("killswitch toggle")).unwrap();
        assert!(matches!(
            result.command,
            Some(ParsedCommand::KillSwitch {
                action: ToggleAction::Toggle
            })
        ));
    }

    #[test]
    fn test_killswitch_default_status() {
        let result = parse_args_from(&args("killswitch")).unwrap();
        assert!(matches!(
            result.command,
            Some(ParsedCommand::KillSwitch {
                action: ToggleAction::Status
            })
        ));
    }

    #[test]
    fn test_verbose_flags() {
        let result = parse_args_from(&args("-v -v status")).unwrap();
        assert_eq!(result.verbose, 2);
    }

    #[test]
    fn test_combined_verbose_flags() {
        let result = parse_args_from(&args("-vvv status")).unwrap();
        assert_eq!(result.verbose, 3);
    }

    #[test]
    fn test_quiet_flag() {
        let result = parse_args_from(&args("-q connect test")).unwrap();
        assert!(result.quiet);
    }

    #[test]
    fn test_timeout_flag() {
        let result = parse_args_from(&args("--timeout 10 ping")).unwrap();
        assert_eq!(result.timeout, 10);
    }

    #[test]
    fn test_help_command() {
        let result = parse_args_from(&args("help connect")).unwrap();
        assert!(matches!(
            result.command,
            Some(ParsedCommand::Help { command: Some(c) }) if c == "connect"
        ));
    }

    #[test]
    fn test_command_help_flag() {
        let result = parse_args_from(&args("connect --help")).unwrap();
        assert!(matches!(
            result.command,
            Some(ParsedCommand::Help { command: Some(c) }) if c == "connect"
        ));
    }

    #[test]
    fn test_invalid_command() {
        let result = parse_args_from(&args("foobar"));
        assert!(result.is_err());
    }

    #[test]
    fn test_debug_on() {
        let result = parse_args_from(&args("debug on")).unwrap();
        assert!(matches!(
            result.command,
            Some(ParsedCommand::Debug {
                action: DebugAction::On
            })
        ));
    }

    #[test]
    fn test_quit_aliases() {
        for cmd in &["quit", "stop", "exit"] {
            let result = parse_args_from(&[cmd.to_string()]).unwrap();
            assert!(matches!(result.command, Some(ParsedCommand::Quit)));
        }
    }

    #[test]
    fn test_list_alias() {
        let result = parse_args_from(&args("ls")).unwrap();
        assert!(matches!(result.command, Some(ParsedCommand::List)));
    }
}
