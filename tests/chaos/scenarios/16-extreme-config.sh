#!/bin/bash
# Scenario: Set health check interval to 0 or extreme values
# Safety: 🟢 SAFE - Tests config validation
#
# EXPERIMENT PLAN:
#   Trigger: Modify config with extreme values, restart shroud
#   Duration: 30 seconds
#   Observe: Does shroud validate? Crash? CPU spike?
#
# EXPECTED BEHAVIOR:
#   - Invalid values should be rejected or clamped
#   - Extreme values should not cause resource exhaustion

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: Extreme Config Values"
echo "  SAFETY: 🟢 SAFE"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

CONFIG_FILE="$HOME/.config/shroud/config.toml"
BACKUP_CONFIG="$CONFIG_FILE.chaos-backup"

# Backup config
cp "$CONFIG_FILE" "$BACKUP_CONFIG" 2>/dev/null || touch "$BACKUP_CONFIG"

test_config() {
    local test_name="$1"
    local config_content="$2"
    
    echo "═══════════════════════════════════════════════════════════════"
    echo "TEST: $test_name"
    echo "═══════════════════════════════════════════════════════════════"
    
    # Write config
    echo "$config_content" > "$CONFIG_FILE"
    echo "Config:"
    cat "$CONFIG_FILE"
    echo ""
    
    # Start shroud and observe
    shroud &
    local pid=$!
    sleep 3
    
    # Check if running
    if kill -0 $pid 2>/dev/null; then
        echo "✓ Shroud started"
        
        # Check CPU (should not be spinning)
        local cpu=$(ps -p $pid -o %cpu= 2>/dev/null || echo "0")
        echo "CPU usage: ${cpu}%"
        if (( $(echo "$cpu > 50" | bc -l 2>/dev/null || echo 0) )); then
            echo "⚠ HIGH CPU - possible busy loop!"
        fi
        
        # Try operations
        shroud status 2>&1 | head -3 || echo "(status failed)"
        
        kill $pid 2>/dev/null || true
        sleep 1
    else
        echo "✗ Shroud failed to start"
        wait $pid 2>/dev/null
        echo "Exit code: $?"
    fi
    
    echo ""
}

# Test 1: health_check_interval = 0
test_config "health_check_interval = 0" "
auto_reconnect = true
health_check_interval = 0
"

# Test 2: health_check_interval = -1
test_config "health_check_interval = -1 (negative)" "
auto_reconnect = true
health_check_interval = -1
"

# Test 3: Very large interval
test_config "health_check_interval = 999999999" "
auto_reconnect = true
health_check_interval = 999999999
"

# Test 4: max_reconnect_attempts = 0 (should mean infinite)
test_config "max_reconnect_attempts = 0 (infinite)" "
auto_reconnect = true
max_reconnect_attempts = 0
"

# Test 5: Very large retry count
test_config "max_reconnect_attempts = 999999" "
auto_reconnect = true
max_reconnect_attempts = 999999
"

# Test 6: Empty config
test_config "Empty config" ""

# Test 7: Only whitespace
test_config "Only whitespace" "    
    
"

# Restore original config
mv "$BACKUP_CONFIG" "$CONFIG_FILE" 2>/dev/null || rm -f "$CONFIG_FILE"

"$SCRIPT_DIR/../post-test.sh"

echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Review output above for:"
echo "- Which configs caused startup failures?"
echo "- Were any configs accepted that shouldn't be?"
echo "- Did any cause high CPU usage?"
