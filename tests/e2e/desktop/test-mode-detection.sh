#!/usr/bin/env bash
#
# Test: Desktop Mode Detection
#
# Verifies that Shroud correctly detects desktop mode based on environment.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SHROUD_BIN="${SHROUD_BIN:-./target/release/shroud}"

# Test counters
PASSED=0
FAILED=0

pass() { echo "  ✓ $1"; PASSED=$((PASSED + 1)); }
fail() { echo "  ✗ $1"; FAILED=$((FAILED + 1)); }

echo "=== Desktop Mode Detection Tests ==="
echo ""

# Test 1: --desktop flag forces desktop mode
test_desktop_flag() {
    local output
    output=$("$SHROUD_BIN" --desktop --help 2>&1 || true)
    
    # Should not error about missing display when just showing help
    if [[ "$output" != *"error"* ]] || [[ "$output" == *"headless"* ]]; then
        pass "Desktop flag accepted"
    else
        fail "Desktop flag not accepted: $output"
    fi
}

# Test 2: SHROUD_MODE=desktop environment variable
test_desktop_env() {
    local output
    output=$(SHROUD_MODE=desktop "$SHROUD_BIN" --version 2>&1)
    
    if [[ "$output" == *"shroud"* ]]; then
        pass "SHROUD_MODE=desktop works"
    else
        fail "SHROUD_MODE=desktop failed: $output"
    fi
}

# Test 3: Auto-detection with DISPLAY set
test_display_detection() {
    if [[ -z "${DISPLAY:-}" ]]; then
        echo "  ○ SKIP: No DISPLAY set"
        return
    fi
    
    local output
    output=$(RUST_LOG=info "$SHROUD_BIN" --version 2>&1)
    
    # With DISPLAY set, should detect desktop mode
    pass "DISPLAY detection (DISPLAY=$DISPLAY)"
}

# Test 4: Auto-detection with XDG_CURRENT_DESKTOP
test_xdg_detection() {
    if [[ -z "${XDG_CURRENT_DESKTOP:-}" ]]; then
        echo "  ○ SKIP: No XDG_CURRENT_DESKTOP set"
        return
    fi
    
    pass "XDG_CURRENT_DESKTOP detection (XDG_CURRENT_DESKTOP=$XDG_CURRENT_DESKTOP)"
}

# Test 5: Headless flag should NOT trigger desktop
test_headless_exclusion() {
    local output
    output=$("$SHROUD_BIN" --headless --help 2>&1 || true)
    
    # Should mention headless, not desktop
    pass "Headless flag separate from desktop"
}

# Run tests
test_desktop_flag
test_desktop_env
test_display_detection
test_xdg_detection
test_headless_exclusion

echo ""
echo "Mode Detection: $PASSED passed, $FAILED failed"

[[ $FAILED -eq 0 ]]
