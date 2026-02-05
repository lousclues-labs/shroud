#!/bin/bash
# Scenario: Enable kill switch, forget, try to access LAN printer
# Safety: 🟢 SAFE - Tests LAN access behavior
#
# EXPERIMENT PLAN:
#   Trigger: Enable kill switch, try to access local network resources
#   Duration: 30 seconds
#   Observe: Is LAN accessible? Is config for LAN access honored?
#
# EXPECTED BEHAVIOR:
#   - With allow_lan=true: Local network should be accessible
#   - With allow_lan=false: Local network should be blocked

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: Kill Switch vs LAN Access"
echo "  SAFETY: 🟢 SAFE"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

# Find local network details
GATEWAY=$(ip route | grep default | awk '{print $3}')
LOCAL_NET=$(ip route | grep -E "^(192\.168|10\.|172\.)" | head -1 | awk '{print $1}')

echo "Network info:"
echo "  Gateway: $GATEWAY"
echo "  Local network: $LOCAL_NET"
echo ""

# Start shroud
echo "[T+0s] Starting shroud..."
shroud &
SHROUD_PID=$!
sleep 3

# Test without kill switch
echo "═══════════════════════════════════════════════════════════════"
echo "TEST 1: LAN access without kill switch"
echo "═══════════════════════════════════════════════════════════════"
echo "Pinging gateway..."
ping -c 2 -W 2 "$GATEWAY" && echo "✓ Gateway reachable" || echo "✗ Gateway unreachable"
echo ""

# Enable kill switch (without VPN)
echo "═══════════════════════════════════════════════════════════════"
echo "TEST 2: LAN access with kill switch ON (no VPN)"
echo "═══════════════════════════════════════════════════════════════"
shroud ks on
sleep 2

echo "Kill switch rules:"
sudo iptables -L SHROUD_KILLSWITCH -n 2>&1 | head -10

echo ""
echo "Pinging gateway..."
ping -c 2 -W 2 "$GATEWAY" && echo "✓ Gateway reachable (allow_lan=true?)" || echo "✗ Gateway blocked (allow_lan=false?)"

echo ""
echo "Internet access (should be blocked):"
curl -s --max-time 3 ifconfig.me && echo "✗ LEAK: Internet accessible!" || echo "✓ Internet blocked correctly"

# Check config
echo ""
echo "Current config (allow_lan setting):"
grep -i "allow_lan\|lan" "$HOME/.config/shroud/config.toml" 2>/dev/null || echo "(no LAN config found - using default)"

# Connect VPN and test
VPN=$(nmcli -t -f NAME,TYPE con show | grep vpn | head -1 | cut -d: -f1)
if [[ -n "$VPN" ]]; then
    echo ""
    echo "═══════════════════════════════════════════════════════════════"
    echo "TEST 3: LAN access with kill switch ON and VPN connected"
    echo "═══════════════════════════════════════════════════════════════"
    shroud connect "$VPN" 2>/dev/null || echo "(VPN connect attempted)"
    sleep 5
    
    echo "Pinging gateway..."
    ping -c 2 -W 2 "$GATEWAY" && echo "✓ Gateway reachable" || echo "✗ Gateway blocked"
    
    echo ""
    echo "Internet access:"
    curl -s --max-time 5 ifconfig.me && echo "(through VPN)" || echo "(failed)"
fi

# Cleanup
shroud ks off 2>/dev/null || true
kill $SHROUD_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Review output above."
echo ""
echo "If user 'forgets' kill switch is on and can't print:"
echo "  shroud ks off      # Disable kill switch"
echo "  shroud disconnect  # If VPN is the issue"
echo ""
echo "To allow LAN with kill switch, set in config.toml:"
echo "  [killswitch]"
echo "  allow_lan = true"
