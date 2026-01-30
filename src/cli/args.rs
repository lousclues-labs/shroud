//! Command-line argument parsing
//!
//! Lightweight argument parsing without external dependencies.
//! Supports both global options and subcommands for daemon vs client mode.

use std::path::PathBuf;

use super::validation::{
    self, validate_log_level, validate_log_path, validate_timeout, validate_verbosity,
    validate_vpn_name,
};
use crate::import::ImportOptions;
use crate::import::VpnConfigType;

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
            timeout: super::validation::DEFAULT_TIMEOUT_SECS,
            command: None,
        }
    }
}

/// Parsed CLI command
#[derive(Debug, Clone)]
pub enum ParsedCommand {
    // Connection management
    Connect {
        name: String,
    },
    Disconnect,
    Reconnect,
    Switch {
        name: String,
    },

    // Status and information
    Status,
    List {
        vpn_type: Option<String>,
        json: bool,
    },

    // Import configs
    Import {
        options: ImportOptions,
    },

    // Kill switch
    KillSwitch {
        action: ToggleAction,
    },

    // Auto-reconnect
    AutoReconnect {
        action: ToggleAction,
    },

    // Autostart
    Autostart {
        action: ToggleAction,
    },

    // Cleanup
    Cleanup,

    // Debug
    Debug {
        action: DebugAction,
    },

    // Daemon control
    Ping,
    Refresh,
    Quit,
    Restart,
    Reload,
    Update {
        yes: bool,
        debug: bool,
    },
    Version {
        check: bool,
    },

    // Development/maintenance
    Audit,

    // Help
    Help {
        command: Option<String>,
    },
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
            "-v" | "--verbose" => {
                args.verbose = validate_verbosity(args.verbose.saturating_add(1));
            }
            "-vv" => {
                args.verbose = validate_verbosity(args.verbose.saturating_add(2));
            }
            "-vvv" => {
                args.verbose = validate_verbosity(args.verbose.saturating_add(3));
            }
            "--log-level" => {
                i += 1;
                let value = argv.get(i).ok_or("--log-level requires a value")?;
                args.log_level = Some(validate_log_level(value).map_err(|e| e.to_string())?);
            }
            "--log-file" => {
                i += 1;
                let value = argv.get(i).ok_or("--log-file requires a value")?;
                args.log_file = Some(validate_log_path(value).map_err(|e| e.to_string())?);
            }
            "--json" => args.json_output = true,
            "-q" | "--quiet" => args.quiet = true,
            "--timeout" => {
                i += 1;
                let value = argv.get(i).ok_or("--timeout requires a value")?;
                args.timeout = validate_timeout(value).map_err(|e| e.to_string())?;
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
                        'v' => {
                            args.verbose = validate_verbosity(args.verbose.saturating_add(1));
                        }
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
            let validated = validate_vpn_name(name).map_err(|e| e.to_string())?;
            if validation::looks_like_injection(&validated) {
                log::debug!(
                    "VPN name contains unusual characters: {:?}",
                    validation::sanitize_for_display(&validated, 50)
                );
            }
            Ok(ParsedCommand::Connect { name: validated })
        }
        "disconnect" => Ok(ParsedCommand::Disconnect),
        "reconnect" => Ok(ParsedCommand::Reconnect),
        "switch" => {
            let name = argv.get(1).ok_or("switch requires a connection name")?;
            let validated = validate_vpn_name(name).map_err(|e| e.to_string())?;
            Ok(ParsedCommand::Switch { name: validated })
        }
        "status" => Ok(ParsedCommand::Status),
        "list" | "ls" => parse_list_flags(&argv[1..]),
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
        "import" => parse_import_args(&argv[1..]),
        "audit" => Ok(ParsedCommand::Audit),
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

fn parse_list_flags(argv: &[String]) -> Result<ParsedCommand, String> {
    let mut vpn_type: Option<String> = None;
    let mut json = false;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--type" => {
                i += 1;
                let value = argv.get(i).ok_or("--type requires a value")?;
                let normalized = value.to_lowercase();
                match normalized.as_str() {
                    "wireguard" | "openvpn" => {
                        vpn_type = Some(normalized);
                    }
                    "all" => {
                        vpn_type = None;
                    }
                    other => {
                        return Err(format!(
                            "Unknown VPN type: '{}'. Use wireguard, openvpn, or all",
                            other
                        ));
                    }
                }
            }
            "--json" => {
                json = true;
            }
            other => {
                return Err(format!(
                    "Unknown list option: '{}'. Use --type <wireguard|openvpn|all>",
                    other
                ));
            }
        }
        i += 1;
    }

    Ok(ParsedCommand::List { vpn_type, json })
}

