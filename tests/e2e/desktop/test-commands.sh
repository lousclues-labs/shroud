#!/usr/bin/env bash
#
# Test: CLI Commands
#
# Verifies that all CLI commands work correctly in desktop mode.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SHROUD_BIN="${SHROUD_BIN:-./target/release/shroud}"

PASSED=0
FAILED=0
DAEMON_PID=""

pass() { echo "  ✓ $1"; PASSED=$((PASSED + 1)); }
fail() { echo "  ✗ $1"; FAILED=$((FAILED + 1)); }

cleanup() {
    if [[ -n "$DAEMON_PID" ]]; then
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

start_daemon() {
    "$SHROUD_BIN" --desktop 2>/dev/null &
    DAEMON_PID=$!
    
    local attempts=0
    while ! "$SHROUD_BIN" ping 2>/dev/null; do
        sleep 0.5
        ((attempts++))
        if [[ $attempts -ge 20 ]]; then
            return 1
        fi
    done
}

echo "=== CLI Command Tests ==="
echo ""

# Test 1: Version command (no daemon needed)
test_version() {
    local output
    output=$("$SHROUD_BIN" --version 2>&1)
    
    if [[ "$output" == *"shroud"* ]] && [[ "$output" == *"1."* ]]; then
        pass "Version command works"
    else
        fail "Version command failed: $output"
    fi
}

# Test 2: Help command (no daemon needed)
test_help() {
    local output
    output=$("$SHROUD_BIN" --help 2>&1)
    
    if [[ "$output" == *"USAGE"* ]] || [[ "$output" == *"Usage"* ]] || [[ "$output" == *"usage"* ]]; then
        pass "Help command works"
    else
        fail "Help command failed: $output"
    fi
}

# Test 3: Help for subcommands
test_subcommand_help() {
    local output
    output=$("$SHROUD_BIN" help connect 2>&1) || true
    
    if [[ "$output" == *"connect"* ]] || [[ "$output" == *"Connect"* ]]; then
        pass "Subcommand help works"
    else
        fail "Subcommand help failed: $output"
    fi
}

# Test 4: Status command
test_status_command() {
    local output
    output=$("$SHROUD_BIN" status 2>&1)
    
    if [[ "$output" == *"Disconnected"* ]] || [[ "$output" == *"Connected"* ]] || [[ "$output" == *"state"* ]]; then
        pass "Status command works"
    else
        fail "Status command failed: $output"
    fi
}

# Test 5: List command
test_list_command() {
    local output
    output=$("$SHROUD_BIN" list 2>&1) || true
    
    # Should not error even if no VPNs configured
    if [[ "$output" != *"Error"* ]] || [[ "$output" == *"No VPN"* ]]; then
        pass "List command works"
    else
        fail "List command failed: $output"
    fi
}

# Test 6: List with JSON
test_list_json() {
    local output
    output=$("$SHROUD_BIN" list --json 2>&1) || true
    
    if [[ "$output" == "["* ]] || [[ "$output" == "{"* ]] || [[ "$output" == "[]" ]]; then
        pass "List --json works"
    else
        fail "List --json failed: $output"
    fi
}

# Test 7: Kill switch status
test_ks_status_command() {
    local output
    output=$("$SHROUD_BIN" ks status 2>&1) || true
    
    if [[ "$output" == *"enabled"* ]] || [[ "$output" == *"disabled"* ]] || [[ "$output" == *"Kill"* ]]; then
        pass "Kill switch status command works"
    else
        fail "Kill switch status failed: $output"
    fi
}

# Test 8: Autostart status
test_autostart_status() {
    local output
    output=$("$SHROUD_BIN" autostart status 2>&1) || true
    
    if [[ "$output" == *"enabled"* ]] || [[ "$output" == *"disabled"* ]] || [[ "$output" == *"Autostart"* ]]; then
        pass "Autostart status works"
    else
        fail "Autostart status failed: $output"
    fi
}

# Test 9: Debug log path
test_debug_log_path() {
    local output
    output=$("$SHROUD_BIN" debug log-path 2>&1) || true
    
    if [[ "$output" == *"/"* ]] || [[ "$output" == *"shroud"* ]]; then
        pass "Debug log-path works"
    else
        fail "Debug log-path failed: $output"
    fi
}

# Test 10: Ping command
test_ping_command() {
    if "$SHROUD_BIN" ping 2>/dev/null; then
        pass "Ping command works"
    else
        fail "Ping command failed"
    fi
}

# Run tests that don't need daemon
test_version
test_help
test_subcommand_help

# Start daemon and run remaining tests
if start_daemon; then
    sleep 1
    
    test_status_command
    test_list_command
    test_list_json
    test_ks_status_command
    test_autostart_status
    test_debug_log_path
    test_ping_command
else
    fail "Could not start daemon"
fi

# Cleanup
"$SHROUD_BIN" quit 2>/dev/null || true

echo ""
echo "CLI Commands: $PASSED passed, $FAILED failed"

[[ $FAILED -eq 0 ]]
