#!/bin/bash
# Scenario: Start multiple shroud instances
# Safety: 🟢 SAFE - Tests daemon lock mechanism
#
# EXPERIMENT PLAN:
#   Trigger: Start shroud, then try to start another instance
#   Duration: 30 seconds
#   Observe: Does second instance fail gracefully? Any resource conflicts?
#
# EXPECTED BEHAVIOR:
#   - Second instance should detect lock and exit with clear error
#   - First instance should continue running unaffected

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: Multiple Shroud Instances"
echo "  SAFETY: 🟢 SAFE"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

# Start first instance
echo "[T+0s] Starting first shroud instance..."
shroud &
PID1=$!
sleep 3

echo "[T+3s] First instance running, PID: $PID1"
shroud status || true
echo ""

# Try to start second instance
echo "═══════════════════════════════════════════════════════════════"
echo "[T+5s] Attempting to start second instance..."
echo "═══════════════════════════════════════════════════════════════"

# Capture output
SECOND_OUTPUT=$(shroud 2>&1 &
    SECOND_PID=$!
    sleep 2
    if kill -0 $SECOND_PID 2>/dev/null; then
        echo "SECOND_RUNNING"
        kill $SECOND_PID 2>/dev/null
    else
        wait $SECOND_PID 2>/dev/null
        echo "EXIT_CODE: $?"
    fi
)

echo "Second instance output:"
echo "$SECOND_OUTPUT"
echo ""

# Check if first instance still works
echo "[T+10s] First instance still functional?"
if kill -0 $PID1 2>/dev/null; then
    echo "✓ First instance still running"
    shroud status || true
else
    echo "✗ First instance died!"
fi

# Try CLI commands with multiple instances
echo ""
echo "[T+12s] Testing CLI with running daemon..."
shroud ping 2>&1 || echo "(ping failed)"

# Cleanup
kill $PID1 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Expected: Second instance should fail with 'already running' error"
echo "Check output above for proper lock detection."
