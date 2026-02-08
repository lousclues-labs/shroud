//! Headless-mode configuration helpers — pure functions, easily testable.
//!
//! Parses stdin commands, validates headless config, and builds
//! systemd notification strings without any I/O.

use std::time::Duration;

/// Stdin command for headless mode interactive control.
#[derive(Debug, Clone, PartialEq)]
pub enum StdinCommand {
    Connect(String),
    Disconnect,
    Status,
    List,
    KillSwitchOn,
    KillSwitchOff,
    Quit,
    Help,
    Unknown(String),
}

impl StdinCommand {
    /// Parse a line of stdin input into a command.
    pub fn parse(input: &str) -> Self {
        let input = input.trim();
        let parts: Vec<&str> = input.split_whitespace().collect();

        if parts.is_empty() {
            return StdinCommand::Unknown(String::new());
        }

        match parts[0].to_lowercase().as_str() {
            "connect" | "c" => {
                if parts.len() > 1 {
                    StdinCommand::Connect(parts[1].to_string())
                } else {
                    StdinCommand::Unknown("connect requires VPN name".into())
                }
            }
            "disconnect" | "d" => StdinCommand::Disconnect,
            "status" | "s" => StdinCommand::Status,
            "list" | "l" => StdinCommand::List,
            "ks-on" | "kill-switch-on" => StdinCommand::KillSwitchOn,
            "ks-off" | "kill-switch-off" => StdinCommand::KillSwitchOff,
            "quit" | "exit" | "q" => StdinCommand::Quit,
            "help" | "h" | "?" => StdinCommand::Help,
            _ => StdinCommand::Unknown(input.to_string()),
        }
    }

    /// Return multi-line help text listing available commands.
    pub fn help_text() -> &'static str {
        concat!(
            "Available commands:\n",
            "  connect <vpn>  - Connect to VPN\n",
            "  disconnect     - Disconnect from VPN\n",
            "  status         - Show connection status\n",
            "  list           - List available VPNs\n",
            "  ks-on          - Enable kill switch\n",
            "  ks-off         - Disable kill switch\n",
            "  quit           - Stop the daemon\n",
            "  help           - Show this help\n",
            "\n",
            "Shortcuts: c, d, s, l, q, h",
        )
    }
}

/// Headless log-level enum (independent of the `log` crate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "error" => LogLevel::Error,
            "warn" | "warning" => LogLevel::Warn,
            "info" => LogLevel::Info,
            "debug" => LogLevel::Debug,
            "trace" => LogLevel::Trace,
            _ => LogLevel::Info,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

/// Validate a watchdog interval parsed from `WATCHDOG_USEC`.
pub fn validate_watchdog_usec(usec_str: &str) -> Result<Duration, String> {
    let usec: u64 = usec_str
        .parse()
        .map_err(|_| format!("Invalid WATCHDOG_USEC: {}", usec_str))?;

    if usec == 0 {
        return Err("Watchdog interval cannot be zero".into());
    }

    // Systemd recommends notifying at half the interval
    Ok(Duration::from_micros(usec / 2))
}

/// Validate a VPN server name for auto-connect.
pub fn validate_auto_connect(server: &str) -> Result<(), String> {
    if server.is_empty() {
        return Err("Auto-connect VPN name cannot be empty".into());
    }
    if server.len() > 255 {
        return Err("Auto-connect VPN name too long".into());
    }
    Ok(())
}

/// Systemd notification message builders (pure strings, no I/O).
pub mod systemd_msg {
    pub const READY: &str = "READY=1";
    pub const STOPPING: &str = "STOPPING=1";
    pub const WATCHDOG: &str = "WATCHDOG=1";
    pub const RELOADING: &str = "RELOADING=1";

    pub fn status(msg: &str) -> String {
        format!("STATUS={}", msg)
    }

    pub fn mainpid(pid: u32) -> String {
        format!("MAINPID={}", pid)
    }

