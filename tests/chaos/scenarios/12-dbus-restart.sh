#!/bin/bash
# Scenario: D-Bus restart
# Safety: 🔴 DANGEROUS - May affect running applications. VM only.
#
# EXPERIMENT PLAN:
#   Trigger: Start shroud, connect, restart D-Bus daemon
#   Duration: 1 minute
#   Observe: Does shroud handle D-Bus reconnection? Errors?
#
# EXPECTED BEHAVIOR:
#   - Shroud should detect D-Bus disconnect
#   - Should attempt to reconnect or fail gracefully
#   - VPN connection managed by NM should persist

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: D-Bus Restart"
echo "  SAFETY: 🔴 DANGEROUS - VM only"
echo "═══════════════════════════════════════════════════════════════"
echo ""
echo "This will restart the D-Bus user session daemon."
echo "This may cause issues with running applications."
read -p "Continue? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 0
fi

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

# Get first VPN
VPN=$(nmcli -t -f NAME,TYPE con show | grep vpn | head -1 | cut -d: -f1)
if [[ -z "$VPN" ]]; then
    echo "✗ No VPN configured"
    exit 1
fi

# Start shroud
echo "[T+0s] Starting shroud..."
shroud &
SHROUD_PID=$!
sleep 3

# Connect
echo "[T+3s] Connecting to VPN..."
shroud connect "$VPN"
sleep 5

echo "[T+8s] Baseline:"
shroud status || true
echo ""

# Restart D-Bus session
echo "═══════════════════════════════════════════════════════════════"
echo "[T+10s] Restarting D-Bus session bus..."
echo "═══════════════════════════════════════════════════════════════"

# Kill the session bus
DBUS_PID=$(pgrep -f "dbus-daemon.*session" | head -1 || echo "")
if [[ -n "$DBUS_PID" ]]; then
    echo "Killing D-Bus session daemon (PID: $DBUS_PID)"
    kill $DBUS_PID 2>/dev/null || true
    sleep 2
    
    # It should respawn, but let's check
    NEW_DBUS=$(pgrep -f "dbus-daemon.*session" | head -1 || echo "")
    if [[ -n "$NEW_DBUS" ]]; then
        echo "D-Bus respawned (PID: $NEW_DBUS)"
    else
        echo "⚠ D-Bus did not respawn - starting manually"
        dbus-daemon --session --fork
        sleep 1
    fi
else
    echo "Could not find D-Bus session daemon"
fi

# Check shroud
echo ""
echo "[T+15s] Checking shroud after D-Bus restart..."

if kill -0 $SHROUD_PID 2>/dev/null; then
    echo "✓ Shroud process still running"
else
    echo "✗ Shroud process died"
fi

# Try operations
echo ""
echo "Testing operations:"
shroud status 2>&1 || echo "(status failed)"
shroud ping 2>&1 || echo "(ping failed)"

# Check VPN
echo ""
echo "VPN state (via nmcli, not shroud):"
nmcli con show --active | grep vpn || echo "(no active VPN)"

# Cleanup
kill $SHROUD_PID 2>/dev/null || true
nmcli con down "$VPN" 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Check output above for:"
echo "- Did shroud survive D-Bus restart?"
echo "- Did VPN connection persist (managed by NM)?"
echo "- Were operations functional after restart?"
