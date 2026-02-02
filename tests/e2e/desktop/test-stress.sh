#!/usr/bin/env bash
#
# Test: Stress Testing
#
# Verifies that rapid operations don't cause hangs, crashes, or deadlocks.

set -euo pipefail

SHROUD_BIN="${SHROUD_BIN:-./target/release/shroud}"

PASSED=0
FAILED=0
DAEMON_PID=""

pass() { echo "  ✓ $1"; PASSED=$((PASSED + 1)); }
fail() { echo "  ✗ $1"; FAILED=$((FAILED + 1)); }

cleanup() {
    [[ -n "$DAEMON_PID" ]] && kill "$DAEMON_PID" 2>/dev/null || true
}
trap cleanup EXIT

start_daemon() {
    "$SHROUD_BIN" --desktop 2>/dev/null &
    DAEMON_PID=$!
    
    local attempts=0
    while ! "$SHROUD_BIN" ping 2>/dev/null; do
        sleep 0.5
        ((attempts++))
        [[ $attempts -ge 20 ]] && return 1
    done
}

echo "=== Stress Tests ==="
echo ""

# Test 1: Rapid status checks
test_rapid_status() {
    local start end duration
    start=$(date +%s%3N)
    
    for _ in {1..50}; do
        "$SHROUD_BIN" status >/dev/null 2>&1 || true
    done
    
    end=$(date +%s%3N)
    duration=$((end - start))
    
    if [[ $duration -lt 10000 ]]; then
        pass "50 status checks in ${duration}ms"
    else
        fail "50 status checks took ${duration}ms (>10s)"
    fi
}

# Test 2: Parallel commands
test_parallel_commands() {
    local pids=()
    
    for _ in {1..10}; do
        "$SHROUD_BIN" status >/dev/null 2>&1 &
        pids+=($!)
    done
    
    local failures=0
    for pid in "${pids[@]}"; do
        if ! wait "$pid"; then
            ((failures++))
        fi
    done
    
    if [[ $failures -eq 0 ]]; then
        pass "10 parallel commands succeeded"
    else
        fail "$failures parallel commands failed"
    fi
}

# Test 3: Kill switch toggle storm
test_ks_toggle_storm() {
    local failures=0
    
    for _ in {1..20}; do
        if ! timeout 2 "$SHROUD_BIN" ks toggle >/dev/null 2>&1; then
            ((failures++))
        fi
    done
    
    if [[ $failures -eq 0 ]]; then
        pass "20 kill switch toggles succeeded"
    elif [[ $failures -lt 5 ]]; then
        pass "Kill switch toggles mostly succeeded ($failures failures)"
    else
        fail "Kill switch toggle storm failed ($failures failures)"
    fi
}

# Test 4: Mixed command stress
test_mixed_commands() {
    local commands=("status" "ping" "ks status" "list")
    local failures=0
    
    for _ in {1..25}; do
        local cmd="${commands[$RANDOM % ${#commands[@]}]}"
        if ! timeout 3 "$SHROUD_BIN" $cmd >/dev/null 2>&1; then
            ((failures++))
        fi
    done
    
    if [[ $failures -lt 3 ]]; then
        pass "Mixed command stress (25 ops, $failures failures)"
    else
        fail "Mixed command stress had $failures failures"
    fi
}

# Test 5: Daemon still responsive after stress
test_responsive_after_stress() {
    sleep 1
    
    if timeout 5 "$SHROUD_BIN" status >/dev/null 2>&1; then
        pass "Daemon responsive after stress"
    else
        fail "Daemon unresponsive after stress"
    fi
}

# Test 6: No zombie processes
test_no_zombies() {
    local zombies
    zombies=$(ps aux | grep shroud | grep -c defunct || echo "0")
    
    if [[ "$zombies" -eq 0 ]]; then
        pass "No zombie processes"
    else
        fail "$zombies zombie shroud processes found"
    fi
}

# Start and run tests
if start_daemon; then
    sleep 1
    
    test_rapid_status
    test_parallel_commands
    test_ks_toggle_storm
    test_mixed_commands
    test_responsive_after_stress
    test_no_zombies
else
    fail "Could not start daemon"
fi

"$SHROUD_BIN" quit 2>/dev/null || true

echo ""
echo "Stress Tests: $PASSED passed, $FAILED failed"

[[ $FAILED -eq 0 ]]
