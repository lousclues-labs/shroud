#!/bin/bash
# Scenario: Delete ~/.config/shroud while running
# Safety: 🟡 CAUTION - Will need config restore
#
# EXPERIMENT PLAN:
#   Trigger: Start shroud, connect, then delete config directory
#   Duration: 1 minute
#   Observe: Does shroud crash? Can it recover? What happens on restart?
#
# EXPECTED BEHAVIOR:
#   - Running daemon should continue (config in memory)
#   - Operations that need config may fail
#   - Restart should recreate default config

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: Delete Config Directory While Running"
echo "  SAFETY: 🟡 CAUTION - Config will be backed up and restored"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight (includes backup)
"$SCRIPT_DIR/../pre-test.sh"

CONFIG_DIR="$HOME/.config/shroud"

# Start shroud
echo "[T+0s] Starting shroud..."
shroud &
SHROUD_PID=$!
sleep 3

echo "[T+3s] Baseline:"
shroud status || true
echo ""

# Delete config
echo "═══════════════════════════════════════════════════════════════"
echo "[T+5s] Deleting config directory..."
echo "═══════════════════════════════════════════════════════════════"
rm -rf "$CONFIG_DIR"
echo "✓ Config directory deleted"
ls -la "$HOME/.config/" | grep shroud || echo "  (no shroud dir found - correct)"
echo ""

# Test operations
echo "[T+7s] Testing operations without config:"
echo "  status:"
shroud status 2>&1 || echo "  (status result)"

echo "  ks status:"
shroud ks status 2>&1 || echo "  (ks status result)"

# Check if daemon survived
if kill -0 $SHROUD_PID 2>/dev/null; then
    echo ""
    echo "✓ Shroud still running without config dir"
else
    echo ""
    echo "✗ Shroud crashed when config deleted"
fi

# Restart shroud
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "[T+10s] Restarting shroud without config..."
echo "═══════════════════════════════════════════════════════════════"
kill $SHROUD_PID 2>/dev/null || true
sleep 2

shroud &
NEW_PID=$!
sleep 3

echo "After restart:"
shroud status 2>&1 || echo "(status result)"

# Check if config was recreated
echo ""
echo "Config directory check:"
if [[ -d "$CONFIG_DIR" ]]; then
    echo "✓ Config directory recreated"
    ls -la "$CONFIG_DIR/"
else
    echo "⚠ Config directory not recreated"
fi

# Cleanup (post-test restores from backup)
kill $NEW_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Check output above for:"
echo "- Did daemon survive config deletion?"
echo "- Did restart recreate config?"
echo "- Were any operations affected?"
