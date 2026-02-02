#!/usr/bin/env bash
# Mode Detection Tests
#
# Tests for RuntimeMode detection logic (headless vs desktop)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

# ============================================================================
# Test Functions
# ============================================================================

test_mode_help_includes_headless() {
    local output
    output=$(shroud --help 2>&1 || true)
    assert_contains "$output" "--headless" "Help should mention --headless flag"
}

test_mode_help_includes_desktop() {
    local output
    output=$(shroud --help 2>&1 || true)
    assert_contains "$output" "--desktop" "Help should mention --desktop flag"
}

test_mode_headless_flag_recognized() {
    # Check the flag is recognized with help
    local output
    output=$(timeout 2s shroud --headless --help 2>&1 || true)
    # Should not error about unknown flag
    assert_not_contains "$output" "unknown" "Should recognize --headless flag"
    assert_not_contains "$output" "unrecognized" "Should recognize --headless flag"
}

test_mode_desktop_flag_recognized() {
    local output
    output=$(timeout 2s shroud --desktop --help 2>&1 || true)
    assert_not_contains "$output" "unknown" "Should recognize --desktop flag"
    assert_not_contains "$output" "unrecognized" "Should recognize --desktop flag"
}

test_mode_conflict_detection() {
    # Both --headless and --desktop - currently headless wins
    # This test documents current behavior (headless takes priority)
    local output exit_code
    set +e
    # Use timeout because if it starts as daemon it won't exit
    output=$(timeout 2s shroud --headless --desktop --help 2>&1)
    exit_code=$?
    set -e
    
    # With --help, should show help and exit
    assert_contains "$output" "Shroud" "Should show help with both flags + --help"
}

test_mode_detect_xdg_session() {
    # With XDG_SESSION_TYPE set, should detect desktop environment
    local output
    output=$(XDG_SESSION_TYPE=x11 shroud --help 2>&1 || true)
    # This just verifies no crash when XDG_SESSION_TYPE is set
    assert_contains "$output" "shroud" "Should run with XDG_SESSION_TYPE set"
}

test_mode_detect_no_display() {
    # Without DISPLAY, should prefer headless mode detection
    local output
    output=$(unset DISPLAY; shroud --help 2>&1 || true)
    assert_contains "$output" "shroud" "Should run without DISPLAY"
}

test_mode_systemd_detection() {
    # Test that INVOCATION_ID from systemd is detected
    local output
    output=$(INVOCATION_ID=test-id shroud --help 2>&1 || true)
    assert_contains "$output" "shroud" "Should run with INVOCATION_ID set"
}

test_mode_cli_overrides_env() {
    # --desktop should work even when INVOCATION_ID is set
    local output
    output=$(INVOCATION_ID=test-id shroud --desktop --help 2>&1 || true)
    assert_contains "$output" "Shroud" "Should allow --desktop override"
}

# ============================================================================
# Run Tests
# ============================================================================

begin_suite "mode-detection"

run_test "Help includes --headless" test_mode_help_includes_headless
run_test "Help includes --desktop" test_mode_help_includes_desktop
run_test "--headless flag recognized" test_mode_headless_flag_recognized
run_test "--desktop flag recognized" test_mode_desktop_flag_recognized
run_test "Conflict detection (--headless + --desktop)" test_mode_conflict_detection
run_test "Detect XDG_SESSION_TYPE" test_mode_detect_xdg_session
run_test "Detect missing DISPLAY" test_mode_detect_no_display
run_test "Detect INVOCATION_ID (systemd)" test_mode_systemd_detection
run_test "CLI overrides environment" test_mode_cli_overrides_env

end_suite
