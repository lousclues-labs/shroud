// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! Input validation for CLI arguments
//!
//! This module provides validation functions for all user-provided inputs.
//! All validation happens at the CLI parsing boundary before values are used.
//!
//! Security principles:
//! - Reject invalid inputs early with clear error messages
//! - Use allowlists, not blocklists
//! - Validate type, range, format, and content
//! - No silent corrections (except documented clamping)

/// Maximum timeout value in seconds (1 hour)
pub const MAX_TIMEOUT_SECS: u64 = 3600;

/// Minimum timeout value in seconds
pub const MIN_TIMEOUT_SECS: u64 = 1;

/// Default timeout value in seconds
pub const DEFAULT_TIMEOUT_SECS: u64 = 5;

/// Maximum VPN name length
pub const MAX_VPN_NAME_LENGTH: usize = 256;

/// Maximum log file path length
pub const MAX_PATH_LENGTH: usize = 4096;

/// Valid log levels
pub const VALID_LOG_LEVELS: &[&str] = &["error", "warn", "info", "debug", "trace"];

/// Result type for validation
pub type ValidationResult<T> = Result<T, ValidationError>;

/// Validation error with context
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    #[allow(dead_code)]
    pub value: String,
    pub message: String,
    pub suggestion: Option<String>,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid {}: {}", self.field, self.message)?;
        if let Some(ref suggestion) = self.suggestion {
            write!(f, "\n  Suggestion: {}", suggestion)?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationError {}

impl ValidationError {
    pub fn new(field: &str, value: &str, message: &str) -> Self {
        Self {
            field: field.to_string(),
            value: value.to_string(),
            message: message.to_string(),
            suggestion: None,
        }
    }

    pub fn with_suggestion(mut self, suggestion: &str) -> Self {
        self.suggestion = Some(suggestion.to_string());
        self
    }
}

// ============================================================================
// TIMEOUT VALIDATION
// ============================================================================

/// Validate and parse timeout value
pub fn validate_timeout(value: &str) -> ValidationResult<u64> {
    let timeout: u64 = value.parse().map_err(|_| {
        ValidationError::new("timeout", value, "must be a positive integer").with_suggestion(
            &format!(
                "Use a value between {} and {} seconds",
                MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS
            ),
        )
    })?;

    if timeout < MIN_TIMEOUT_SECS {
        return Err(ValidationError::new(
            "timeout",
            value,
            &format!("must be at least {} second(s)", MIN_TIMEOUT_SECS),
        ));
    }

    if timeout > MAX_TIMEOUT_SECS {
        return Err(ValidationError::new(
            "timeout",
            value,
            &format!(
                "must be at most {} seconds ({} hour)",
                MAX_TIMEOUT_SECS,
                MAX_TIMEOUT_SECS / 3600
            ),
        )
        .with_suggestion("For long-running operations, use a shorter timeout with retries"));
    }

    Ok(timeout)
}

// ============================================================================
// LOG LEVEL VALIDATION
// ============================================================================

/// Validate log level
pub fn validate_log_level(value: &str) -> ValidationResult<String> {
    let normalized = value.to_lowercase();

    if VALID_LOG_LEVELS.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(ValidationError::new(
            "log level",
            value,
            &format!("must be one of: {}", VALID_LOG_LEVELS.join(", ")),
        )
        .with_suggestion("Use 'info' for normal operation, 'debug' for troubleshooting"))
    }
}

// ============================================================================
// VPN NAME VALIDATION
// ============================================================================

