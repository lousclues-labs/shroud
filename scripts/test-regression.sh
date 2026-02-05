#!/usr/bin/env bash
#
# Regression Tests for Shroud
#
# Tests for previously fixed bugs to prevent regressions.
# Run with: ./scripts/test-regression.sh
#

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/common.sh"

log_section "Shroud Regression Tests"

reset_counters
start_timer

# ============================================================================
# BUG: Invalid VPN State (Fixed in 1.8.8)
# https://github.com/loujr/shroud/commit/1333555
#
# Issue: When connecting to a non-existent VPN, state got stuck in 
#        "Reconnecting" and showed "Connected to: nonexistent-vpn"
# Fix: Added ConnectionFailed event that transitions to Disconnected
# ============================================================================

test_invalid_vpn_state() {
    log_info "Testing: Invalid VPN state bug (issue 1.8.8)"
    
    # Check that ConnectionFailed event exists in the codebase
    if grep -q "ConnectionFailed" "$PROJECT_ROOT/src/state/types.rs"; then
        record_result "ConnectionFailed event exists" "pass"
    else
        record_result "ConnectionFailed event exists" "fail" "Missing from state types"
        return 1
    fi
    
    # Check that handlers dispatch ConnectionFailed on failure
    if grep -q "Event::ConnectionFailed" "$PROJECT_ROOT/src/supervisor/handlers.rs"; then
        record_result "Handlers dispatch ConnectionFailed" "pass"
    else
        record_result "Handlers dispatch ConnectionFailed" "fail" "Not dispatched in handlers"
        return 1
    fi
    
    # Check state machine handles ConnectionFailed -> Disconnected
    if grep -A6 "Event::ConnectionFailed" "$PROJECT_ROOT/src/state/machine.rs" | grep -q "VpnState::Disconnected"; then
        record_result "State machine handles ConnectionFailed" "pass"
    else
        record_result "State machine handles ConnectionFailed" "fail" "Doesn't transition to Disconnected"
        return 1
    fi
}

# ============================================================================
# BUG: Kill Switch Toggle Race Condition (Fixed in 1.8.9)
# https://github.com/loujr/shroud/commit/84573a6
#
# Issue: When toggling kill switch, tray would briefly show wrong state
# Fix: Optimistic UI update before async operation completes
# ============================================================================

test_killswitch_race_condition() {
    log_info "Testing: Kill switch toggle race condition (issue 1.8.9)"
    
    local handler_file="$PROJECT_ROOT/src/supervisor/handlers.rs"
    
    # Look for the pattern: update shared state, then call enable/disable
    if grep -A20 "pub(crate) async fn toggle_kill_switch" "$handler_file" | \
       grep -B5 "self.kill_switch.enable" | \
       grep -q "state.kill_switch = new_enabled"; then
        record_result "Optimistic state update before async" "pass"
    else
        record_result "Optimistic state update before async" "fail" "Missing or wrong order"
        return 1
    fi
    
    # Check for rollback on failure
    if grep -A10 "Err(e) =>" "$handler_file" | grep -q "kill_switch = current_enabled"; then
        record_result "Rollback logic for failed toggle" "pass"
    elif grep -q "Rollback optimistic state update" "$handler_file"; then
        record_result "Rollback logic for failed toggle" "pass"
    else
        record_result "Rollback logic for failed toggle" "fail" "Missing rollback"
        return 1
    fi
}

# ============================================================================
# BUG: Kill Switch State Flicker (Fixed in 1.8.7)
#
# Issue: Kill switch would flicker enabled/disabled because state checks
#        ran iptables without sudo, causing permission denied
# Fix: Use sudo -n for all iptables state checking
# ============================================================================

test_killswitch_state_flicker() {
    log_info "Testing: Kill switch state flicker (issue 1.8.7)"
    
    local firewall_file="$PROJECT_ROOT/src/killswitch/firewall.rs"
    
    # Check that is_actually_enabled uses sudo
    if grep -A10 "fn is_actually_enabled" "$firewall_file" 2>/dev/null | \
       grep -q "sudo"; then
        record_result "is_actually_enabled uses sudo" "pass"
    elif grep -q "run_iptables.*sudo\|sudo.*iptables" "$firewall_file" 2>/dev/null; then
        record_result "iptables checks use sudo" "pass"
    else
        record_result "iptables state checks use sudo" "fail" "May not use sudo"
        return 1
    fi
}

