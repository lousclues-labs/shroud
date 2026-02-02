#!/usr/bin/env bash
#
# Test: Kill Switch Control (Privileged)
#
# Verifies kill switch enable/disable creates and removes iptables rules.
# Requires root privileges.

set -euo pipefail

SHROUD_BIN="${SHROUD_BIN:-./target/release/shroud}"

PASSED=0
FAILED=0
DAEMON_PID=""

pass() { echo "  ✓ $1"; PASSED=$((PASSED + 1)); }
fail() { echo "  ✗ $1"; FAILED=$((FAILED + 1)); }

cleanup() {
    [[ -n "$DAEMON_PID" ]] && kill "$DAEMON_PID" 2>/dev/null || true
    # Clean up any leftover rules
    iptables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
    iptables -F SHROUD_KILLSWITCH 2>/dev/null || true
    iptables -X SHROUD_KILLSWITCH 2>/dev/null || true
}
trap cleanup EXIT

# Check for root
if [[ $EUID -ne 0 ]]; then
    echo "This test requires root. Run with sudo."
    exit 1
fi

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

chain_exists() {
    iptables -L SHROUD_KILLSWITCH -n >/dev/null 2>&1
}

echo "=== Kill Switch Tests (Privileged) ==="
echo ""

# Test 1: Enable creates chain
test_enable_creates_chain() {
    "$SHROUD_BIN" ks off 2>/dev/null || true
    sleep 0.3
    
    "$SHROUD_BIN" ks on 2>/dev/null || true
    sleep 0.5
    
    if chain_exists; then
        pass "Kill switch enable creates iptables chain"
    else
        fail "Kill switch enable did not create chain"
    fi
}

# Test 2: Disable removes chain
test_disable_removes_chain() {
    "$SHROUD_BIN" ks on 2>/dev/null || true
    sleep 0.3
    
    "$SHROUD_BIN" ks off 2>/dev/null || true
    sleep 0.5
    
    if ! chain_exists; then
        pass "Kill switch disable removes iptables chain"
    else
        fail "Kill switch disable did not remove chain"
    fi
}

# Test 3: Chain has DROP rule
test_chain_has_drop() {
    "$SHROUD_BIN" ks on 2>/dev/null || true
    sleep 0.3
    
    if iptables -L SHROUD_KILLSWITCH -n 2>/dev/null | grep -q "DROP"; then
        pass "Kill switch chain has DROP rule"
    else
        fail "Kill switch chain missing DROP rule"
    fi
    
    "$SHROUD_BIN" ks off 2>/dev/null || true
}

# Test 4: Toggle works correctly
test_toggle() {
    "$SHROUD_BIN" ks off 2>/dev/null || true
    sleep 0.3
    
    # Toggle on
    "$SHROUD_BIN" ks toggle 2>/dev/null || true
    sleep 0.3
    
    if ! chain_exists; then
        fail "Toggle did not enable kill switch"
        return
    fi
    
    # Toggle off
    "$SHROUD_BIN" ks toggle 2>/dev/null || true
    sleep 0.3
    
    if chain_exists; then
        fail "Toggle did not disable kill switch"
    else
        pass "Kill switch toggle works"
    fi
}

# Test 5: Rapid toggle doesn't leave stale rules
test_rapid_toggle_cleanup() {
    for _ in {1..5}; do
        "$SHROUD_BIN" ks on 2>/dev/null || true
        "$SHROUD_BIN" ks off 2>/dev/null || true
    done
    sleep 0.5
    
    # Should not have chain
    if ! chain_exists; then
        pass "No stale rules after rapid toggles"
    else
        fail "Stale rules remain after rapid toggles"
    fi
}

# Test 6: No boot kill switch in desktop mode
test_no_boot_killswitch() {
    if iptables -L SHROUD_BOOT_KS -n >/dev/null 2>&1; then
        fail "Boot kill switch exists in desktop mode (should only be headless)"
    else
        pass "No boot kill switch in desktop mode"
    fi
}

# Start and run tests
if start_daemon; then
    sleep 1
    
    test_enable_creates_chain
    test_disable_removes_chain
    test_chain_has_drop
    test_toggle
    test_rapid_toggle_cleanup
    test_no_boot_killswitch
else
    fail "Could not start daemon"
fi

# Clean shutdown
"$SHROUD_BIN" ks off 2>/dev/null || true
"$SHROUD_BIN" quit 2>/dev/null || true
sleep 1

# Test 7: Rules cleaned up after quit
if ! chain_exists; then
    pass "Rules cleaned up after daemon quit"
else
    fail "Rules remain after daemon quit"
fi

echo ""
echo "Kill Switch Tests: $PASSED passed, $FAILED failed"

[[ $FAILED -eq 0 ]]
