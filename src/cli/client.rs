//! CLI client for sending commands to the daemon
//!
//! Connects to the daemon's Unix socket and sends commands.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use crate::cli::commands::{CliCommand, CliRequest, CliResponse};
use crate::cli::error::CliError;

/// Output format for CLI responses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Human,
    Json,
}

/// Get the socket path
pub fn get_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("shroud.sock")
    } else {
        // Fallback using UID
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/tmp/shroud-{}.sock", uid))
    }
}

/// Send a command to the daemon and wait for response
pub fn send_command(command: CliCommand, timeout_secs: u64) -> Result<CliResponse, CliError> {
    let socket_path = get_socket_path();

    // Connect to daemon
    let stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(CliError::DaemonNotRunning);
        }
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            return Err(CliError::DaemonNotRunning);
        }
        Err(e) => return Err(CliError::Io(e)),
    };

    // Set timeouts
    let timeout = Duration::from_secs(timeout_secs);
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let mut stream = stream;
    let request = CliRequest::new(command);

    // Send request as JSON line
    let request_json = serde_json::to_string(&request)? + "\n";
    stream.write_all(request_json.as_bytes())?;
    stream.flush()?;

    // Read response
    let mut reader = BufReader::new(&stream);
    let mut response_line = String::new();
    match reader.read_line(&mut response_line) {
        Ok(0) => Err(CliError::ConnectionClosed),
        Ok(_) => {
            let response: CliResponse = serde_json::from_str(&response_line)?;
            Ok(response)
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Err(CliError::Timeout),
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Err(CliError::Timeout),
        Err(e) => Err(CliError::Io(e)),
    }
}

/// Print a response in human-readable or JSON format
pub fn print_response(response: &CliResponse, format: OutputFormat, quiet: bool) -> i32 {
    if quiet {
        return if response.success { 0 } else { 1 };
    }

    match format {
        OutputFormat::Json => {
            if let Some(ref data) = response.data {
                println!("{}", serde_json::to_string_pretty(data).unwrap_or_default());
            } else if let Some(ref error) = response.error {
                let err_json = serde_json::json!({
                    "error": {
                        "code": error.code,
                        "message": error.message
                    }
                });
                eprintln!("{}", serde_json::to_string_pretty(&err_json).unwrap_or_default());
            }
        }
        OutputFormat::Human => {
            if let Some(ref data) = response.data {
                print_human_readable(data);
            }
            if let Some(ref error) = response.error {
                eprintln!("Error: {}", error.message);
            }
        }
    }

    if response.success { 0 } else { 1 }
}

/// Print data in human-readable format
fn print_human_readable(data: &serde_json::Value) {
    // Handle message-only responses
    if let Some(msg) = data.get("message").and_then(|m| m.as_str()) {
        println!("{}", msg);
        return;
    }

    // Status response
    if let Some(state) = data.get("state").and_then(|s| s.as_str()) {
        println!("State: {}", state);
    }

    if let Some(conn) = data.get("connection") {
        if let Some(name) = conn.as_str() {
            println!("Connection: {}", name);
        }
    }

    if let Some(since) = data.get("connected_since").and_then(|s| s.as_str()) {
        if let Some(uptime) = data.get("uptime_seconds").and_then(|u| u.as_u64()) {
            println!("Connected since: {} ({})", since, format_duration(uptime));
        }
    }

    if let Some(ks) = data.get("kill_switch").and_then(|k| k.as_bool()) {
        println!(
            "Kill switch: {}",
            if ks { "enabled" } else { "disabled" }
        );
    }

    if let Some(ar) = data.get("auto_reconnect").and_then(|a| a.as_bool()) {
        println!(
            "Auto-reconnect: {}",
            if ar { "enabled" } else { "disabled" }
        );
    }

    // DNS/IPv6 mode for killswitch status
    if let Some(dns) = data.get("dns_mode").and_then(|d| d.as_str()) {
        println!("DNS mode: {}", dns);
    }
    if let Some(ipv6) = data.get("ipv6_mode").and_then(|i| i.as_str()) {
        println!("IPv6 mode: {}", ipv6);
    }

    // List response
    if let Some(connections) = data.get("connections").and_then(|c| c.as_array()) {
        let current = data.get("current").and_then(|c| c.as_str());
        println!("Available VPN connections:");
        for conn in connections {
            if let Some(name) = conn.as_str() {
                if Some(name) == current {
                    println!("  * {} (current)", name);
                } else {
                    println!("    {}", name);
                }
            }
        }
    }

    // Ping response
    if let Some(status) = data.get("status").and_then(|s| s.as_str()) {
        if status == "running" {
            let pid = data.get("pid").and_then(|p| p.as_u64()).unwrap_or(0);
            let version = data
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let uptime = data.get("uptime_seconds").and_then(|u| u.as_u64());
            if let Some(up) = uptime {
                println!(
                    "Shroud daemon is running (PID: {}, version: {}, uptime: {})",
                    pid,
                    version,
                    format_duration(up)
                );
            } else {
                println!(
                    "Shroud daemon is running (PID: {}, version: {})",
                    pid, version
                );
            }
        }
    }

    // Debug log path
    if let Some(path) = data.get("log_path").and_then(|p| p.as_str()) {
        println!("{}", path);
    }

    // Enabled/disabled status
    if let Some(enabled) = data.get("enabled").and_then(|e| e.as_bool()) {
        if data.get("feature").is_some() {
            let feature = data.get("feature").and_then(|f| f.as_str()).unwrap_or("");
            println!(
                "{}: {}",
                feature,
                if enabled { "enabled" } else { "disabled" }
            );
        }
    }
}

/// Format duration in human-readable form
fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else if seconds < 86400 {
        let hours = seconds / 3600;
        let mins = (seconds % 3600) / 60;
        format!("{}h {}m", hours, mins)
    } else {
        let days = seconds / 86400;
        let hours = (seconds % 86400) / 3600;
        format!("{}d {}h", days, hours)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(30), "30s");
        assert_eq!(format_duration(90), "1m 30s");
        assert_eq!(format_duration(3661), "1h 1m");
        assert_eq!(format_duration(90000), "1d 1h");
    }

    #[test]
    fn test_socket_path() {
        let path = get_socket_path();
        assert!(path.to_string_lossy().contains("shroud.sock"));
    }
}