# ============================================================================
# BUG: SHROUD_NMCLI Environment Variable Support
#
# Ensure the mock nmcli can be used for testing
# ============================================================================

test_nmcli_env_override() {
    log_info "Testing: SHROUD_NMCLI environment variable support"
    
    # Check nm/client.rs has nmcli_command() function
    if grep -q "fn nmcli_command()" "$PROJECT_ROOT/src/nm/client.rs"; then
        record_result "nmcli_command() helper exists" "pass"
    else
        record_result "nmcli_command() helper exists" "fail" "Missing helper function"
        return 1
    fi
    
    # Check it reads SHROUD_NMCLI
    if grep -q 'SHROUD_NMCLI' "$PROJECT_ROOT/src/nm/client.rs"; then
        record_result "SHROUD_NMCLI env var supported" "pass"
    else
        record_result "SHROUD_NMCLI env var supported" "fail" "Not reading env var"
        return 1
    fi
}

# ============================================================================
# BUG: Stale Kill Switch Rules on Crash
#
# Issue: If shroud is killed while kill switch is active, rules are orphaned
# Fix: Detect and clean stale rules on startup
# ============================================================================

test_stale_rules_detection() {
    log_info "Testing: Stale kill switch rules detection"
    
    # Check for stale rules detection in cleanup module
    if grep -rq "STALE.*KILL.*SWITCH\|stale.*rules" "$PROJECT_ROOT/src/killswitch/" 2>/dev/null; then
        record_result "Stale rules detection exists" "pass"
    else
        record_result "Stale rules detection exists" "fail" "Missing detection logic"
        return 1
    fi
}

# ============================================================================
# BUG: IPC Socket Cleanup on Startup
#
# Issue: If shroud crashes, stale socket prevents restart
# Fix: Clean up socket on startup
# ============================================================================

test_ipc_socket_cleanup() {
    log_info "Testing: IPC socket cleanup on startup"
    
    if grep -q "remove_file\|unlink\|fs::remove" "$PROJECT_ROOT/src/ipc/server.rs"; then
        record_result "IPC socket cleanup exists" "pass"
    else
        record_result "IPC socket cleanup exists" "fail" "Missing cleanup"
        return 1
    fi
}

# ============================================================================
# BUG: Signal Handlers
#
# Issue: Shroud didn't handle SIGTERM gracefully
# Fix: Install signal handlers for graceful shutdown
# ============================================================================

test_signal_handlers() {
    log_info "Testing: Signal handlers installed"
    
    local main_content supervisor_content
    main_content=$(cat "$PROJECT_ROOT/src/main.rs" 2>/dev/null || echo "")
    supervisor_content=$(cat "$PROJECT_ROOT/src/supervisor/mod.rs" 2>/dev/null || echo "")
    
    if echo "$main_content" | grep -qE "signal|ctrlc|tokio.*signal" || \
       echo "$supervisor_content" | grep -qE "signal|ctrlc"; then
        record_result "Signal handlers installed" "pass"
    else
        record_result "Signal handlers installed" "fail" "Missing signal handling"
        return 1
    fi
}

# ============================================================================
# RUN ALL REGRESSION TESTS
# ============================================================================

echo ""
log_info "Running regression tests..."
echo ""

test_invalid_vpn_state || true
test_killswitch_race_condition || true
test_killswitch_state_flicker || true
test_nmcli_env_override || true
test_stale_rules_detection || true
test_ipc_socket_cleanup || true
test_signal_handlers || true

echo ""

elapsed=$(get_elapsed)
write_recorded_results "regression" "$elapsed"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS: $(get_passed) passed, $(get_failed) failed, $(get_skipped) skipped"
echo "═══════════════════════════════════════════════════════════════"

[[ $(get_failed) -eq 0 ]]