    pub fn errno(code: i32) -> String {
        format!("ERRNO={}", code)
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod stdin_command {
        use super::*;

        #[test]
        fn test_connect() {
            assert_eq!(
                StdinCommand::parse("connect my-vpn"),
                StdinCommand::Connect("my-vpn".into())
            );
        }

        #[test]
        fn test_connect_shortcut() {
            assert_eq!(
                StdinCommand::parse("c vpn"),
                StdinCommand::Connect("vpn".into())
            );
        }

        #[test]
        fn test_connect_missing_arg() {
            assert!(matches!(
                StdinCommand::parse("connect"),
                StdinCommand::Unknown(_)
            ));
        }

        #[test]
        fn test_disconnect() {
            assert_eq!(StdinCommand::parse("disconnect"), StdinCommand::Disconnect);
            assert_eq!(StdinCommand::parse("d"), StdinCommand::Disconnect);
        }

        #[test]
        fn test_status() {
            assert_eq!(StdinCommand::parse("status"), StdinCommand::Status);
            assert_eq!(StdinCommand::parse("s"), StdinCommand::Status);
        }

        #[test]
        fn test_list() {
            assert_eq!(StdinCommand::parse("list"), StdinCommand::List);
            assert_eq!(StdinCommand::parse("l"), StdinCommand::List);
        }

        #[test]
        fn test_killswitch_on() {
            assert_eq!(StdinCommand::parse("ks-on"), StdinCommand::KillSwitchOn);
            assert_eq!(
                StdinCommand::parse("kill-switch-on"),
                StdinCommand::KillSwitchOn
            );
        }

        #[test]
        fn test_killswitch_off() {
            assert_eq!(StdinCommand::parse("ks-off"), StdinCommand::KillSwitchOff);
            assert_eq!(
                StdinCommand::parse("kill-switch-off"),
                StdinCommand::KillSwitchOff
            );
        }

        #[test]
        fn test_quit_variants() {
            assert_eq!(StdinCommand::parse("quit"), StdinCommand::Quit);
            assert_eq!(StdinCommand::parse("exit"), StdinCommand::Quit);
            assert_eq!(StdinCommand::parse("q"), StdinCommand::Quit);
        }

        #[test]
        fn test_help_variants() {
            assert_eq!(StdinCommand::parse("help"), StdinCommand::Help);
            assert_eq!(StdinCommand::parse("h"), StdinCommand::Help);
            assert_eq!(StdinCommand::parse("?"), StdinCommand::Help);
        }

        #[test]
        fn test_unknown() {
            assert_eq!(
                StdinCommand::parse("foobar"),
                StdinCommand::Unknown("foobar".into())
            );
        }

        #[test]
        fn test_empty_input() {
            assert!(matches!(StdinCommand::parse(""), StdinCommand::Unknown(_)));
        }

        #[test]
        fn test_whitespace_only() {
            assert!(matches!(
                StdinCommand::parse("   "),
                StdinCommand::Unknown(_)
            ));
        }

        #[test]
        fn test_whitespace_handling() {
            assert_eq!(StdinCommand::parse("  status  "), StdinCommand::Status);
            assert_eq!(
                StdinCommand::parse("  connect   vpn  "),
                StdinCommand::Connect("vpn".into())
            );
        }

        #[test]
        fn test_case_insensitive() {
            assert_eq!(StdinCommand::parse("STATUS"), StdinCommand::Status);
            assert_eq!(
                StdinCommand::parse("CONNECT vpn"),
                StdinCommand::Connect("vpn".into())
            );
            assert_eq!(StdinCommand::parse("QUIT"), StdinCommand::Quit);
        }

        #[test]
        fn test_help_text_contains_commands() {
            let help = StdinCommand::help_text();
            assert!(help.contains("connect"));
            assert!(help.contains("disconnect"));
            assert!(help.contains("status"));
            assert!(help.contains("list"));
            assert!(help.contains("quit"));
            assert!(help.contains("help"));
            assert!(help.contains("ks-on"));
            assert!(help.contains("ks-off"));
        }
    }

    mod log_level {
        use super::*;

        #[test]
        fn test_from_str_known() {
            assert_eq!(LogLevel::from_str_lossy("error"), LogLevel::Error);
            assert_eq!(LogLevel::from_str_lossy("warn"), LogLevel::Warn);
            assert_eq!(LogLevel::from_str_lossy("warning"), LogLevel::Warn);
            assert_eq!(LogLevel::from_str_lossy("info"), LogLevel::Info);
            assert_eq!(LogLevel::from_str_lossy("debug"), LogLevel::Debug);
            assert_eq!(LogLevel::from_str_lossy("trace"), LogLevel::Trace);
        }

        #[test]
        fn test_from_str_case_insensitive() {
            assert_eq!(LogLevel::from_str_lossy("ERROR"), LogLevel::Error);
            assert_eq!(LogLevel::from_str_lossy("WARN"), LogLevel::Warn);
            assert_eq!(LogLevel::from_str_lossy("Info"), LogLevel::Info);
        }

        #[test]
        fn test_from_str_unknown_defaults_to_info() {
            assert_eq!(LogLevel::from_str_lossy("unknown"), LogLevel::Info);
            assert_eq!(LogLevel::from_str_lossy(""), LogLevel::Info);
        }

        #[test]
        fn test_as_str() {
            assert_eq!(LogLevel::Error.as_str(), "error");
            assert_eq!(LogLevel::Warn.as_str(), "warn");
            assert_eq!(LogLevel::Info.as_str(), "info");
            assert_eq!(LogLevel::Debug.as_str(), "debug");
            assert_eq!(LogLevel::Trace.as_str(), "trace");
        }

        #[test]
        fn test_roundtrip() {
            for level in &[
                LogLevel::Error,
                LogLevel::Warn,
                LogLevel::Info,
                LogLevel::Debug,
                LogLevel::Trace,
            ] {
                assert_eq!(LogLevel::from_str_lossy(level.as_str()), *level);
            }
        }
    }

    mod watchdog_validation {
        use super::*;

        #[test]
        fn test_valid_watchdog() {
            let dur = validate_watchdog_usec("30000000").unwrap();
            assert_eq!(dur.as_secs(), 15); // half of 30s
        }

        #[test]
        fn test_valid_watchdog_small() {
            let dur = validate_watchdog_usec("2000000").unwrap();
            assert_eq!(dur.as_secs(), 1);
        }

        #[test]
        fn test_zero_watchdog() {
            assert!(validate_watchdog_usec("0").is_err());
        }

        #[test]
        fn test_non_numeric_watchdog() {
            assert!(validate_watchdog_usec("abc").is_err());
        }

        #[test]
        fn test_empty_watchdog() {
            assert!(validate_watchdog_usec("").is_err());
        }
    }

    mod auto_connect_validation {
        use super::*;

        #[test]
        fn test_valid_server() {
            assert!(validate_auto_connect("my-vpn").is_ok());
        }

        #[test]
        fn test_empty_server() {
            assert!(validate_auto_connect("").is_err());
        }

        #[test]
        fn test_too_long_server() {
            let long = "x".repeat(256);
            assert!(validate_auto_connect(&long).is_err());
        }
    }

    mod systemd_messages {
        use super::*;

        #[test]
        fn test_constants() {
            assert_eq!(systemd_msg::READY, "READY=1");
            assert_eq!(systemd_msg::STOPPING, "STOPPING=1");
            assert_eq!(systemd_msg::WATCHDOG, "WATCHDOG=1");
            assert_eq!(systemd_msg::RELOADING, "RELOADING=1");
        }

        #[test]
        fn test_status() {
            assert_eq!(systemd_msg::status("Running"), "STATUS=Running");
            assert_eq!(
                systemd_msg::status("Connected to vpn"),
                "STATUS=Connected to vpn"
            );
        }

        #[test]
        fn test_mainpid() {
            assert_eq!(systemd_msg::mainpid(12345), "MAINPID=12345");
            assert_eq!(systemd_msg::mainpid(1), "MAINPID=1");
        }

        #[test]
        fn test_errno() {
            assert_eq!(systemd_msg::errno(0), "ERRNO=0");
            assert_eq!(systemd_msg::errno(-1), "ERRNO=-1");
        }
    }
}
