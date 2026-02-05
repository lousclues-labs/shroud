#!/bin/bash
# Scenario: Fill /tmp while daemon runs
# Safety: 🔴 DANGEROUS - May affect system stability. VM only.
#
# EXPERIMENT PLAN:
#   Trigger: Start shroud, fill /tmp until full, observe behavior
#   Duration: 1 minute
#   Observe: Does shroud handle disk full gracefully? Logs? IPC?
#
# EXPECTED BEHAVIOR:
#   - Shroud should continue running
#   - Log writes may fail but shouldn't crash daemon
#   - IPC should still work (socket-based)

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: Fill /tmp Disk Space"
echo "  SAFETY: 🔴 DANGEROUS - VM only"
echo "═══════════════════════════════════════════════════════════════"
echo ""
echo "This will temporarily fill /tmp. Requires ~1GB free space."
read -p "Continue? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 0
fi

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

# Check /tmp space
TMP_FREE=$(df /tmp | tail -1 | awk '{print $4}')
echo "Free space in /tmp: ${TMP_FREE}K"

if [[ $TMP_FREE -lt 102400 ]]; then  # 100MB minimum
    echo "✗ Not enough space in /tmp for test"
    exit 1
fi

# Start shroud
echo "[T+0s] Starting shroud..."
shroud &
SHROUD_PID=$!
sleep 3

echo "[T+3s] Baseline:"
shroud status || true
echo ""

# Create fill files
echo "═══════════════════════════════════════════════════════════════"
echo "[T+5s] Filling /tmp..."
echo "═══════════════════════════════════════════════════════════════"

FILL_DIR="/tmp/shroud-chaos-fill-$$"
mkdir -p "$FILL_DIR"

# Fill in 100MB chunks until full
CHUNKS=0
while dd if=/dev/zero of="$FILL_DIR/fill-$CHUNKS" bs=1M count=100 2>/dev/null; do
    CHUNKS=$((CHUNKS + 1))
    echo "  Created chunk $CHUNKS (${CHUNKS}00 MB total)"
    
    # Check shroud after each chunk
    if ! kill -0 $SHROUD_PID 2>/dev/null; then
        echo "✗ Shroud died during fill!"
        break
    fi
    
    # Stop at 10 chunks (1GB) or when full
    if [[ $CHUNKS -ge 10 ]]; then
        echo "  (stopping at 1GB for safety)"
        break
    fi
done

echo ""
echo "[T+30s] /tmp filled. Checking shroud..."

# Test shroud operations
echo ""
echo "Testing operations with /tmp full:"
echo "  status:"
shroud status 2>&1 | head -5 || echo "  (status failed)"

echo "  ping:"
shroud ping 2>&1 || echo "  (ping failed)"

echo "  debug log:"
shroud debug tail 2>&1 | head -3 || echo "  (log access failed)"

# Check if daemon still responsive
if kill -0 $SHROUD_PID 2>/dev/null; then
    echo ""
    echo "✓ Shroud still running with /tmp full"
else
    echo ""
    echo "✗ Shroud crashed with /tmp full"
fi

# Cleanup fill files FIRST
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "[T+35s] Cleaning fill files..."
echo "═══════════════════════════════════════════════════════════════"
rm -rf "$FILL_DIR"
echo "✓ Fill files removed"

# Check recovery
echo ""
echo "[T+40s] Post-cleanup check:"
shroud status 2>&1 || echo "(status after cleanup)"

# Cleanup
kill $SHROUD_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Filled $CHUNKS chunks (${CHUNKS}00 MB)"
echo "Check output above for:"
echo "- Did shroud survive disk full condition?"
echo "- Were operations still responsive?"
echo "- Did it recover after space freed?"
