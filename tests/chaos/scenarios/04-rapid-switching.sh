#!/bin/bash
# Scenario: Rapidly switch between VPN servers
# Safety: 🟢 SAFE - Normal operations, just fast
#
# EXPERIMENT PLAN:
#   Trigger: Connect/switch between 5 VPN servers as fast as possible
#   Duration: 1 minute
#   Observe: Race conditions? State confusion? Leaked traffic between switches?
#
# EXPECTED BEHAVIOR:
#   - Each switch should be atomic (disconnect old, connect new)
#   - Kill switch should protect during transitions
#   - State should always be accurate

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: Rapid VPN Server Switching"
echo "  SAFETY: 🟢 SAFE"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

# Get all VPNs
VPNS=$(nmcli -t -f NAME,TYPE con show | grep vpn | cut -d: -f1)
VPN_COUNT=$(echo "$VPNS" | wc -l)

if [[ $VPN_COUNT -lt 2 ]]; then
    echo "✗ Need at least 2 VPN connections configured"
    echo "  Found: $VPN_COUNT"
    exit 1
fi
echo "Found $VPN_COUNT VPNs:"
echo "$VPNS"
echo ""

# Start shroud
echo "[T+0s] Starting shroud with kill switch..."
shroud &
SHROUD_PID=$!
sleep 3

# Enable kill switch for leak protection
shroud ks on
sleep 1

# Get baseline
REAL_IP=$(curl -s --max-time 5 ifconfig.me || echo "BLOCKED")
echo "Real IP (with KS, no VPN): $REAL_IP"
echo ""

# Rapid switching
echo "═══════════════════════════════════════════════════════════════"
echo "[T+5s] Starting rapid switching..."
echo "═══════════════════════════════════════════════════════════════"

SWITCH_COUNT=0
LEAK_COUNT=0
TRANSITION_LOG=""

for round in {1..3}; do
    echo ""
    echo "--- Round $round ---"
    
    while IFS= read -r vpn; do
        [[ -z "$vpn" ]] && continue
        
        SWITCH_COUNT=$((SWITCH_COUNT + 1))
        START_TIME=$(date +%s.%N)
        
        echo -n "Switch $SWITCH_COUNT to '$vpn'..."
        
        # Switch
        shroud switch "$vpn" 2>/dev/null &
        SWITCH_PID=$!
        
        # Don't wait for completion - that's the chaos
        sleep 0.5
        
        # Quick leak check during transition
        TRANS_IP=$(timeout 2 curl -s ifconfig.me 2>/dev/null || echo "BLOCKED")
        
        # Wait for switch to complete
        wait $SWITCH_PID 2>/dev/null || true
        
        END_TIME=$(date +%s.%N)
        DURATION=$(echo "$END_TIME - $START_TIME" | bc 2>/dev/null || echo "?")
        
        # Get final state
        FINAL_IP=$(curl -s --max-time 3 ifconfig.me || echo "TIMEOUT")
        
        echo " done (${DURATION}s)"
        echo "  During: $TRANS_IP | After: $FINAL_IP"
        
        # Check for real IP leak
        if [[ "$TRANS_IP" == "$REAL_IP" && "$REAL_IP" != "BLOCKED" ]]; then
            echo "  ⚠ POSSIBLE LEAK during transition!"
            LEAK_COUNT=$((LEAK_COUNT + 1))
        fi
        
        TRANSITION_LOG="$TRANSITION_LOG\n$SWITCH_COUNT,$vpn,$TRANS_IP,$FINAL_IP"
        
    done <<< "$VPNS"
done

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "[T+45s] Rapid switching complete"
echo "═══════════════════════════════════════════════════════════════"

# Final state check
echo ""
echo "Final state:"
shroud status || true

# Cleanup
shroud ks off 2>/dev/null || true
kill $SHROUD_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Total switches: $SWITCH_COUNT"
echo "Possible leaks: $LEAK_COUNT"
echo ""
if [[ $LEAK_COUNT -eq 0 ]]; then
    echo "✓ No leaks detected during rapid switching"
else
    echo "✗ $LEAK_COUNT potential leak(s) during transitions"
    echo "  (Real IP appeared during switch)"
fi