fn parse_import_args(argv: &[String]) -> Result<ParsedCommand, String> {
    if argv.is_empty() {
        return Err("import requires a file or directory path".to_string());
    }

    let mut path: Option<PathBuf> = None;
    let mut name: Option<String> = None;
    let mut connect = false;
    let mut force = false;
    let mut recursive = false;
    let mut dry_run = false;
    let mut config_type: Option<VpnConfigType> = None;
    let mut quiet = false;
    let mut json = false;

    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-n" | "--name" => {
                i += 1;
                let value = argv.get(i).ok_or("--name requires a value")?;
                let validated = validate_vpn_name(value).map_err(|e| e.to_string())?;
                name = Some(validated);
            }
            "-c" | "--connect" => {
                connect = true;
            }
            "-f" | "--force" => {
                force = true;
            }
            "-r" | "--recursive" => {
                recursive = true;
            }
            "--dry-run" => {
                dry_run = true;
            }
            "--type" => {
                i += 1;
                let value = argv.get(i).ok_or("--type requires a value")?;
                let normalized = value.to_lowercase();
                config_type = match normalized.as_str() {
                    "wireguard" => Some(VpnConfigType::WireGuard),
                    "openvpn" => Some(VpnConfigType::OpenVpn),
                    other => {
                        return Err(format!(
                            "Unknown import type: '{}'. Use wireguard or openvpn",
                            other
                        ));
                    }
                };
            }
            "-q" | "--quiet" => {
                quiet = true;
            }
            "--json" => {
                json = true;
            }
            value if !value.starts_with('-') => {
                if path.is_some() {
                    return Err("import accepts only one path".to_string());
                }
                path = Some(PathBuf::from(value));
            }
            other => {
                return Err(format!("Unknown import option: '{}'", other));
            }
        }
        i += 1;
    }

    let path = path.ok_or("import requires a file or directory path")?;

    Ok(ParsedCommand::Import {
        options: ImportOptions {
            path,
            name,
            connect,
            force,
            recursive,
            dry_run,
            config_type,
            quiet,
            json,
        },
    })
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
        assert!(matches!(
            result.command,
            Some(ParsedCommand::List {
                vpn_type: None,
                json: false
            })
        ));
    }

    #[test]
    fn test_list_with_type_filter() {
        let result = parse_args_from(&args("list --type wireguard")).unwrap();
        assert!(matches!(
            result.command,
            Some(ParsedCommand::List {
                vpn_type: Some(t),
                json: false
            }) if t == "wireguard"
        ));
    }

    #[test]
    fn test_list_json_flag() {
        let result = parse_args_from(&args("list --json")).unwrap();
        assert!(matches!(
            result.command,
            Some(ParsedCommand::List {
                vpn_type: None,
                json: true
            })
        ));
    }

    #[test]
    fn test_import_command() {
        let result = parse_args_from(&args("import /tmp/vpn.conf --dry-run")).unwrap();
        assert!(matches!(result.command, Some(ParsedCommand::Import { .. })));
    }

    #[test]
    fn test_audit_command() {
        let result = parse_args_from(&args("audit")).unwrap();
        assert!(matches!(result.command, Some(ParsedCommand::Audit)));
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn test_timeout_zero_rejected() {
        let args = vec![
            "--timeout".to_string(),
            "0".to_string(),
            "status".to_string(),
        ];
        let result = parse_args_from(&args);
        assert!(result.is_err(), "Timeout of 0 should be rejected");
        assert!(result.unwrap_err().contains("at least"));
    }

    #[test]
    fn test_timeout_negative_rejected() {
        let args = vec![
            "--timeout".to_string(),
            "-5".to_string(),
            "status".to_string(),
        ];
        let result = parse_args_from(&args);
        assert!(result.is_err(), "Negative timeout should be rejected");
    }

    #[test]
    fn test_timeout_too_large_rejected() {
        let args = vec![
            "--timeout".to_string(),
            "9999999".to_string(),
            "status".to_string(),
        ];
        let result = parse_args_from(&args);
        assert!(result.is_err(), "Huge timeout should be rejected");
        assert!(result.unwrap_err().contains("at most"));
    }

    #[test]
    fn test_timeout_valid_accepted() {
        let args = vec![
            "--timeout".to_string(),
            "30".to_string(),
            "status".to_string(),
        ];
        let result = parse_args_from(&args);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().timeout, 30);
    }

    #[test]
    fn test_timeout_max_accepted() {
        let args = vec![
            "--timeout".to_string(),
            "3600".to_string(),
            "status".to_string(),
        ];
        let result = parse_args_from(&args);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().timeout, 3600);
    }

    #[test]
    fn test_log_level_invalid_rejected() {
        let args = vec![
            "--log-level".to_string(),
            "invalid".to_string(),
            "status".to_string(),
        ];
        let result = parse_args_from(&args);
        assert!(result.is_err(), "Invalid log level should be rejected");
        assert!(result.unwrap_err().contains("must be one of"));
    }

    #[test]
    fn test_log_level_injection_rejected() {
        let args = vec![
            "--log-level".to_string(),
            "debug; rm -rf /".to_string(),
            "status".to_string(),
        ];
        let result = parse_args_from(&args);
        assert!(result.is_err(), "Injection attempt should be rejected");
    }

    #[test]
    fn test_log_level_valid_accepted() {
        for level in &["error", "warn", "info", "debug", "trace"] {
            let args = vec![
                "--log-level".to_string(),
                level.to_string(),
                "status".to_string(),
            ];
            let result = parse_args_from(&args);
            assert!(
                result.is_ok(),
                "Valid log level '{}' should be accepted",
                level
            );
        }
    }

    #[test]
    fn test_log_level_case_insensitive() {
        let args = vec![
            "--log-level".to_string(),
            "DEBUG".to_string(),
            "status".to_string(),
        ];
        let result = parse_args_from(&args);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().log_level.unwrap(), "debug");
    }

    #[test]
    fn test_vpn_name_empty_rejected() {
        let args = vec!["connect".to_string(), "".to_string()];
        let result = parse_args_from(&args);
        assert!(result.is_err(), "Empty VPN name should be rejected");
    }

    #[test]
    fn test_vpn_name_too_long_rejected() {
        let long_name = "a".repeat(300);
        let args = vec!["connect".to_string(), long_name];
        let result = parse_args_from(&args);
        assert!(result.is_err(), "Very long VPN name should be rejected");
    }

    #[test]
    fn test_vpn_name_null_bytes_rejected() {
        let args = vec!["connect".to_string(), "vpn\x00hidden".to_string()];
        let result = parse_args_from(&args);
        assert!(
            result.is_err(),
            "VPN name with null bytes should be rejected"
        );
    }

    #[test]
    fn test_vpn_name_newlines_rejected() {
        let args = vec!["connect".to_string(), "vpn\ninjected".to_string()];
        let result = parse_args_from(&args);
        assert!(result.is_err(), "VPN name with newlines should be rejected");
    }

    #[test]
    fn test_vpn_name_shell_chars_allowed() {
        let args = vec!["connect".to_string(), "$(whoami)".to_string()];
        let result = parse_args_from(&args);
        assert!(
            result.is_ok(),
            "Shell chars should be allowed (handled safely)"
        );
    }

    #[test]
    fn test_log_path_empty_rejected() {
        let args = vec![
            "--log-file".to_string(),
            "".to_string(),
            "status".to_string(),
        ];
        let result = parse_args_from(&args);
        assert!(result.is_err(), "Empty log path should be rejected");
    }

    #[test]
    fn test_log_path_null_bytes_rejected() {
        let args = vec![
            "--log-file".to_string(),
            "/tmp/log\x00.txt".to_string(),
            "status".to_string(),
        ];
        let result = parse_args_from(&args);
        assert!(
            result.is_err(),
            "Log path with null bytes should be rejected"
        );
    }

    #[test]
    fn test_verbosity_clamped() {
        let args = vec!["-vvvvvvvvvv".to_string(), "status".to_string()];
        let result = parse_args_from(&args);
        assert!(result.is_ok());
        assert!(
            result.unwrap().verbose <= 3,
            "Verbosity should be clamped to 3"
        );
    }
}
