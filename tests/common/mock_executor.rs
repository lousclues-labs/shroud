//! Mock command executor for testing
//!
//! Provides a mock implementation of command execution (iptables, ip, etc.)
//! that allows testing kill switch logic without requiring root access.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Mock command executor that records calls and returns preset results
///
/// Use this to:
/// - Test iptables rule generation without running as root
/// - Verify correct command arguments are passed
/// - Simulate failure scenarios
#[derive(Clone)]
pub struct MockExecutor {
    calls: Arc<Mutex<Vec<MockCommand>>>,
    results: Arc<Mutex<HashMap<String, MockResult>>>,
    default_result: Arc<Mutex<MockResult>>,
    exit_codes: Arc<Mutex<HashMap<String, i32>>>,
}

/// A recorded command execution
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockCommand {
    pub program: String,
    pub args: Vec<String>,
    pub sudo: bool,
}

impl MockCommand {
    /// Check if command contains a pattern
    pub fn contains(&self, pattern: &str) -> bool {
        self.program.contains(pattern)
            || self.args.iter().any(|a| a.contains(pattern))
            || self.full_command().contains(pattern)
    }

    /// Get full command as string
    pub fn full_command(&self) -> String {
        let prefix = if self.sudo { "sudo " } else { "" };
        format!("{}{} {}", prefix, self.program, self.args.join(" "))
    }
}

/// Result of a mock command execution
#[derive(Debug, Clone)]
pub struct MockResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl MockResult {
    /// Successful result with no output
    pub fn ok() -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    /// Successful result with stdout
    pub fn ok_with_output(stdout: &str) -> Self {
        Self {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    /// Failed result with stderr
    pub fn error(stderr: &str) -> Self {
        Self {
            stdout: String::new(),
            stderr: stderr.to_string(),
            exit_code: 1,
        }
    }

    /// Failed result with specific exit code
    pub fn error_with_code(stderr: &str, code: i32) -> Self {
        Self {
            stdout: String::new(),
            stderr: stderr.to_string(),
            exit_code: code,
        }
    }
}

impl MockExecutor {
    /// Create a new mock executor with default success results
    pub fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(HashMap::new())),
            default_result: Arc::new(Mutex::new(MockResult::ok())),
            exit_codes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create executor that fails all commands by default
    pub fn failing() -> Self {
        let exec = Self::new();
        exec.set_default(MockResult::error("Command failed"));
        exec
    }

    /// Set result for commands matching a pattern
    ///
    /// The pattern is matched against the full command string.
    pub fn when(&self, pattern: &str, result: MockResult) {
        self.results
            .lock()
            .unwrap()
            .insert(pattern.to_string(), result);
    }

    /// Set default result for unmatched commands
    pub fn set_default(&self, result: MockResult) {
        *self.default_result.lock().unwrap() = result;
    }

    /// Set expected exit code for a pattern
    pub fn expect_exit_code(&self, pattern: &str, code: i32) {
        self.exit_codes
            .lock()
            .unwrap()
            .insert(pattern.to_string(), code);
    }

    /// Get all executed commands
    pub fn calls(&self) -> Vec<MockCommand> {
        self.calls.lock().unwrap().clone()
    }

    /// Check if any command matching pattern was executed
    pub fn was_called(&self, pattern: &str) -> bool {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.contains(pattern))
    }