/// Validate VPN connection name
pub fn validate_vpn_name(value: &str) -> ValidationResult<String> {
    let trimmed = value.trim();

    if trimmed.is_empty() {
        return Err(ValidationError::new("VPN name", value, "cannot be empty")
            .with_suggestion("Use 'shroud list' to see available VPN connections"));
    }

    if trimmed.len() > MAX_VPN_NAME_LENGTH {
        return Err(ValidationError::new(
            "VPN name",
            &format!("{}...", &value[..value.len().min(50)]),
            &format!(
                "exceeds maximum length of {} characters",
                MAX_VPN_NAME_LENGTH
            ),
        ));
    }

    if trimmed.contains('\0') {
        return Err(ValidationError::new(
            "VPN name",
            value,
            "cannot contain null bytes",
        ));
    }

    if trimmed.contains('\n') || trimmed.contains('\r') || trimmed.contains('\t') {
        return Err(ValidationError::new(
            "VPN name",
            value,
            "cannot contain control characters (newline, carriage return, tab)",
        ));
    }

    // SECURITY: Reject any remaining control characters (SHROUD-VULN-023).
    // These can forge log lines or misalign terminal output.
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(ValidationError::new(
            "VPN name",
            value,
            "cannot contain control characters",
        ));
    }

    // SECURITY: Reject shell metacharacters and ANSI escape sequences.
    // VPN names are passed to nmcli via Command::new().args() (safe from shell
    // injection), but they also appear in log messages and iptables comments.
    const DENIED_CHARS: &[char] = &[';', '|', '&', '$', '`', '\\', '<', '>', '!'];
    const ANSI_ESCAPE: char = '\x1b';

    if trimmed.contains(ANSI_ESCAPE) || trimmed.chars().any(|c| DENIED_CHARS.contains(&c)) {
        return Err(ValidationError::new(
            "VPN name",
            value,
            "contains potentially dangerous characters (shell metacharacters or escape sequences are not allowed)",
        ));
    }

    Ok(trimmed.to_string())
}

// ============================================================================
// FILE PATH VALIDATION
// ============================================================================

/// Validate file path for log files
pub fn validate_log_path(value: &str) -> ValidationResult<std::path::PathBuf> {
    if value.is_empty() {
        return Err(ValidationError::new(
            "log file path",
            value,
            "cannot be empty",
        ));
    }

    if value.len() > MAX_PATH_LENGTH {
        return Err(ValidationError::new(
            "log file path",
            &format!("{}...", &value[..value.len().min(50)]),
            &format!("exceeds maximum length of {} characters", MAX_PATH_LENGTH),
        ));
    }

    if value.contains('\0') {
        return Err(ValidationError::new(
            "log file path",
            value,
            "cannot contain null bytes",
        ));
    }

    let path = std::path::PathBuf::from(value);

    if path.is_dir() {
        return Err(
            ValidationError::new("log file path", value, "is a directory, not a file")
                .with_suggestion("Specify a file path like '/path/to/shroud.log'"),
        );
    }

    let sensitive_prefixes = ["/etc/", "/bin/", "/sbin/", "/usr/bin/", "/usr/sbin/"];
    for prefix in sensitive_prefixes {
        if value.starts_with(prefix) {
            eprintln!("Warning: Log file in sensitive location: {}", value);
            break;
        }
    }

    Ok(path)
}

// ============================================================================
// VERBOSITY VALIDATION
// ============================================================================

/// Validate verbosity level
pub fn validate_verbosity(value: u8) -> u8 {
    value.min(3)
}

// ============================================================================
// GENERIC VALIDATORS
// ============================================================================

/// Check if a string contains shell metacharacters
#[allow(dead_code)]
pub fn contains_shell_metacharacters(value: &str) -> bool {
    let metacharacters = [
        '|', '&', ';', '$', '`', '(', ')', '{', '}', '[', ']', '<', '>', '!', '#', '*', '?', '~',
        '\\', '"', '\'', '\n',
    ];
    value.chars().any(|c| metacharacters.contains(&c))
}

