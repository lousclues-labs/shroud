#!/bin/bash
# Scenario: Connect to non-existent VPN
# Safety: 🟢 SAFE - Tests error handling
#
# EXPERIMENT PLAN:
#   Trigger: Try to connect to VPN names that don't exist
#   Duration: 30 seconds
#   Observe: Clear errors? No crashes? State stays consistent?
#
# EXPECTED BEHAVIOR:
#   - Clear "VPN not found" error
#   - State remains Disconnected
#   - No partial state changes

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: Connect to Non-Existent VPN"
echo "  SAFETY: 🟢 SAFE"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

# Start shroud
echo "[T+0s] Starting shroud..."
shroud &
SHROUD_PID=$!
sleep 3

echo "[T+3s] Baseline:"
shroud status || true
shroud list || true
echo ""

# Test various invalid names
echo "═══════════════════════════════════════════════════════════════"
echo "Testing invalid VPN names:"
echo "═══════════════════════════════════════════════════════════════"

TESTS=(
    "nonexistent-vpn-12345"
    ""
    " "
    "../../etc/passwd"
    "vpn; rm -rf /"
    "$(printf 'A%.0s' {1..500})"
    "vpn with spaces"
    "vpn\twith\ttabs"
    "vpn\nwith\nnewlines"
)

for test_name in "${TESTS[@]}"; do
    echo ""
    echo "--- Testing: '${test_name:0:50}...' ---"
    
    # Record state before
    STATE_BEFORE=$(shroud status 2>&1 | head -1 || echo "unknown")
    
    # Try to connect
    shroud connect "$test_name" 2>&1 || echo "(connect failed - expected)"
    
    # Check state after
    STATE_AFTER=$(shroud status 2>&1 | head -1 || echo "unknown")
    
    echo "State: $STATE_BEFORE -> $STATE_AFTER"
    
    if [[ "$STATE_BEFORE" != "$STATE_AFTER" ]]; then
        echo "⚠ State changed unexpectedly!"
    else
        echo "✓ State unchanged"
    fi
done

# Check daemon still running
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "STABILITY CHECK"
echo "═══════════════════════════════════════════════════════════════"
if kill -0 $SHROUD_PID 2>/dev/null; then
    echo "✓ Shroud still running"
    shroud status || true
else
    echo "✗ Shroud crashed!"
fi

# Cleanup
kill $SHROUD_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"
