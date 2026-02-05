#!/bin/bash
# Scenario: Simulate network hiccup with tc (traffic control)
# Safety: 🟡 CAUTION - Temporarily disrupts network
#
# EXPERIMENT PLAN:
#   Trigger: Connect to VPN, then use tc to add latency/packet loss
#   Duration: 2 minutes
#   Observe: Does shroud detect degraded state? Does it recover when fixed?
#
# EXPECTED BEHAVIOR:
#   - Shroud should detect degraded state (high latency)
#   - Should NOT disconnect just because of latency
#   - Should recover cleanly when network stabilizes

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: Network Hiccup (tc simulation)"
echo "  SAFETY: 🟡 CAUTION - Temporary network disruption"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

# Find active interface
IFACE=$(ip route | grep default | awk '{print $5}' | head -1)
if [[ -z "$IFACE" ]]; then
    echo "✗ Could not detect network interface"
    exit 1
fi
echo "Using interface: $IFACE"

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

# Enable debug logging
shroud debug on 2>/dev/null || true

# Connect VPN
echo "[T+3s] Connecting to VPN..."
shroud connect "$VPN"
sleep 5

echo "[T+8s] Baseline status:"
shroud status || true
BASELINE_IP=$(curl -s --max-time 5 ifconfig.me || echo "FAILED")
echo "Baseline IP: $BASELINE_IP"
echo ""

# Add network degradation
echo "═══════════════════════════════════════════════════════════════"
echo "[T+10s] Adding 2000ms latency + 30% packet loss..."
echo "═══════════════════════════════════════════════════════════════"
sudo tc qdisc add dev "$IFACE" root netem delay 2000ms loss 30%
echo "✓ Traffic shaping applied"
echo ""

# Wait for detection
echo "[T+12s] Waiting for shroud to detect degraded state..."
for i in {1..6}; do
    sleep 5
    echo ""
    echo "[T+$((12 + i*5))s] Status check $i:"
    shroud status 2>/dev/null || echo "(status check failed - expected with packet loss)"
done

# Check if degraded was detected
echo ""
echo "[T+42s] Checking logs for degraded detection..."
shroud debug dump 2>/dev/null | tail -20 | grep -i "degrad\|latency\|health" || echo "(no degraded entries found)"

# Remove degradation
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "[T+45s] Removing network degradation..."
echo "═══════════════════════════════════════════════════════════════"
sudo tc qdisc del dev "$IFACE" root 2>/dev/null || true
echo "✓ Traffic shaping removed"
echo ""

# Wait for recovery
echo "[T+47s] Waiting for recovery..."
sleep 10

echo "[T+57s] Post-recovery status:"
shroud status || true
RECOVERY_IP=$(curl -s --max-time 5 ifconfig.me || echo "FAILED")
echo "Recovery IP: $RECOVERY_IP"

# Cleanup
kill $SHROUD_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Baseline IP:  $BASELINE_IP"
echo "Recovery IP:  $RECOVERY_IP"
echo ""
if [[ "$BASELINE_IP" == "$RECOVERY_IP" && "$RECOVERY_IP" != "FAILED" ]]; then
    echo "✓ VPN connection maintained through network hiccup"
else
    echo "? Connection state changed - check logs"
fi
