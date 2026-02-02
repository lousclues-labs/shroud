#!/usr/bin/env bash
#
# Test: Cleanup Verification (Privileged)
#
# Verifies that all resources are cleaned up when daemon exits.

set -euo pipefail

SHROUD_BIN="${SHROUD_BIN:-./target/release/shroud}"

PASSED=0
FAILED=0

pass() { echo "  ✓ $1"; PASSED=$((PASSED + 1)); }
fail() { echo "  ✗ $1"; FAILED=$((FAILED + 1)); }

if [[ $EUID -ne 0 ]]; then
    echo "This test requires root. Run with sudo."
    exit 1
fi

cleanup_all() {
    pkill -f shroud 2>/dev/null || true
    sleep 1
    iptables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
    iptables -F SHROUD_KILLSWITCH 2>/dev/null || true
    iptables -X SHROUD_KILLSWITCH 2>/dev/null || true
    iptables -D OUTPUT -j SHROUD_BOOT_KS 2>/dev/null || true
    iptables -F SHROUD_BOOT_KS 2>/dev/null || true
    iptables -X SHROUD_BOOT_KS 2>/dev/null || true
    rm -f "${XDG_RUNTIME_DIR:-/tmp}/shroud.sock" 2>/dev/null || true
}

echo "=== Cleanup Verification Tests ==="
echo ""

# Clean slate
cleanup_all

# Test 1: Graceful quit cleans up
test_graceful_quit() {
    "$SHROUD_BIN" --desktop 2>/dev/null &
    local pid=$!
    sleep 2
    
    "$SHROUD_BIN" ks on 2>/dev/null || true
    sleep 0.5
    
    "$SHROUD_BIN" quit 2>/dev/null || true
    sleep 2
    
    # Check cleanup
    if iptables -L SHROUD_KILLSWITCH -n >/dev/null 2>&1; then
        fail "Graceful quit did not clean iptables"
    elif [[ -S "${XDG_RUNTIME_DIR:-/tmp}/shroud.sock" ]]; then
        fail "Graceful quit did not remove socket"
    else
        pass "Graceful quit cleaned up all resources"
    fi
    
    cleanup_all
}

# Test 2: SIGTERM cleans up
test_sigterm_cleanup() {
    "$SHROUD_BIN" --desktop 2>/dev/null &
    local pid=$!
    sleep 2
    
    "$SHROUD_BIN" ks on 2>/dev/null || true
    sleep 0.5
    
    kill -TERM "$pid" 2>/dev/null || true
    sleep 2
    
    if iptables -L SHROUD_KILLSWITCH -n >/dev/null 2>&1; then
        fail "SIGTERM did not clean iptables"
    else
        pass "SIGTERM cleanup successful"
    fi
    
    cleanup_all
}

# Test 3: SIGINT cleans up
test_sigint_cleanup() {
    "$SHROUD_BIN" --desktop 2>/dev/null &
    local pid=$!
    sleep 2
    
    "$SHROUD_BIN" ks on 2>/dev/null || true
    sleep 0.5
    
    kill -INT "$pid" 2>/dev/null || true
    sleep 2
    
    if iptables -L SHROUD_KILLSWITCH -n >/dev/null 2>&1; then
        fail "SIGINT did not clean iptables"
    else
        pass "SIGINT cleanup successful"
    fi
    
    cleanup_all
}

# Test 4: No orphan processes
test_no_orphans() {
    "$SHROUD_BIN" --desktop 2>/dev/null &
    sleep 2
    "$SHROUD_BIN" quit 2>/dev/null || true
    sleep 2
    
    if pgrep -f "shroud" >/dev/null 2>&1; then
        fail "Orphan shroud processes remain"
    else
        pass "No orphan processes"
    fi
}

# Run tests
test_graceful_quit
test_sigterm_cleanup
test_sigint_cleanup
test_no_orphans

echo ""
echo "Cleanup Tests: $PASSED passed, $FAILED failed"

[[ $FAILED -eq 0 ]]
