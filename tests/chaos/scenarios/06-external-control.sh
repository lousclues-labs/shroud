#!/bin/bash
# Scenario: Use nm-applet while shroud is running
# Safety: 🟢 SAFE - Tests state sync between shroud and external tools
#
# EXPERIMENT PLAN:
#   Trigger: Start shroud, connect via shroud, then disconnect via nmcli
#   Duration: 1 minute
#   Observe: Does shroud detect the external disconnect? Does state sync?
#
# EXPECTED BEHAVIOR:
#   - Shroud should detect VPN disconnect via D-Bus events
#   - State should sync to "Disconnected"
#   - Kill switch should respond appropriately

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: External VPN Control (nm-applet simulation)"
echo "  SAFETY: 🟢 SAFE"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

# Get first VPN
VPN=$(nmcli -t -f NAME,TYPE con show | grep vpn | head -1 | cut -d: -f1)
if [[ -z "$VPN" ]]; then
    echo "✗ No VPN configured"
    exit 1
fi
echo "Using VPN: $VPN"
echo ""

# Start shroud
echo "[T+0s] Starting shroud..."
shroud &
SHROUD_PID=$!
sleep 3

# Connect via shroud
echo "[T+3s] Connecting via shroud..."
shroud connect "$VPN"
sleep 5

echo "[T+8s] Shroud status:"
shroud status || true
echo ""

# Verify connected
VPN_IP=$(curl -s --max-time 5 ifconfig.me || echo "FAILED")
echo "VPN IP: $VPN_IP"
echo ""

# Disconnect via nmcli (simulating nm-applet)
echo "═══════════════════════════════════════════════════════════════"
echo "[T+10s] Disconnecting via nmcli (external control)..."
echo "═══════════════════════════════════════════════════════════════"
nmcli con down "$VPN" 2>&1 || echo "(already disconnected)"

# Wait for shroud to notice
echo ""
echo "[T+12s] Waiting for shroud to detect disconnect..."
sleep 3

echo "[T+15s] Shroud status after external disconnect:"
shroud status || true
echo ""

# Check if state synced
AFTER_IP=$(curl -s --max-time 5 ifconfig.me || echo "FAILED")
echo "IP after external disconnect: $AFTER_IP"

# Now connect via nmcli, see if shroud notices
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "[T+20s] Connecting via nmcli (external control)..."
echo "═══════════════════════════════════════════════════════════════"
nmcli con up "$VPN" 2>&1 || echo "(connection failed)"
sleep 5

echo "[T+25s] Shroud status after external connect:"
shroud status || true
echo ""

EXTERNAL_IP=$(curl -s --max-time 5 ifconfig.me || echo "FAILED")
echo "IP after external connect: $EXTERNAL_IP"

# Cleanup
nmcli con down "$VPN" 2>/dev/null || true
kill $SHROUD_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "VPN IP (connected via shroud):   $VPN_IP"
echo "IP after external disconnect:    $AFTER_IP"
echo "IP after external connect:       $EXTERNAL_IP"
echo ""
echo "Key Questions:"
echo "- Did shroud detect external disconnect? (check status output)"
echo "- Did shroud detect external connect? (check status output)"
echo "- Was there any state confusion or errors?"
