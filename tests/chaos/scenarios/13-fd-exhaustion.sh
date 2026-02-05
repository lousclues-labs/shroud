#!/bin/bash
# Scenario: Exhaust file descriptors
# Safety: 🔴 DANGEROUS - May affect system stability. VM only.
#
# EXPERIMENT PLAN:
#   Trigger: Start shroud, then exhaust available file descriptors
#   Duration: 1 minute
#   Observe: Does shroud handle FD exhaustion? IPC? Recovery?
#
# EXPECTED BEHAVIOR:
#   - New connections should fail gracefully
#   - Existing connections should persist
#   - Recovery after FD release

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: File Descriptor Exhaustion"
echo "  SAFETY: 🔴 DANGEROUS - VM only"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

# Check current limits
echo "Current FD limits:"
ulimit -n
echo "Open FDs:"
ls /proc/$$/fd | wc -l
echo ""

# Start shroud
echo "[T+0s] Starting shroud..."
shroud &
SHROUD_PID=$!
sleep 3

echo "[T+3s] Shroud FDs before exhaust:"
ls /proc/$SHROUD_PID/fd 2>/dev/null | wc -l || echo "(cannot read)"
shroud status || true
echo ""

# Exhaust FDs in a subshell
echo "═══════════════════════════════════════════════════════════════"
echo "[T+5s] Exhausting file descriptors..."
echo "═══════════════════════════════════════════════════════════════"

# Create a bunch of open files
FD_DIR="/tmp/shroud-fd-exhaust-$$"
mkdir -p "$FD_DIR"

# Get soft limit
LIMIT=$(ulimit -n)
echo "Soft limit: $LIMIT"

# Open files until we can't
exec 3>/dev/null  # Reserve one for script use

# This script opens FDs which affects the current shell, not shroud
# Instead, let's create pressure on the system
echo "Creating open file pressure..."

PIDS=""
for i in {1..50}; do
    (
        # Each subshell opens many files
        for j in {1..100}; do
            exec {fd}>"$FD_DIR/file-$i-$j" 2>/dev/null || true
        done
        sleep 30
    ) &
    PIDS="$PIDS $!"
done

echo "Created 50 processes holding ~5000 FDs"
sleep 2

# Test shroud under pressure
echo ""
echo "[T+10s] Testing shroud under FD pressure:"
echo "  Shroud FDs:"
ls /proc/$SHROUD_PID/fd 2>/dev/null | wc -l || echo "(cannot read)"

echo "  status:"
shroud status 2>&1 || echo "(status failed)"

echo "  ping:"
shroud ping 2>&1 || echo "(ping failed)"

# Try to connect VPN
VPN=$(nmcli -t -f NAME,TYPE con show | grep vpn | head -1 | cut -d: -f1)
if [[ -n "$VPN" ]]; then
    echo "  connect:"
    shroud connect "$VPN" 2>&1 || echo "(connect failed)"
fi

# Check if daemon alive
echo ""
if kill -0 $SHROUD_PID 2>/dev/null; then
    echo "✓ Shroud still running under FD pressure"
else
    echo "✗ Shroud died under FD pressure"
fi

# Release pressure
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "[T+20s] Releasing FD pressure..."
echo "═══════════════════════════════════════════════════════════════"
for pid in $PIDS; do
    kill $pid 2>/dev/null || true
done
rm -rf "$FD_DIR"
sleep 2

# Test recovery
echo ""
echo "[T+25s] Testing after pressure released:"
shroud status 2>&1 || echo "(status failed)"
shroud ping 2>&1 || echo "(ping failed)"

# Cleanup
kill $SHROUD_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Check output above for:"
echo "- Did shroud survive FD pressure?"
echo "- Were operations responsive under pressure?"
echo "- Did it recover after pressure released?"
