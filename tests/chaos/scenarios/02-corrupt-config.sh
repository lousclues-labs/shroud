#!/bin/bash
# Scenario: Corrupt config file while daemon is running
# Safety: 🟡 CAUTION - May need config restore
#
# EXPERIMENT PLAN:
#   Trigger: Start shroud, connect, then corrupt config.toml
#   Duration: 1 minute
#   Observe: Does shroud crash? Does it recover? What happens on next load?
#
# EXPECTED BEHAVIOR:
#   - Running daemon should continue (config already loaded)
#   - On restart, should detect corruption, backup, and create fresh config

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: Corrupt Config Mid-Operation"
echo "  SAFETY: 🟡 CAUTION - Config will be restored"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

CONFIG_FILE="$HOME/.config/shroud/config.toml"

# Ensure config exists
if [[ ! -f "$CONFIG_FILE" ]]; then
    mkdir -p "$(dirname "$CONFIG_FILE")"
    echo 'auto_reconnect = true' > "$CONFIG_FILE"
fi

# Show original config
echo "Original config:"
cat "$CONFIG_FILE"
echo ""

# Start shroud
echo "[T+0s] Starting shroud..."
shroud &
SHROUD_PID=$!
sleep 3

echo "[T+3s] Shroud running, PID: $SHROUD_PID"
shroud status || true
echo ""

# Corrupt the config
echo "[T+5s] Corrupting config file..."
echo "{{{{CORRUPTED GARBAGE]]]]" > "$CONFIG_FILE"
echo "asdfjkl;qwerty not valid toml ====" >> "$CONFIG_FILE"

echo "Corrupted config:"
cat "$CONFIG_FILE"
echo ""

# Wait and observe
echo "[T+7s] Waiting to see if daemon crashes..."
sleep 5

if kill -0 $SHROUD_PID 2>/dev/null; then
    echo "✓ Daemon still running (good - config was already loaded)"
else
    echo "✗ Daemon crashed after config corruption"
fi

# Try some operations
echo ""
echo "[T+12s] Testing operations with corrupted config..."
shroud status || echo "(status failed)"
shroud ks status || echo "(ks status failed)"

# Restart shroud
echo ""
echo "[T+15s] Stopping and restarting shroud..."
kill $SHROUD_PID 2>/dev/null || true
sleep 2

echo "Starting fresh shroud with corrupted config..."
shroud 2>&1 &
NEW_PID=$!
sleep 3

# Check what happened
echo ""
echo "[T+20s] After restart with corrupted config:"
if kill -0 $NEW_PID 2>/dev/null; then
    echo "✓ Shroud started successfully"
    shroud status || true
else
    echo "✗ Shroud failed to start"
fi

# Check if backup was created
echo ""
echo "Config file check:"
if [[ -f "$CONFIG_FILE.corrupted" ]]; then
    echo "✓ Corrupted config backed up to config.toml.corrupted"
fi
if [[ -f "$CONFIG_FILE" ]]; then
    echo "Current config.toml:"
    head -5 "$CONFIG_FILE"
fi

# Cleanup
kill $NEW_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "- Running daemon survived corruption: [check above]"
echo "- Restart handled corruption gracefully: [check above]"
echo "- Backup created: [check for config.toml.corrupted]"
