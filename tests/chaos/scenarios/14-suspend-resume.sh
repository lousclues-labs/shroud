#!/bin/bash
# Scenario: Suspend/Resume (simulated with SIGSTOP/SIGCONT)
# Safety: 🟡 CAUTION - VPN will disconnect during "suspend"
#
# EXPERIMENT PLAN:
#   Trigger: Connect to VPN, SIGSTOP shroud for 10s, SIGCONT
#   Duration: 1 minute
#   Observe: Does shroud recover? State sync? Kill switch?
#
# NOTE: Real suspend/resume would be more thorough but requires
#       physical or VM control. This simulates process freeze.
#
# EXPECTED BEHAVIOR:
#   - VPN may disconnect during freeze (NM continues)
#   - On resume, shroud should detect state change
#   - Should sync state with reality

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: Suspend/Resume Simulation (SIGSTOP/SIGCONT)"
echo "  SAFETY: 🟡 CAUTION"
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

# Start shroud
echo "[T+0s] Starting shroud with kill switch..."
shroud &
SHROUD_PID=$!
sleep 3

# Enable kill switch and connect
shroud ks on
shroud connect "$VPN"
sleep 5

echo "[T+8s] Baseline:"
shroud status || true
BASELINE_IP=$(curl -s --max-time 5 ifconfig.me || echo "FAILED")
echo "VPN IP: $BASELINE_IP"
echo ""

# Freeze shroud
echo "═══════════════════════════════════════════════════════════════"
echo "[T+10s] Freezing shroud (SIGSTOP)..."
echo "═══════════════════════════════════════════════════════════════"
kill -STOP $SHROUD_PID
echo "Shroud frozen."

# Wait (simulating suspend duration)
echo ""
echo "Simulating 10 second suspend..."
for i in {1..10}; do
    echo -n "."
    sleep 1
done
echo ""

# Check VPN state while frozen (via nmcli)
echo ""
echo "[T+20s] VPN state while shroud frozen:"
nmcli con show --active | grep vpn || echo "(no active VPN)"

# Check if traffic still works
FROZEN_IP=$(curl -s --max-time 5 ifconfig.me || echo "BLOCKED")
echo "IP during freeze: $FROZEN_IP"

# Unfreeze
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "[T+22s] Resuming shroud (SIGCONT)..."
echo "═══════════════════════════════════════════════════════════════"
kill -CONT $SHROUD_PID
echo "Shroud resumed."
sleep 5

# Check recovery
echo ""
echo "[T+27s] After resume:"
shroud status 2>&1 || echo "(status failed)"
RESUME_IP=$(curl -s --max-time 5 ifconfig.me || echo "FAILED")
echo "IP after resume: $RESUME_IP"

# Give time for state sync
echo ""
echo "[T+30s] After state sync (5s wait):"
sleep 5
shroud status || true

# Cleanup
shroud ks off 2>/dev/null || true
kill $SHROUD_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Baseline IP:    $BASELINE_IP"
echo "Frozen IP:      $FROZEN_IP"
echo "Resume IP:      $RESUME_IP"
echo ""
echo "Key questions:"
echo "- Did VPN persist during freeze?"
echo "- Did kill switch protect if VPN dropped?"
echo "- Did shroud correctly sync state on resume?"