    /// Count how many times a pattern was matched
    pub fn call_count(&self, pattern: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.contains(pattern))
            .count()
    }

    /// Get commands matching a pattern
    pub fn find_calls(&self, pattern: &str) -> Vec<MockCommand> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.contains(pattern))
            .cloned()
            .collect()
    }

    /// Clear call history
    pub fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }

    /// Execute a command (records call and returns preset result)
    pub fn execute(&self, program: &str, args: &[&str]) -> MockResult {
        self.execute_internal(program, args, false)
    }

    /// Execute a command with sudo
    pub fn execute_sudo(&self, program: &str, args: &[&str]) -> MockResult {
        self.execute_internal(program, args, true)
    }

    fn execute_internal(&self, program: &str, args: &[&str], sudo: bool) -> MockResult {
        let cmd = MockCommand {
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            sudo,
        };

        self.calls.lock().unwrap().push(cmd.clone());

        // Find matching result
        let full_cmd = cmd.full_command();
        let results = self.results.lock().unwrap();

        for (pattern, result) in results.iter() {
            if full_cmd.contains(pattern) {
                return result.clone();
            }
        }

        self.default_result.lock().unwrap().clone()
    }

    // ========================================================================
    // Convenience methods for common iptables patterns
    // ========================================================================

    /// Set up mock for successful iptables operations
    pub fn mock_iptables_success(&self) {
        self.when("iptables", MockResult::ok());
        self.when("ip6tables", MockResult::ok());
    }

    /// Set up mock for iptables permission denied
    pub fn mock_iptables_permission_denied(&self) {
        self.when(
            "iptables",
            MockResult::error("iptables: Permission denied (you must be root)."),
        );
    }

    /// Set up mock for iptables not found
    pub fn mock_iptables_not_found(&self) {
        self.when(
            "iptables",
            MockResult::error_with_code("iptables: command not found", 127),
        );
    }

    /// Set up mock to report an existing chain
    pub fn mock_iptables_chain_exists(&self, chain: &str) {
        self.when(
            &format!("-L {}", chain),
            MockResult::ok_with_output(&format!(
                "Chain {} (0 references)\ntarget prot opt source destination",
                chain
            )),
        );
    }

    /// Set up mock for nftables operations
    pub fn mock_nft_success(&self) {
        self.when("nft", MockResult::ok());
    }

    /// Mock ip command for routing table queries
    pub fn mock_ip_route(&self, output: &str) {
        self.when("ip route", MockResult::ok_with_output(output));
    }
}

impl Default for MockExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_executor_basic() {
        let exec = MockExecutor::new();

        // Execute a command
        let result = exec.execute("iptables", &["-L", "-n"]);
        assert_eq!(result.exit_code, 0);

        // Verify it was recorded
        assert!(exec.was_called("iptables"));
        assert!(exec.was_called("-L"));
        assert_eq!(exec.call_count("iptables"), 1);
    }

    #[test]
    fn test_mock_executor_pattern_matching() {
        let exec = MockExecutor::new();
        exec.when("iptables -L", MockResult::ok_with_output("some rules"));
        exec.when("iptables -A", MockResult::error("permission denied"));

        // List should succeed
        let result = exec.execute("iptables", &["-L", "OUTPUT"]);
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("some rules"));

        // Append should fail
        let result = exec.execute("iptables", &["-A", "OUTPUT", "-j", "DROP"]);
        assert_eq!(result.exit_code, 1);
    }

    #[test]
    fn test_mock_executor_sudo() {
        let exec = MockExecutor::new();

        exec.execute_sudo("iptables", &["-F"]);

        let calls = exec.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].sudo);
        assert!(calls[0].full_command().starts_with("sudo"));
    }

    #[test]
    fn test_mock_executor_find_calls() {
        let exec = MockExecutor::new();

        exec.execute("iptables", &["-A", "OUTPUT", "-j", "DROP"]);
        exec.execute("iptables", &["-A", "OUTPUT", "-j", "ACCEPT"]);
        exec.execute("ip6tables", &["-A", "OUTPUT", "-j", "DROP"]);

        // "iptables" only matches exact "iptables" commands (not ip6tables - different string)
        let iptables_calls = exec.find_calls("iptables");
        assert_eq!(iptables_calls.len(), 2);

        // "tables" matches all of them
        let all_tables = exec.find_calls("tables");
        assert_eq!(all_tables.len(), 3);

        let drop_calls = exec.find_calls("DROP");
        assert_eq!(drop_calls.len(), 2);
    }
}
