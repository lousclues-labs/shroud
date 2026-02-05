#!/bin/bash
# Check for DNS/IP leaks
# Usage: ./check-leaks.sh [expected_vpn_ip]

echo "═══════════════════════════════════════════════════════════════"
echo "  LEAK CHECK"
echo "═══════════════════════════════════════════════════════════════"

EXPECTED_VPN_IP="${1:-}"

# Get current IP
CURRENT_IP=$(curl -s --max-time 5 ifconfig.me || echo "FAILED")
echo "Current IP: $CURRENT_IP"

# DNS check
DNS_RESULT=$(dig +short +time=2 whoami.akamai.net 2>/dev/null || echo "FAILED")
echo "DNS Result: $DNS_RESULT"

# WebRTC would need browser - skip

# Check if VPN is supposed to be active
VPN_ACTIVE=$(nmcli con show --active 2>/dev/null | grep -c vpn || echo "0")
echo "VPN Active: $VPN_ACTIVE"

if [[ "$VPN_ACTIVE" -gt 0 ]]; then
    # Get VPN interface IP
    VPN_IF=$(ip route | grep -E "tun|wg" | head -1 | awk '{print $3}' || echo "")
    echo "VPN Interface: $VPN_IF"
    
    if [[ -n "$EXPECTED_VPN_IP" ]]; then
        if [[ "$CURRENT_IP" == "$EXPECTED_VPN_IP" ]]; then
            echo "✓ IP matches expected VPN IP"
        else
            echo "✗ IP LEAK DETECTED! Expected $EXPECTED_VPN_IP, got $CURRENT_IP"
        fi
    fi
fi

# Check if kill switch is active
KS_ACTIVE=$(sudo iptables -L SHROUD_KILLSWITCH -n 2>/dev/null | wc -l || echo "0")
echo "Kill Switch Rules: $KS_ACTIVE"

if [[ "$KS_ACTIVE" -gt 2 && "$VPN_ACTIVE" -eq 0 ]]; then
    echo "⚠ Kill switch active but no VPN - traffic should be blocked"
    if [[ "$CURRENT_IP" != "FAILED" ]]; then
        echo "✗ LEAK! Kill switch active but traffic got through!"
    else
        echo "✓ Traffic correctly blocked"
    fi
fi

echo "═══════════════════════════════════════════════════════════════"
