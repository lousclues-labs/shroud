#!/bin/bash
# Pre-test setup - run before each chaos test
# Usage: ./pre-test.sh

set -e

echo "═══════════════════════════════════════════════════════════════"
echo "  SHROUD CHAOS TEST - PRE-FLIGHT CHECK"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Timestamp
echo "Timestamp: $(date -Iseconds)"
echo ""

# 1. Backup config
CONFIG_DIR="$HOME/.config/shroud"
BACKUP_DIR="/tmp/shroud-chaos-backup-$$"
if [[ -d "$CONFIG_DIR" ]]; then
    cp -r "$CONFIG_DIR" "$BACKUP_DIR"
    echo "✓ Config backed up to: $BACKUP_DIR"
else
    echo "⚠ No config dir found (fresh install)"
fi

# 2. Record current iptables state
sudo iptables-save > /tmp/iptables-before-$$
sudo ip6tables-save > /tmp/ip6tables-before-$$
echo "✓ iptables state saved"

# 3. Kill any existing shroud
if pgrep -x shroud > /dev/null; then
    echo "⚠ Killing existing shroud process..."
    pkill -9 shroud 2>/dev/null || true
    sleep 1
fi
echo "✓ No shroud running"

# 4. Clean any stale rules
if sudo iptables -L SHROUD_KILLSWITCH -n 2>/dev/null; then
    echo "⚠ Cleaning stale kill switch rules..."
    sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
    sudo iptables -F SHROUD_KILLSWITCH 2>/dev/null || true
    sudo iptables -X SHROUD_KILLSWITCH 2>/dev/null || true
    sudo ip6tables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
    sudo ip6tables -F SHROUD_KILLSWITCH 2>/dev/null || true
    sudo ip6tables -X SHROUD_KILLSWITCH 2>/dev/null || true
fi
echo "✓ No stale rules"

# 5. Verify NetworkManager
if ! systemctl is-active --quiet NetworkManager; then
    echo "✗ NetworkManager not running!"
    exit 1
fi
echo "✓ NetworkManager running"

# 6. Record network baseline
echo ""
echo "Network Baseline:"
echo "  Real IP: $(curl -s --max-time 5 ifconfig.me || echo 'FAILED')"
echo "  DNS:     $(dig +short +time=2 whoami.akamai.net || echo 'FAILED')"
echo ""

# 7. Enable debug logging
export SHROUD_LOG_LEVEL=debug
echo "✓ Debug logging enabled"

echo ""
echo "Ready for chaos. Config backup: $BACKUP_DIR"
echo "═══════════════════════════════════════════════════════════════"

# Save backup path for post-test
echo "$BACKUP_DIR" > /tmp/shroud-chaos-backup-path
echo "/tmp/iptables-before-$$" > /tmp/shroud-chaos-iptables-path
