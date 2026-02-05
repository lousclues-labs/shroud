#!/bin/bash
# Scenario: SIGKILL shroud while kill switch is active
# Safety: 🔴 DANGEROUS - May leave you without network. Run in VM.
# 
# EXPERIMENT PLAN:
#   Trigger: Enable kill switch, connect to VPN, then SIGKILL shroud
#   Duration: 2 minutes
#   Observe: Are rules orphaned? Can user recover? Does restart clean up?
#
# EXPECTED BEHAVIOR:
#   - Rules remain (SIGKILL bypasses cleanup)
#   - User has no internet
#   - Restarting shroud should detect stale rules and clean up OR reattach

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: SIGKILL with Kill Switch Active"
echo "  SAFETY: 🔴 DANGEROUS - Run in VM only"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight
source "$SCRIPT_DIR/pre-test.sh" 2>/dev/null || "$SCRIPT_DIR/pre-test.sh"

# Get first VPN
VPN=$(nmcli -t -f NAME,TYPE con show | grep vpn | head -1 | cut -d: -f1)
if [[ -z "$VPN" ]]; then
    echo "✗ No VPN configured. Import one first."
    exit 1
fi
echo "Using VPN: $VPN"
echo ""

# Start shroud
echo "[T+0s] Starting shroud..."
shroud &
SHROUD_PID=$!
sleep 3

# Enable kill switch
echo "[T+3s] Enabling kill switch..."
shroud ks on
sleep 2

# Verify kill switch
echo "[T+5s] Verifying kill switch rules..."
sudo iptables -L SHROUD_KILLSWITCH -n || echo "No rules found!"

# Connect VPN
echo "[T+5s] Connecting to VPN..."
shroud connect "$VPN"
sleep 5

# Verify connected
echo "[T+10s] Status:"
shroud status || true
echo ""

# Check IP through VPN
VPN_IP=$(curl -s --max-time 5 ifconfig.me || echo "BLOCKED")
echo "IP through VPN: $VPN_IP"

# THE KILL
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "[T+15s] SIGKILL shroud (simulating crash)..."
echo "═══════════════════════════════════════════════════════════════"
kill -9 $SHROUD_PID 2>/dev/null || true
sleep 2

# Observations
echo ""
echo "POST-KILL OBSERVATIONS:"
echo "------------------------"

# Check if rules remain
RULES=$(sudo iptables -L SHROUD_KILLSWITCH -n 2>&1 || echo "CHAIN GONE")
echo "Kill switch rules: "
echo "$RULES" | head -5

# Check if network is blocked
echo ""
echo "Network test (should be BLOCKED if kill switch orphaned):"
BLOCKED_IP=$(curl -s --max-time 5 ifconfig.me || echo "BLOCKED")
echo "  Result: $BLOCKED_IP"

# Try to restart shroud
echo ""
echo "[T+20s] Restarting shroud (should detect stale rules)..."
shroud &
NEW_PID=$!
sleep 3

# Check status
echo ""
echo "After restart:"
shroud status || true

# Check if rules were handled
RULES_AFTER=$(sudo iptables -L SHROUD_KILLSWITCH -n 2>&1 || echo "CHAIN GONE")
echo ""
echo "Kill switch rules after restart:"
echo "$RULES_AFTER" | head -5

# Can we get network back?
echo ""
echo "Network after restart (with ks still on):"
shroud ks off
sleep 1
RESTORED_IP=$(curl -s --max-time 5 ifconfig.me || echo "STILL BLOCKED")
echo "  Result: $RESTORED_IP"

# Cleanup
echo ""
kill $NEW_PID 2>/dev/null || true
"$SCRIPT_DIR/post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "VPN IP before kill:  $VPN_IP"
echo "IP after SIGKILL:    $BLOCKED_IP"
echo "IP after restart:    $RESTORED_IP"
echo ""
if [[ "$BLOCKED_IP" == "BLOCKED" ]]; then
    echo "✓ Kill switch correctly blocked traffic after crash"
else
    echo "✗ LEAK: Traffic escaped kill switch after crash!"
fi
if [[ "$RESTORED_IP" != "BLOCKED" && "$RESTORED_IP" != "STILL BLOCKED" ]]; then
    echo "✓ Network recovered after restart"
else
    echo "✗ Network NOT recovered - manual cleanup needed"
fi
