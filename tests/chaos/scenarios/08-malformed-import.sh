#!/bin/bash
# Scenario: Import malformed OpenVPN config
# Safety: 🟢 SAFE - Tests input validation
#
# EXPERIMENT PLAN:
#   Trigger: Try to import various malformed config files
#   Duration: 30 seconds
#   Observe: Does shroud reject gracefully? Any crashes? Clear errors?
#
# EXPECTED BEHAVIOR:
#   - Malformed configs should be rejected with clear error messages
#   - No crashes, no partial imports
#   - Valid configs should still work after bad ones fail

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════"
echo "  SCENARIO: Malformed Config Import"
echo "  SAFETY: 🟢 SAFE"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Pre-flight
"$SCRIPT_DIR/../pre-test.sh"

# Create temp dir for test configs
TEMP_DIR=$(mktemp -d)
echo "Using temp dir: $TEMP_DIR"
echo ""

# Start shroud
echo "[T+0s] Starting shroud..."
shroud &
SHROUD_PID=$!
sleep 3

# Test 1: Empty file
echo "═══════════════════════════════════════════════════════════════"
echo "TEST 1: Empty file"
echo "═══════════════════════════════════════════════════════════════"
touch "$TEMP_DIR/empty.ovpn"
shroud import "$TEMP_DIR/empty.ovpn" 2>&1 || echo "(import failed - expected)"
echo ""

# Test 2: Binary garbage
echo "═══════════════════════════════════════════════════════════════"
echo "TEST 2: Binary garbage"
echo "═══════════════════════════════════════════════════════════════"
dd if=/dev/urandom of="$TEMP_DIR/garbage.ovpn" bs=1K count=1 2>/dev/null
shroud import "$TEMP_DIR/garbage.ovpn" 2>&1 || echo "(import failed - expected)"
echo ""

# Test 3: Partial/truncated config
echo "═══════════════════════════════════════════════════════════════"
echo "TEST 3: Truncated config"
echo "═══════════════════════════════════════════════════════════════"
cat > "$TEMP_DIR/truncated.ovpn" << 'EOF'
client
dev tun
proto udp
remote vpn.example.com 1194
EOF
# Missing many required fields
shroud import "$TEMP_DIR/truncated.ovpn" 2>&1 || echo "(import result)"
echo ""

# Test 4: Path traversal attempt
echo "═══════════════════════════════════════════════════════════════"
echo "TEST 4: Path traversal in name"
echo "═══════════════════════════════════════════════════════════════"
cat > "$TEMP_DIR/valid.ovpn" << 'EOF'
client
dev tun
proto udp
remote vpn.example.com 1194
resolv-retry infinite
nobind
persist-key
persist-tun
EOF
shroud import "$TEMP_DIR/valid.ovpn" --name "../../../etc/passwd" 2>&1 || echo "(import result)"
echo ""

# Test 5: Very long name
echo "═══════════════════════════════════════════════════════════════"
echo "TEST 5: Extremely long name (1000 chars)"
echo "═══════════════════════════════════════════════════════════════"
LONG_NAME=$(printf 'A%.0s' {1..1000})
shroud import "$TEMP_DIR/valid.ovpn" --name "$LONG_NAME" 2>&1 || echo "(import result)"
echo ""

# Test 6: Special characters in name
echo "═══════════════════════════════════════════════════════════════"
echo "TEST 6: Special characters in name"
echo "═══════════════════════════════════════════════════════════════"
shroud import "$TEMP_DIR/valid.ovpn" --name "test;rm -rf /" 2>&1 || echo "(import result)"
echo ""

# Test 7: Non-existent file
echo "═══════════════════════════════════════════════════════════════"
echo "TEST 7: Non-existent file"
echo "═══════════════════════════════════════════════════════════════"
shroud import "/nonexistent/path/to/config.ovpn" 2>&1 || echo "(import failed - expected)"
echo ""

# Check shroud still running
echo "═══════════════════════════════════════════════════════════════"
echo "STABILITY CHECK"
echo "═══════════════════════════════════════════════════════════════"
if kill -0 $SHROUD_PID 2>/dev/null; then
    echo "✓ Shroud still running after all malformed inputs"
    shroud status || true
else
    echo "✗ Shroud crashed during malformed input tests!"
fi

# Cleanup
rm -rf "$TEMP_DIR"
kill $SHROUD_PID 2>/dev/null || true
"$SCRIPT_DIR/../post-test.sh"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
echo "Review output above for each test case."
echo "Key questions:"
echo "- Were all malformed configs rejected?"
echo "- Were error messages clear and helpful?"
echo "- Did shroud remain stable throughout?"