/// Check if a string looks like a command injection attempt
pub fn looks_like_injection(value: &str) -> bool {
    let patterns = [
        "$(",
        "`",
        "&&",
        "||",
        "; ",
        "| ",
        "< ",
        "> ",
        "../",
        "..\\",
        "/etc/passwd",
        "/etc/shadow",
        "rm -rf",
        "chmod ",
        "chown ",
    ];

    let lower = value.to_lowercase();
    patterns.iter().any(|p| lower.contains(p))
}

/// Sanitize a string for safe display in logs/errors
pub fn sanitize_for_display(value: &str, max_len: usize) -> String {
    let sanitized: String = value
        .chars()
        .take(max_len)
        .map(|c| if c.is_control() { '?' } else { c })
        .collect();

    if value.len() > max_len {
        format!("{}...", sanitized)
    } else {
        sanitized
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_timeout_valid() {
        assert_eq!(validate_timeout("1").unwrap(), 1);
        assert_eq!(validate_timeout("5").unwrap(), 5);
        assert_eq!(validate_timeout("60").unwrap(), 60);
        assert_eq!(validate_timeout("3600").unwrap(), 3600);
    }

    #[test]
    fn test_validate_timeout_zero_rejected() {
        let err = validate_timeout("0").unwrap_err();
        assert!(err.message.contains("at least"));
    }

    #[test]
    fn test_validate_timeout_negative_rejected() {
        assert!(validate_timeout("-1").is_err());
        assert!(validate_timeout("-100").is_err());
    }

    #[test]
    fn test_validate_timeout_too_large_rejected() {
        let err = validate_timeout("3601").unwrap_err();
        assert!(err.message.contains("at most"));

        assert!(validate_timeout("999999").is_err());
        assert!(validate_timeout("999999999999").is_err());
    }

    #[test]
    fn test_validate_timeout_non_numeric_rejected() {
        assert!(validate_timeout("abc").is_err());
        assert!(validate_timeout("5s").is_err());
        assert!(validate_timeout("5.0").is_err());
        assert!(validate_timeout("").is_err());
        assert!(validate_timeout(" ").is_err());
    }

    #[test]
    fn test_validate_timeout_injection_rejected() {
        assert!(validate_timeout("5; rm -rf /").is_err());
        assert!(validate_timeout("$(whoami)").is_err());
    }

    #[test]
    fn test_validate_log_level_valid() {
        assert_eq!(validate_log_level("error").unwrap(), "error");
        assert_eq!(validate_log_level("warn").unwrap(), "warn");
        assert_eq!(validate_log_level("info").unwrap(), "info");
        assert_eq!(validate_log_level("debug").unwrap(), "debug");
        assert_eq!(validate_log_level("trace").unwrap(), "trace");
    }

    #[test]
    fn test_validate_log_level_case_insensitive() {
        assert_eq!(validate_log_level("DEBUG").unwrap(), "debug");
        assert_eq!(validate_log_level("Debug").unwrap(), "debug");
        assert_eq!(validate_log_level("INFO").unwrap(), "info");
    }

    #[test]
    fn test_validate_log_level_invalid() {
        assert!(validate_log_level("invalid").is_err());
        assert!(validate_log_level("warning").is_err());
        assert!(validate_log_level("err").is_err());
        assert!(validate_log_level("").is_err());
        assert!(validate_log_level(" ").is_err());
    }

    #[test]
    fn test_validate_log_level_injection() {
        assert!(validate_log_level("debug; rm -rf /").is_err());
        assert!(validate_log_level("$(whoami)").is_err());
    }

    #[test]
    fn test_validate_vpn_name_valid() {
        assert_eq!(validate_vpn_name("my-vpn").unwrap(), "my-vpn");
        assert_eq!(
            validate_vpn_name("VPN With Spaces").unwrap(),
            "VPN With Spaces"
        );
        assert_eq!(
            validate_vpn_name("vpn_underscore").unwrap(),
            "vpn_underscore"
        );
        assert_eq!(validate_vpn_name("vpn.with.dots").unwrap(), "vpn.with.dots");
    }

    #[test]
    fn test_validate_vpn_name_trims_whitespace() {
        assert_eq!(validate_vpn_name("  my-vpn  ").unwrap(), "my-vpn");
        assert_eq!(validate_vpn_name("\tmy-vpn\t").unwrap(), "my-vpn");
    }

    #[test]
    fn test_validate_vpn_name_empty_rejected() {
        assert!(validate_vpn_name("").is_err());
        assert!(validate_vpn_name("   ").is_err());
        assert!(validate_vpn_name("\t\n").is_err());
    }

    #[test]
    fn test_validate_vpn_name_too_long_rejected() {
        let long_name = "a".repeat(MAX_VPN_NAME_LENGTH + 1);
        assert!(validate_vpn_name(&long_name).is_err());
    }

    #[test]
    fn test_validate_vpn_name_null_bytes_rejected() {
        assert!(validate_vpn_name("vpn\x00hidden").is_err());
    }

    #[test]
    fn test_validate_vpn_name_newlines_rejected() {
        assert!(validate_vpn_name("vpn\ninjected").is_err());
        assert!(validate_vpn_name("vpn\r\ninjected").is_err());
    }

    #[test]
    fn test_validate_vpn_name_shell_chars_rejected() {
        assert!(validate_vpn_name("vpn; ls").is_err());
        assert!(validate_vpn_name("$(whoami)").is_err());
        assert!(validate_vpn_name("vpn | cat").is_err());
        assert!(validate_vpn_name("vpn & bg").is_err());
        assert!(validate_vpn_name("vpn`id`").is_err());
        assert!(validate_vpn_name("vpn\x1b[31m").is_err());
    }

    #[test]
    fn test_validate_vpn_name_real_world_names_accepted() {
        assert!(validate_vpn_name("user@company").is_ok());
        assert!(validate_vpn_name("VPN (office)").is_ok());
        assert!(validate_vpn_name("müllvad-se2").is_ok());
        assert!(validate_vpn_name("my.vpn.connection").is_ok());
        assert!(validate_vpn_name("vpn #3").is_ok());
    }

    #[test]
    fn test_validate_log_path_valid() {
        assert!(validate_log_path("/tmp/shroud.log").is_ok());
        assert!(validate_log_path("./debug.log").is_ok());
        assert!(validate_log_path("shroud.log").is_ok());
    }

    #[test]
    fn test_validate_log_path_empty_rejected() {
        assert!(validate_log_path("").is_err());
    }

    #[test]
    fn test_validate_log_path_too_long_rejected() {
        let long_path = format!("/{}", "a".repeat(MAX_PATH_LENGTH));
        assert!(validate_log_path(&long_path).is_err());
    }

    #[test]
    fn test_validate_log_path_null_bytes_rejected() {
        assert!(validate_log_path("/tmp/log\x00.txt").is_err());
    }

    #[test]
    fn test_contains_shell_metacharacters() {
        assert!(contains_shell_metacharacters("test; rm"));
        assert!(contains_shell_metacharacters("$(cmd)"));
        assert!(contains_shell_metacharacters("test | cat"));
        assert!(!contains_shell_metacharacters("normal-name"));
        assert!(!contains_shell_metacharacters("name with spaces"));
    }

    #[test]
    fn test_looks_like_injection() {
        assert!(looks_like_injection("$(whoami)"));
        assert!(looks_like_injection("test && rm -rf /"));
        assert!(looks_like_injection("../../../etc/passwd"));
        assert!(!looks_like_injection("normal-vpn-name"));
    }

    #[test]
    fn test_sanitize_for_display() {
        assert_eq!(sanitize_for_display("hello", 10), "hello");
        assert_eq!(sanitize_for_display("hello world", 5), "hello...");
        assert_eq!(sanitize_for_display("test\x00hidden", 20), "test?hidden");
        assert_eq!(sanitize_for_display("test\n\r", 20), "test??");
    }
}
