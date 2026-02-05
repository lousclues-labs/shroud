#!/bin/bash
# Scenario: NetworkManager crash/restart
# Safety: 🔴 DANGEROUS - Disrupts all network connections. VM only.
#
# EXPERIMENT PLAN:
#   Trigger: Connect to VPN, then restart NetworkManager service
#   Duration: 2 minutes
#   Observe: Does shroud detect NM restart? Does it recover? Kill switch state?
#
# EXPECTED BEHAVIOR:
#   - Shroud should detect VPN drop when NM restarts
#   - Kill switch should protect during outage
#   - After NM comes back, shroud should be able to reconnect

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: NetworkManager Crash/Restart"
echo "  SAFETY: 🔴 DANGEROUS - VM only"
echo "═══════════════════════════════════════════════════════════════"
echo ""
echo "This will restart NetworkManager, dropping all connections."
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
echo "Using VPN: $VPN"
echo ""

# Start shroud
echo "[T+0s] Starting shroud with kill switch..."
shroud &
SHROUD_PID=$!
sleep 3

# Enable kill switch
shroud ks on
sleep 1

# Connect VPN
echo "[T+5s] Connecting to VPN..."
shroud connect "$VPN"
sleep 5

echo "[T+10s] Baseline:"
shroud status || true
BASELINE_IP=$(curl -s --max-time 5 ifconfig.me || echo "FAILED")
echo "VPN IP: $BASELINE_IP"
echo ""

# THE CRASH
echo "═══════════════════════════════════════════════════════════════"
echo "[T+15s] Restarting NetworkManager..."
echo "═══════════════════════════════════════════════════════════════"

# Restart NM
sudo systemctl restart NetworkManager

echo "NM restarting..."
sleep 5

# Check shroud's reaction
echo ""
echo "[T+20s] Shroud state during NM outage:"
shroud status 2>&1 || echo "(status failed - expected)"

# Check kill switch
echo ""
echo "Kill switch rules during outage:"
sudo iptables -L SHROUD_KILLSWITCH -n 2>&1 | head -5 || echo "(no rules)"

# Check for leaks
echo ""
echo "Leak check during NM restart:"
OUTAGE_IP=$(curl -s --max-time 5 ifconfig.me || echo "BLOCKED")
echo "IP: $OUTAGE_IP"

# Wait for NM to fully recover
echo ""
echo "[T+25s] Waiting for NetworkManager to stabilize..."
sleep 10

# Check NM status
echo ""
echo "[T+35s] NetworkManager status:"
systemctl is-active NetworkManager || echo "NM not active!"

# Check shroud's recovery
echo ""
echo "[T+35s] Shroud state after NM recovery:"
shroud status 2>&1 || echo "(shroud may need time)"

# Try to reconnect
echo ""
echo "[T+40s] Attempting reconnection..."
shroud connect "$VPN" 2>&1 || echo "(reconnect may fail if NM still initializing)"
sleep 10

echo "[T+50s] Final state:"
shroud status || true
FINAL_IP=$(curl -s --max-time 5 ifconfig.me || echo "FAILED")
echo "Final IP: $FINAL_IP"

# Cleanup
shroud ks off 2>/dev/null || true
kill $SHROUD_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Baseline VPN IP:  $BASELINE_IP"
echo "IP during outage: $OUTAGE_IP"
echo "Final IP:         $FINAL_IP"
echo ""
if [[ "$OUTAGE_IP" == "BLOCKED" ]]; then
    echo "✓ Kill switch protected during NM restart"
else
    echo "✗ Traffic escaped during NM restart!"
fi
if [[ "$FINAL_IP" != "FAILED" && "$FINAL_IP" != "BLOCKED" ]]; then
    echo "✓ Recovered after NM restart"
else
    echo "⚠ Recovery may need manual intervention"
fi
