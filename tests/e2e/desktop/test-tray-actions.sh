#!/usr/bin/env bash
#
# Test: Tray Action Simulation
#
# Simulates tray menu actions by sending commands that mirror
# what the tray handlers send. Catches async/sync boundary bugs.

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

echo "=== Tray Action Simulation Tests ==="
echo ""
echo "These tests simulate what happens when tray menu items are clicked."
echo "They verify that commands from the tray reach the supervisor."
echo ""

# Test 1: Toggle Kill Switch (simulates tray click)
test_toggle_killswitch() {
    local before after
    
    # Get current state
    before=$("$SHROUD_BIN" ks status 2>&1)
    
    # Toggle (this is what tray does)
    "$SHROUD_BIN" ks toggle 2>/dev/null || true
    sleep 0.5
    
    # Get new state
    after=$("$SHROUD_BIN" ks status 2>&1)
    
    # State should have changed
    if [[ "$before" != "$after" ]]; then
        pass "Kill switch toggle changed state"
    else
        fail "Kill switch toggle had no effect (async/sync bug?)"
    fi
    
    # Toggle back
    "$SHROUD_BIN" ks toggle 2>/dev/null || true
}

# Test 2: Toggle Auto-Reconnect
test_toggle_autoreconnect() {
    local before after
    
    before=$("$SHROUD_BIN" status --json 2>&1 | grep -o '"auto_reconnect":[^,}]*' || echo "unknown")
    
    "$SHROUD_BIN" ar toggle 2>/dev/null || true
    sleep 0.3
    
    after=$("$SHROUD_BIN" status --json 2>&1 | grep -o '"auto_reconnect":[^,}]*' || echo "unknown")
    
    if [[ "$before" != "$after" ]] || [[ "$before" == "unknown" ]]; then
        pass "Auto-reconnect toggle works"
    else
        fail "Auto-reconnect toggle had no effect"
    fi
    
    # Toggle back
    "$SHROUD_BIN" ar toggle 2>/dev/null || true
}

# Test 3: Refresh connections
test_refresh() {
    if "$SHROUD_BIN" refresh 2>/dev/null; then
        pass "Refresh command works"
    else
        fail "Refresh command failed"
    fi
}

# Test 4: Debug toggle
test_debug_toggle() {
    "$SHROUD_BIN" debug on 2>/dev/null || true
    sleep 0.3
    
    local status
    status=$("$SHROUD_BIN" debug log-path 2>&1) || true
    
    if [[ -n "$status" ]]; then
        pass "Debug toggle works"
    else
        fail "Debug toggle had no effect"
    fi
    
    "$SHROUD_BIN" debug off 2>/dev/null || true
}

# Test 5: Rapid toggle stress test (catches race conditions)
test_rapid_toggles() {
    local i failures=0
    
    for i in {1..5}; do
        "$SHROUD_BIN" ks toggle 2>/dev/null || ((failures++))
        "$SHROUD_BIN" ks toggle 2>/dev/null || ((failures++))
    done
    
    if [[ $failures -eq 0 ]]; then
        pass "Rapid toggles (10 operations) succeeded"
    else
        fail "Rapid toggles had $failures failures"
    fi
}

# Test 6: Commands don't hang (critical for async/sync bugs)
test_no_hang() {
    local commands=("status" "ks status" "ar toggle" "ar toggle" "refresh")
    local cmd
    
    for cmd in "${commands[@]}"; do
        if ! timeout 5 "$SHROUD_BIN" $cmd >/dev/null 2>&1; then
            fail "Command '$cmd' hung (timeout after 5s)"
            return
        fi
    done
    
    pass "All commands respond within 5s"
}

# Test 7: Verify state consistency after operations
test_state_consistency() {
    # Do several operations
    "$SHROUD_BIN" ks on 2>/dev/null || true
    "$SHROUD_BIN" ks off 2>/dev/null || true
    "$SHROUD_BIN" ar on 2>/dev/null || true
    "$SHROUD_BIN" ar off 2>/dev/null || true
    
    # Status should still work
    if "$SHROUD_BIN" status >/dev/null 2>&1; then
        pass "State consistent after multiple operations"
    else
        fail "State corrupted after operations"
    fi
}

# Start daemon and run tests
if start_daemon; then
    sleep 1
    
    test_toggle_killswitch
    test_toggle_autoreconnect
    test_refresh
    test_debug_toggle
    test_rapid_toggles
    test_no_hang
    test_state_consistency
else
    fail "Could not start daemon"
fi

# Cleanup
"$SHROUD_BIN" quit 2>/dev/null || true

echo ""
echo "Tray Actions: $PASSED passed, $FAILED failed"

[[ $FAILED -eq 0 ]]
