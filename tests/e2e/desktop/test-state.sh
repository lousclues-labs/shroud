#!/usr/bin/env bash
#
# Test: State Machine Consistency
#
# Verifies that the state machine remains consistent through various operations.

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

get_state() {
    "$SHROUD_BIN" status --json 2>/dev/null | grep -o '"state":"[^"]*"' | cut -d'"' -f4 || echo "unknown"
}

echo "=== State Consistency Tests ==="
echo ""

# Test 1: Initial state is Disconnected
test_initial_state() {
    local state
    state=$(get_state)
    
    if [[ "$state" == "Disconnected" ]] || [[ "$state" == "disconnected" ]]; then
        pass "Initial state is Disconnected"
    else
        fail "Unexpected initial state: $state"
    fi
}

# Test 2: State persists across status calls
test_state_persistence() {
    local state1 state2 state3
    
    state1=$(get_state)
    state2=$(get_state)
    state3=$(get_state)
    
    if [[ "$state1" == "$state2" ]] && [[ "$state2" == "$state3" ]]; then
        pass "State consistent across multiple reads"
    else
        fail "State inconsistent: $state1, $state2, $state3"
    fi
}

# Test 3: Kill switch state persists
test_ks_state_persistence() {
    "$SHROUD_BIN" ks on 2>/dev/null || true
    sleep 0.3
    
    local ks1 ks2
    ks1=$("$SHROUD_BIN" ks status 2>&1 | grep -i enabled || echo "")
    ks2=$("$SHROUD_BIN" ks status 2>&1 | grep -i enabled || echo "")
    
    if [[ -n "$ks1" ]] && [[ "$ks1" == "$ks2" ]]; then
        pass "Kill switch state persists"
    else
        fail "Kill switch state inconsistent"
    fi
    
    "$SHROUD_BIN" ks off 2>/dev/null || true
}

# Test 4: State after rapid operations
test_state_after_stress() {
    for _ in {1..10}; do
        "$SHROUD_BIN" ks toggle 2>/dev/null || true
    done
    
    # Should end in same state (even number of toggles)
    local state
    state=$(get_state)
    
    if [[ "$state" != "unknown" ]] && [[ "$state" != "" ]]; then
        pass "State valid after stress: $state"
    else
        fail "State corrupted after stress"
    fi
}

# Test 5: Status always returns valid JSON
test_json_validity() {
    local i
    for i in {1..5}; do
        local json
        json=$("$SHROUD_BIN" status --json 2>&1)
        
        # Check it starts with { and ends with }
        if [[ "$json" != "{"* ]] || [[ "$json" != *"}" ]]; then
            if [[ "$json" != "null" ]]; then
                fail "Invalid JSON at iteration $i: $json"
                return
            fi
        fi
    done
    
    pass "JSON output always valid"
}

# Start and run tests
if start_daemon; then
    sleep 1
    
    test_initial_state
    test_state_persistence
    test_ks_state_persistence
    test_state_after_stress
    test_json_validity
else
    fail "Could not start daemon"
fi

"$SHROUD_BIN" quit 2>/dev/null || true

echo ""
echo "State Consistency: $PASSED passed, $FAILED failed"

[[ $FAILED -eq 0 ]]
