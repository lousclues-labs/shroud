#!/usr/bin/env bash
#
# Test: IPC Communication
#
# Verifies that CLI commands reach the daemon via IPC socket.
# This catches bugs like the tokio::spawn-in-std::thread issue.

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
    # Clean up socket
    rm -f "${XDG_RUNTIME_DIR:-/tmp}/shroud.sock" 2>/dev/null || true
}
trap cleanup EXIT

start_daemon() {
    echo "Starting daemon..."
    
    # Start in background with minimal features
    "$SHROUD_BIN" --desktop 2>/dev/null &
    DAEMON_PID=$!
    
    # Wait for daemon to be ready
    local attempts=0
    while ! "$SHROUD_BIN" ping 2>/dev/null; do
        sleep 0.5
        ((attempts++))
        if [[ $attempts -ge 20 ]]; then
            fail "Daemon failed to start"
            return 1
        fi
    done
    
    echo "Daemon started (PID: $DAEMON_PID)"
}

echo "=== IPC Communication Tests ==="
echo ""

# Test 1: Daemon responds to ping
test_ping() {
    if "$SHROUD_BIN" ping 2>/dev/null; then
        pass "Ping successful"
    else
        fail "Ping failed"
    fi
}

# Test 2: Status command returns data
test_status() {
    local output
    output=$("$SHROUD_BIN" status 2>&1)
    
    if [[ "$output" == *"State"* ]] || [[ "$output" == *"state"* ]] || [[ "$output" == *"Disconnected"* ]]; then
        pass "Status returns valid response"
    else
        fail "Status returned unexpected output: $output"
    fi
}

# Test 3: Status with JSON flag
test_status_json() {
    local output
    output=$("$SHROUD_BIN" status --json 2>&1)
    
    if [[ "$output" == "{"* ]] || [[ "$output" == "null" ]]; then
        pass "Status --json returns JSON"
    else
        fail "Status --json not valid JSON: $output"
    fi
}

# Test 4: List command works
test_list() {
    local output
    output=$("$SHROUD_BIN" list 2>&1) || true
    
    # Should return list (possibly empty) without error
    if [[ "$output" != *"error"* ]] && [[ "$output" != *"Error"* ]]; then
        pass "List command works"
    else
        fail "List command failed: $output"
    fi
}

# Test 5: Kill switch status
test_ks_status() {
    local output
    output=$("$SHROUD_BIN" ks status 2>&1) || true
    
    if [[ "$output" == *"Kill"* ]] || [[ "$output" == *"kill"* ]] || [[ "$output" == *"enabled"* ]] || [[ "$output" == *"disabled"* ]]; then
        pass "Kill switch status works"
    else
        fail "Kill switch status failed: $output"
    fi
}

# Test 6: Command timeout (ensures IPC is responsive)
test_command_timeout() {
    local start end duration
    start=$(date +%s%3N)
    
    "$SHROUD_BIN" status >/dev/null 2>&1
    
    end=$(date +%s%3N)
    duration=$((end - start))
    
    if [[ $duration -lt 5000 ]]; then
        pass "Command completed in ${duration}ms (< 5s)"
    else
        fail "Command took too long: ${duration}ms"
    fi
}

# Test 7: Multiple rapid commands (stress IPC)
test_rapid_commands() {
    local i failures=0
    
    for i in {1..10}; do
        if ! "$SHROUD_BIN" ping >/dev/null 2>&1; then
            ((failures++))
        fi
    done
    
    if [[ $failures -eq 0 ]]; then
        pass "10 rapid pings all succeeded"
    else
        fail "$failures/10 rapid pings failed"
    fi
}

# Test 8: IPC socket exists
test_socket_exists() {
    local socket="${XDG_RUNTIME_DIR:-/tmp}/shroud.sock"
    
    if [[ -S "$socket" ]]; then
        pass "IPC socket exists at $socket"
    else
        # Check alternative locations
        if [[ -S "/run/shroud/shroud.sock" ]]; then
            pass "IPC socket exists at /run/shroud/shroud.sock"
        else
            fail "IPC socket not found"
        fi
    fi
}

# Start daemon and run tests
if start_daemon; then
    sleep 1  # Let it fully initialize
    
    test_ping
    test_status
    test_status_json
    test_list
    test_ks_status
    test_command_timeout
    test_rapid_commands
    test_socket_exists
fi

# Stop daemon gracefully
"$SHROUD_BIN" quit 2>/dev/null || true
sleep 1

echo ""
echo "IPC Communication: $PASSED passed, $FAILED failed"

[[ $FAILED -eq 0 ]]
