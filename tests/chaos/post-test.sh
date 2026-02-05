#!/bin/bash
# Post-test cleanup - run after each chaos test
# Usage: ./post-test.sh

set -e

echo "═══════════════════════════════════════════════════════════════"
echo "  SHROUD CHAOS TEST - POST-TEST CLEANUP"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Timestamp
echo "Timestamp: $(date -Iseconds)"
echo ""

# 1. Kill shroud if running
if pgrep -x shroud > /dev/null; then
    echo "Stopping shroud..."
    shroud quit 2>/dev/null || pkill -15 shroud 2>/dev/null || true
    sleep 2
    pkill -9 shroud 2>/dev/null || true
fi
echo "✓ Shroud stopped"

# 2. Clean kill switch rules
echo "Cleaning kill switch rules..."
sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
sudo iptables -F SHROUD_KILLSWITCH 2>/dev/null || true
sudo iptables -X SHROUD_KILLSWITCH 2>/dev/null || true
sudo ip6tables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
sudo ip6tables -F SHROUD_KILLSWITCH 2>/dev/null || true
sudo ip6tables -X SHROUD_KILLSWITCH 2>/dev/null || true
sudo iptables -D OUTPUT -j SHROUD_BOOT_KS 2>/dev/null || true
sudo iptables -F SHROUD_BOOT_KS 2>/dev/null || true
sudo iptables -X SHROUD_BOOT_KS 2>/dev/null || true
echo "✓ Kill switch rules cleaned"

# 3. Disconnect any VPN
echo "Disconnecting VPNs..."
nmcli con show --active | grep vpn | awk '{print $1}' | while read vpn; do
    nmcli con down "$vpn" 2>/dev/null || true
done
echo "✓ VPNs disconnected"

# 4. Restore config if backed up
BACKUP_PATH=$(cat /tmp/shroud-chaos-backup-path 2>/dev/null || echo "")
if [[ -n "$BACKUP_PATH" && -d "$BACKUP_PATH" ]]; then
    echo "Restoring config from $BACKUP_PATH..."
    rm -rf "$HOME/.config/shroud"
    mv "$BACKUP_PATH" "$HOME/.config/shroud"
    echo "✓ Config restored"
else
    echo "⚠ No config backup found"
fi

# 5. Remove tc rules if any
sudo tc qdisc del dev eth0 root 2>/dev/null || true
sudo tc qdisc del dev wlan0 root 2>/dev/null || true
sudo tc qdisc del dev enp0s3 root 2>/dev/null || true
echo "✓ Traffic shaping rules removed"

# 6. Check for residual rules
echo ""
echo "Residual Rule Check:"
RESIDUAL=$(sudo iptables -S 2>/dev/null | grep -c SHROUD || echo "0")
if [[ "$RESIDUAL" -gt 0 ]]; then
    echo "  ⚠ WARNING: $RESIDUAL residual SHROUD rules found!"
    sudo iptables -S | grep SHROUD
else
    echo "  ✓ No residual rules"
fi

# 7. Verify network restored
echo ""
echo "Network Verification:"
if curl -s --max-time 5 ifconfig.me > /dev/null; then
    echo "  ✓ Internet connectivity restored"
    echo "  IP: $(curl -s --max-time 5 ifconfig.me)"
else
    echo "  ✗ WARNING: No internet connectivity!"
fi

# 8. Clean temp files
rm -f /tmp/shroud-chaos-backup-path
rm -f /tmp/shroud-chaos-iptables-path
rm -f /tmp/iptables-before-*
rm -f /tmp/ip6tables-before-*

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  CLEANUP COMPLETE"
echo "═══════════════════════════════════════════════════════════════"
