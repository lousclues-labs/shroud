//! CLI output formatting — pure functions, easily testable.
//!
//! Human-readable and JSON formatting for CLI status, list, error,
//! and success output.

use std::time::Duration;

/// Format a `Duration` for human-readable display.
pub fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        let mins = secs / 60;
        let rem = secs % 60;
        format!("{}m {}s", mins, rem)
    } else {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        format!("{}h {}m", hours, mins)
    }
}

/// Format a VPN list for CLI display.
pub fn format_list_output(vpns: &[String], active: Option<&str>) -> String {
    if vpns.is_empty() {
        return "No VPN connections configured.".into();
    }

    let mut lines = vec!["Available VPN connections:".to_string()];
    for vpn in vpns {
        if Some(vpn.as_str()) == active {
            lines.push(format!("  * {} (active)", vpn));
        } else {
            lines.push(format!("    {}", vpn));
        }
    }
    lines.join("\n")
}

/// Format an error message for CLI display.
pub fn format_error(error: &str) -> String {
    format!("Error: {}", error)
}

/// Format a success message for CLI display.
pub fn format_success(message: &str) -> String {
    format!("✓ {}", message)
}

/// CLI exit codes.
pub mod exit_codes {
    pub const SUCCESS: i32 = 0;
    pub const GENERAL_ERROR: i32 = 1;
    pub const USAGE_ERROR: i32 = 2;
    pub const CONNECTION_ERROR: i32 = 3;
    pub const TIMEOUT: i32 = 4;
    pub const NOT_RUNNING: i32 = 5;
}

/// Format any serialisable value as pretty-printed JSON.
pub fn format_json<T: serde::Serialize>(data: &T) -> Result<String, String> {
    serde_json::to_string_pretty(data).map_err(|e| e.to_string())
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod duration_tests {
        use super::*;

        #[test]
        fn test_zero() {
            assert_eq!(format_duration(Duration::ZERO), "0s");
        }

        #[test]
        fn test_seconds() {
            assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        }

        #[test]
        fn test_one_minute() {
            assert_eq!(format_duration(Duration::from_secs(60)), "1m 0s");
        }

        #[test]
        fn test_minutes_and_seconds() {
            assert_eq!(format_duration(Duration::from_secs(125)), "2m 5s");
        }

        #[test]
        fn test_one_hour() {
            assert_eq!(format_duration(Duration::from_secs(3600)), "1h 0m");
        }

        #[test]
        fn test_hours_and_minutes() {
            assert_eq!(format_duration(Duration::from_secs(3725)), "1h 2m");
        }

        #[test]
        fn test_many_hours() {
            assert_eq!(format_duration(Duration::from_secs(86400)), "24h 0m");
        }
    }

    mod list_tests {
        use super::*;

        #[test]
        fn test_empty() {
            let out = format_list_output(&[], None);
            assert!(out.contains("No VPN"));
        }

        #[test]
        fn test_with_vpns() {
            let vpns = vec!["vpn1".into(), "vpn2".into(), "vpn3".into()];
            let out = format_list_output(&vpns, None);
            assert!(out.contains("Available VPN"));
            assert!(out.contains("vpn1"));
            assert!(out.contains("vpn2"));
            assert!(out.contains("vpn3"));
        }

        #[test]
        fn test_with_active() {
            let vpns = vec!["vpn1".into(), "vpn2".into()];
            let out = format_list_output(&vpns, Some("vpn1"));
            assert!(out.contains("* vpn1 (active)"));
            assert!(!out.contains("* vpn2"));
        }

        #[test]
        fn test_active_not_in_list() {
            let vpns = vec!["vpn1".into()];
            let out = format_list_output(&vpns, Some("other"));
            assert!(!out.contains("(active)"));
        }

        #[test]
        fn test_single_vpn() {
            let vpns = vec!["only".into()];
            let out = format_list_output(&vpns, None);
            assert!(out.contains("only"));
        }
    }

    mod formatting_tests {
        use super::*;

        #[test]
        fn test_format_error() {
            let out = format_error("Something failed");
            assert!(out.starts_with("Error:"));
            assert!(out.contains("Something failed"));
        }

        #[test]
        fn test_format_success() {
            let out = format_success("Done");
            assert!(out.contains('✓'));
            assert!(out.contains("Done"));
        }
    }

    mod exit_code_tests {
        use super::exit_codes;

        #[test]
        fn test_success_is_zero() {
            assert_eq!(exit_codes::SUCCESS, 0);
        }

        #[test]
        fn test_errors_are_nonzero() {
            let error_codes = [
                exit_codes::GENERAL_ERROR,
                exit_codes::USAGE_ERROR,
                exit_codes::CONNECTION_ERROR,
                exit_codes::TIMEOUT,
                exit_codes::NOT_RUNNING,
            ];
            for code in error_codes {
                assert!(code > 0, "Exit code {} should be nonzero", code);
            }
        }

        #[test]
        fn test_codes_are_distinct() {
            let codes = [
                exit_codes::SUCCESS,
                exit_codes::GENERAL_ERROR,
                exit_codes::USAGE_ERROR,
                exit_codes::CONNECTION_ERROR,
                exit_codes::TIMEOUT,
                exit_codes::NOT_RUNNING,
            ];
            for i in 0..codes.len() {
                for j in (i + 1)..codes.len() {
                    assert_ne!(codes[i], codes[j], "Codes at {} and {} collide", i, j);
                }
            }
        }
    }

    mod json_tests {
        use super::*;

        #[test]
        fn test_format_json_simple() {
            let data = serde_json::json!({"name": "test", "value": 42});
            let json = format_json(&data).unwrap();
            assert!(json.contains("test"));
            assert!(json.contains("42"));
        }

        #[test]
        fn test_format_json_vec() {
            let data = vec!["a", "b", "c"];
            let json = format_json(&data).unwrap();
            assert!(json.contains("\"a\""));
        }
    }
}
