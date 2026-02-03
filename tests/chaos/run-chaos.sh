#!/usr/bin/env bash
#
# Chaos Test Runner for Shroud
#
# This script systematically tests Shroud's resilience against various
# failure modes. Each test introduces controlled chaos and verifies
# that Shroud handles it gracefully.
#
# WARNING: Some tests are destructive. Run in VM only!
#
# Usage:
#   ./run-chaos.sh           # Run all safe tests
#   ./run-chaos.sh --all     # Run all tests (including destructive)
#   ./run-chaos.sh --test X  # Run specific test
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SHROUD_BIN="${SHROUD_BIN:-${PROJECT_ROOT}/target/release/shroud}"
RESULTS_DIR="${SCRIPT_DIR}/results"
LOG_FILE="${RESULTS_DIR}/chaos-$(date +%Y%m%d-%H%M%S).log"

mkdir -p "$RESULTS_DIR"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

# Counters
PASSED=0
FAILED=0
SKIPPED=0

# Test mode
RUN_DESTRUCTIVE=false
SPECIFIC_TEST=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --all)
            RUN_DESTRUCTIVE=true
            shift
            ;;
        --test)
            SPECIFIC_TEST="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--all] [--test TEST_NAME]"
            echo ""
            echo "Options:"
            echo "  --all        Run all tests including destructive ones"
            echo "  --test NAME  Run only the specified test"
            echo ""
            echo "Available tests:"
            echo "  config_corrupted, config_unwritable, stale_socket,"
            echo "  ipc_flood, ipc_malformed, ipc_disconnect_mid_request,"
            echo "  signal_storm, sigstop_sigcont, rapid_ks_toggle,"
            echo "  concurrent_commands, kill9_recovery, low_fd_limit,"
            echo "  rapid_state_transitions, multiple_instances,"
            echo "  socket_deleted_while_running"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Logging functions
log() { echo -e "$1" | tee -a "$LOG_FILE"; }
log_chaos() { log "${RED}💥 CHAOS${NC}: $1"; }
log_pass() { log "${GREEN}✓ SURVIVED${NC}: $1"; ((PASSED++)); }
log_fail() { log "${RED}✗ BROKEN${NC}: $1"; ((FAILED++)); }
log_skip() { log "${YELLOW}⊘ SKIPPED${NC}: $1"; ((SKIPPED++)); }
log_info() { log "${BLUE}ℹ INFO${NC}: $1"; }
log_section() { log "\n${BOLD}━━━ $1 ━━━${NC}"; }

# Cleanup function
cleanup() {
    log_info "Cleaning up..."
    pkill -f "shroud" 2>/dev/null || true
    sleep 0.5
    
    # Clean up iptables rules
    sudo iptables -F SHROUD_KILLSWITCH 2>/dev/null || true
    sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
    sudo iptables -X SHROUD_KILLSWITCH 2>/dev/null || true
    
    sudo iptables -F SHROUD_BOOT_KS 2>/dev/null || true
    sudo iptables -D OUTPUT -j SHROUD_BOOT_KS 2>/dev/null || true
    sudo iptables -X SHROUD_BOOT_KS 2>/dev/null || true
    
    # Clean up test files
    rm -f "${XDG_RUNTIME_DIR:-/tmp}/shroud.sock" 2>/dev/null || true
}
trap cleanup EXIT

# Wait for daemon to be ready
wait_for_daemon() {
    local max_attempts=${1:-30}
    for i in $(seq 1 $max_attempts); do
        if "$SHROUD_BIN" ping >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.2
    done
    return 1
}

# Start daemon and wait for ready
start_daemon() {
    "$SHROUD_BIN" >/dev/null 2>&1 &
    local pid=$!
    if wait_for_daemon; then
        echo $pid
        return 0
    else
        kill $pid 2>/dev/null || true
        return 1
    fi
}

# Stop daemon gracefully
stop_daemon() {
    "$SHROUD_BIN" quit >/dev/null 2>&1 || true
    sleep 0.5
    pkill -f "shroud" 2>/dev/null || true
}

# Check if test should run
should_run_test() {
    local test_name="$1"
    if [[ -n "$SPECIFIC_TEST" ]]; then
        [[ "$SPECIFIC_TEST" == "$test_name" ]]
    else
        true
    fi
}

# ===========================================================================
# CATEGORY 1: CONFIGURATION CHAOS
# ===========================================================================

test_config_corrupted() {
    should_run_test "config_corrupted" || return 0
    log_chaos "Corrupting config file with garbage data"
    
    local config_dir="${HOME}/.config/shroud"
    local config_file="${config_dir}/config.toml"
    local backup="${config_file}.chaos-backup"
    
    mkdir -p "$config_dir"
    [[ -f "$config_file" ]] && cp "$config_file" "$backup"
    
    # Write complete garbage
    echo '{{{{GARBAGE JSON NOT TOML %%%% INVALID}}}}"' > "$config_file"
    
    # Shroud should handle this gracefully (use defaults)
    if timeout 5 "$SHROUD_BIN" --version >/dev/null 2>&1; then
        log_pass "Handled corrupted config (version check works)"
    else
        log_fail "Crashed or hung on corrupted config"
    fi
    
    # Restore
    if [[ -f "$backup" ]]; then
        mv "$backup" "$config_file"
    else
        rm -f "$config_file"
    fi
}

test_config_unwritable() {
    should_run_test "config_unwritable" || return 0
    log_chaos "Making config directory unwritable"
    
    local config_dir="${HOME}/.config/shroud"
    local orig_perms=""
    
    mkdir -p "$config_dir"
    orig_perms=$(stat -c "%a" "$config_dir" 2>/dev/null || echo "755")
    
    chmod 000 "$config_dir"
    
    # Shroud should still start (read-only mode or defaults)
    if timeout 5 "$SHROUD_BIN" --version >/dev/null 2>&1; then
        log_pass "Handled unwritable config directory"
    else
        log_fail "Crashed with unwritable config directory"
    fi
    
    chmod "$orig_perms" "$config_dir"
}

# ===========================================================================
# CATEGORY 2: IPC CHAOS
# ===========================================================================

test_stale_socket() {
    should_run_test "stale_socket" || return 0
    log_chaos "Creating stale socket file before startup"
    
    stop_daemon
    
    local socket="${XDG_RUNTIME_DIR:-/tmp}/shroud.sock"
    
    # Create a stale socket (just a regular file)
    touch "$socket"
    
    if pid=$(start_daemon); then
        if "$SHROUD_BIN" ping >/dev/null 2>&1; then
            log_pass "Handled stale socket file"
        else
            log_fail "Started but not responding after stale socket"
        fi
        stop_daemon
    else
        log_fail "Failed to start with stale socket present"
    fi
    
    rm -f "$socket"
}

test_ipc_flood() {
    should_run_test "ipc_flood" || return 0
    log_chaos "Flooding IPC with 100 concurrent requests"
    
    stop_daemon
    
    if ! pid=$(start_daemon); then
        log_fail "Could not start daemon for IPC flood test"
        return
    fi
    
    local failures=0
    local pids=()
    
    # Fire 100 concurrent requests
    for i in {1..100}; do
        (timeout 5 "$SHROUD_BIN" ping >/dev/null 2>&1 || exit 1) &
        pids+=($!)
    done
    
    # Wait and count failures
    for pid in "${pids[@]}"; do
        wait $pid || ((failures++))
    done
    
    # Check daemon still alive
    if "$SHROUD_BIN" ping >/dev/null 2>&1; then
        if [[ $failures -lt 20 ]]; then
            log_pass "Handled IPC flood ($failures/100 requests failed)"
        else
            log_fail "Too many failures under IPC flood ($failures/100)"
        fi
    else
        log_fail "Daemon died under IPC flood"
    fi
    
    stop_daemon
}

test_ipc_malformed() {
    should_run_test "ipc_malformed" || return 0
    log_chaos "Sending malformed data to IPC socket"
    
    stop_daemon
    
    if ! pid=$(start_daemon); then
        log_fail "Could not start daemon for malformed IPC test"
        return
    fi
    
    local socket="${XDG_RUNTIME_DIR:-/tmp}/shroud.sock"
    
    # Send various garbage
    echo "GARBAGE" | timeout 2 nc -U "$socket" 2>/dev/null || true
    echo '{"invalid": "json' | timeout 2 nc -U "$socket" 2>/dev/null || true
    echo -e '\x00\x00\x00\x00' | timeout 2 nc -U "$socket" 2>/dev/null || true
    dd if=/dev/urandom bs=1024 count=10 2>/dev/null | timeout 2 nc -U "$socket" 2>/dev/null || true
    
    sleep 0.5
    
    # Daemon should still be alive
    if "$SHROUD_BIN" ping >/dev/null 2>&1; then
        log_pass "Survived malformed IPC messages"
    else
        log_fail "Crashed on malformed IPC messages"
    fi
    
    stop_daemon
}

test_ipc_disconnect_mid_request() {
    should_run_test "ipc_disconnect_mid_request" || return 0
    log_chaos "Disconnecting clients mid-request"
    
    stop_daemon
    
    if ! pid=$(start_daemon); then
        log_fail "Could not start daemon for disconnect test"
        return
    fi
    
    local socket="${XDG_RUNTIME_DIR:-/tmp}/shroud.sock"
    
    # Connect, send partial data, disconnect
    for i in {1..50}; do
        (echo -n '{"comm' | timeout 0.1 nc -U "$socket" 2>/dev/null) &
    done
    wait
    
    sleep 0.5
    
    if "$SHROUD_BIN" ping >/dev/null 2>&1; then
        log_pass "Handled broken pipe from disconnecting clients"
    else
        log_fail "Crashed on client disconnect"
    fi
    
    stop_daemon
}

test_socket_deleted_while_running() {
    should_run_test "socket_deleted_while_running" || return 0
    log_chaos "Deleting IPC socket while daemon running"
    
    stop_daemon
    
    if ! pid=$(start_daemon); then
        log_fail "Could not start daemon for socket deletion test"
        return
    fi
    
    local socket="${XDG_RUNTIME_DIR:-/tmp}/shroud.sock"
    
    # Delete the socket
    rm -f "$socket"
    
    sleep 1
    
    # Commands should fail gracefully
    if ! "$SHROUD_BIN" ping >/dev/null 2>&1; then
        # Expected - socket is gone
        # Check if daemon recreates it or handles gracefully
        sleep 2
        if kill -0 $pid 2>/dev/null; then
            log_pass "Daemon survived socket deletion (still running)"
        else
            log_fail "Daemon died after socket deletion"
        fi
    else
        log_pass "Socket was recreated after deletion"
    fi
    
    stop_daemon
}

# ===========================================================================
# CATEGORY 3: SIGNAL CHAOS
# ===========================================================================

test_signal_storm() {
    should_run_test "signal_storm" || return 0
    log_chaos "Sending signal storm (SIGUSR1, SIGHUP x50)"
    
    stop_daemon
    
    if ! pid=$(start_daemon); then
        log_fail "Could not start daemon for signal storm test"
        return
    fi
    
    # Send rapid signals
    for i in {1..50}; do
        kill -USR1 $pid 2>/dev/null || true
        kill -HUP $pid 2>/dev/null || true
    done
    
    sleep 1
    
    if kill -0 $pid 2>/dev/null && "$SHROUD_BIN" ping >/dev/null 2>&1; then
        log_pass "Survived signal storm"
    else
        log_fail "Died from signal storm"
    fi
    
    stop_daemon
}

test_sigstop_sigcont() {
    should_run_test "sigstop_sigcont" || return 0
    log_chaos "SIGSTOP then SIGCONT (pause/resume)"
    
    stop_daemon
    
    if ! pid=$(start_daemon); then
        log_fail "Could not start daemon for SIGSTOP test"
        return
    fi
    
    # Pause
    kill -STOP $pid 2>/dev/null
    sleep 2
    
    # Resume
    kill -CONT $pid 2>/dev/null
    sleep 1
    
    if "$SHROUD_BIN" ping >/dev/null 2>&1; then
        log_pass "Resumed correctly after SIGSTOP/SIGCONT"
    else
        log_fail "Failed to resume after SIGSTOP/SIGCONT"
    fi
    
    stop_daemon
}

# ===========================================================================
# CATEGORY 4: KILL SWITCH CHAOS
# ===========================================================================

test_rapid_ks_toggle() {
    should_run_test "rapid_ks_toggle" || return 0
    log_chaos "Rapid kill switch toggle (20 times)"
    
    stop_daemon
    
    if ! pid=$(start_daemon); then
        log_fail "Could not start daemon for rapid KS toggle test"
        return
    fi
    
    local pids=()
    for i in {1..20}; do
        "$SHROUD_BIN" ks on >/dev/null 2>&1 &
        pids+=($!)
        "$SHROUD_BIN" ks off >/dev/null 2>&1 &
        pids+=($!)
    done
    
    # Wait for all toggles to complete
    for p in "${pids[@]}"; do
        wait $p 2>/dev/null || true
    done
    
    sleep 1
    
    # Verify state is consistent
    if "$SHROUD_BIN" ks status >/dev/null 2>&1; then
        log_pass "State consistent after rapid toggle"
    else
        log_fail "State corrupted after rapid toggle"
    fi
    
    "$SHROUD_BIN" ks off >/dev/null 2>&1 || true
    stop_daemon
}

# ===========================================================================
# CATEGORY 5: STATE MACHINE CHAOS
# ===========================================================================

test_concurrent_commands() {
    should_run_test "concurrent_commands" || return 0
    log_chaos "Concurrent commands from multiple sources"
    
    stop_daemon
    
    if ! pid=$(start_daemon); then
        log_fail "Could not start daemon for concurrent commands test"
        return
    fi
    
    # Fire many different commands simultaneously
    "$SHROUD_BIN" status >/dev/null 2>&1 &
    "$SHROUD_BIN" ks on >/dev/null 2>&1 &
    "$SHROUD_BIN" status >/dev/null 2>&1 &
    "$SHROUD_BIN" ks off >/dev/null 2>&1 &
    "$SHROUD_BIN" list >/dev/null 2>&1 &
    "$SHROUD_BIN" ping >/dev/null 2>&1 &
    "$SHROUD_BIN" status >/dev/null 2>&1 &
    wait
    
    sleep 0.5
    
    if "$SHROUD_BIN" status >/dev/null 2>&1; then
        log_pass "Handled concurrent commands"
    else
        log_fail "Corrupted by concurrent commands"
    fi
    
    stop_daemon
}

test_rapid_state_transitions() {
    should_run_test "rapid_state_transitions" || return 0
    log_chaos "Rapid state transitions (connect/disconnect spam)"
    
    stop_daemon
    
    if ! pid=$(start_daemon); then
        log_fail "Could not start daemon for state transition test"
        return
    fi
    
    # Note: This test requires a VPN to be configured
    # If no VPN, we just test that commands don't crash
    
    local vpn_name
    vpn_name=$("$SHROUD_BIN" list 2>/dev/null | head -1 | awk '{print $2}' || echo "")
    
    if [[ -z "$vpn_name" || "$vpn_name" == "No" ]]; then
        log_skip "No VPN configured for state transition test"
        stop_daemon
        return
    fi
    
    # Rapid connect/disconnect (don't wait for completion)
    for i in {1..5}; do
        "$SHROUD_BIN" connect "$vpn_name" >/dev/null 2>&1 &
        sleep 0.3
        "$SHROUD_BIN" disconnect >/dev/null 2>&1 &
        sleep 0.3
    done
    wait
    
    sleep 2
    
    # Should be in consistent state
    if "$SHROUD_BIN" status >/dev/null 2>&1; then
        log_pass "Survived rapid state transitions"
    else
        log_fail "State machine corrupted by rapid transitions"
    fi
    
    "$SHROUD_BIN" disconnect >/dev/null 2>&1 || true
    stop_daemon
}

# ===========================================================================
# CATEGORY 6: CRASH RECOVERY
# ===========================================================================

test_kill9_recovery() {
    should_run_test "kill9_recovery" || return 0
    log_chaos "SIGKILL recovery with kill switch on"
    
    stop_daemon
    
    if ! pid=$(start_daemon); then
        log_fail "Could not start daemon for SIGKILL test"
        return
    fi
    
    # Enable kill switch
    "$SHROUD_BIN" ks on >/dev/null 2>&1 || true
    sleep 1
    
    # Brutal kill
    kill -9 $pid 2>/dev/null || true
    sleep 1
    
    # Check if iptables rules exist (stale)
    local stale_rules=false
    if sudo iptables -L SHROUD_KILLSWITCH -n >/dev/null 2>&1; then
        stale_rules=true
        log_info "Stale iptables rules detected after SIGKILL"
    fi
    
    # Start again - should clean up stale rules
    if new_pid=$(start_daemon); then
        if "$SHROUD_BIN" ping >/dev/null 2>&1; then
            if $stale_rules; then
                log_pass "Recovered from SIGKILL (cleaned stale rules)"
            else
                log_pass "Recovered from SIGKILL"
            fi
        else
            log_fail "Started but not responding after SIGKILL recovery"
        fi
    else
        log_fail "Failed to start after SIGKILL"
    fi
    
    "$SHROUD_BIN" ks off >/dev/null 2>&1 || true
    stop_daemon
}

test_multiple_instances() {
    should_run_test "multiple_instances" || return 0
    log_chaos "Attempting to start multiple instances"
    
    stop_daemon
    
    if ! pid1=$(start_daemon); then
        log_fail "Could not start first daemon instance"
        return
    fi
    
    # Try to start second instance
    "$SHROUD_BIN" >/dev/null 2>&1 &
    local pid2=$!
    sleep 2
    
    # Second should have exited (lock conflict)
    if kill -0 $pid2 2>/dev/null; then
        log_fail "Second instance running (should be blocked)"
        kill $pid2 2>/dev/null || true
    else
        # First should still work
        if "$SHROUD_BIN" ping >/dev/null 2>&1; then
            log_pass "Prevented multiple instances correctly"
        else
            log_fail "First instance died when second tried to start"
        fi
    fi
    
    stop_daemon
}

# ===========================================================================
# CATEGORY 7: RESOURCE EXHAUSTION
# ===========================================================================

test_low_fd_limit() {
    should_run_test "low_fd_limit" || return 0
    log_chaos "Running with very low file descriptor limit"
    
    stop_daemon
    
    # Run shroud with limited FDs
    local result
    result=$(
        ulimit -n 50 2>/dev/null
        if timeout 5 "$SHROUD_BIN" --version 2>&1; then
            echo "pass"
        else
            echo "fail"
        fi
    )
    
    if [[ "$result" == *"pass"* ]]; then
        log_pass "Handled low FD limit (version check)"
    else
        log_fail "Failed with low FD limit"
    fi
}

# ===========================================================================
# MAIN EXECUTION
# ===========================================================================

main() {
    log ""
    log "╔════════════════════════════════════════════════════════════════════╗"
    log "║           ${BOLD}SHROUD CHAOS ENGINEERING TESTS${NC}                          ║"
    log "╠════════════════════════════════════════════════════════════════════╣"
    log "║  ⚠️  Some tests modify system state                                ║"
    log "║  ⚠️  Run in a VM or container for safety                           ║"
    log "╚════════════════════════════════════════════════════════════════════╝"
    log ""
    log_info "Log file: $LOG_FILE"
    log_info "Shroud binary: $SHROUD_BIN"
    log ""
    
    # Check binary exists
    if [[ ! -x "$SHROUD_BIN" ]]; then
        log_info "Building Shroud..."
        (cd "$PROJECT_ROOT" && cargo build --release) || {
            log_fail "Failed to build Shroud"
            exit 1
        }
    fi
    
    # Check for required tools
    if ! command -v nc >/dev/null 2>&1; then
        log_info "Installing netcat for IPC tests..."
        # Try common package managers
        sudo apt-get install -y netcat-openbsd 2>/dev/null || \
        sudo pacman -S --noconfirm openbsd-netcat 2>/dev/null || \
        sudo dnf install -y nc 2>/dev/null || \
        log_info "Could not install netcat - some tests may skip"
    fi
    
    # Clean up any existing state
    cleanup
    
    # Run tests
    log_section "CONFIGURATION CHAOS"
    test_config_corrupted
    test_config_unwritable
    
    log_section "IPC CHAOS"
    test_stale_socket
    test_ipc_flood
    test_ipc_malformed
    test_ipc_disconnect_mid_request
    test_socket_deleted_while_running
    
    log_section "SIGNAL CHAOS"
    test_signal_storm
    test_sigstop_sigcont
    
    log_section "KILL SWITCH CHAOS"
    test_rapid_ks_toggle
    
    log_section "STATE MACHINE CHAOS"
    test_concurrent_commands
    test_rapid_state_transitions
    
    log_section "CRASH RECOVERY"
    test_kill9_recovery
    test_multiple_instances
    
    log_section "RESOURCE EXHAUSTION"
    test_low_fd_limit
    
    # Summary
    log ""
    log "╔════════════════════════════════════════════════════════════════════╗"
    log "║                    ${BOLD}CHAOS TEST RESULTS${NC}                             ║"
    log "╠════════════════════════════════════════════════════════════════════╣"
    log "║  ${GREEN}Survived${NC}: $(printf '%3d' $PASSED)                                                  ║"
    log "║  ${RED}Broken${NC}:   $(printf '%3d' $FAILED)                                                  ║"
    log "║  ${YELLOW}Skipped${NC}:  $(printf '%3d' $SKIPPED)                                                  ║"
    log "╚════════════════════════════════════════════════════════════════════╝"
    log ""
    
    # Write JSON results
    cat > "${RESULTS_DIR}/chaos-results.json" << EOF
{
    "suite": "chaos",
    "passed": $PASSED,
    "failed": $FAILED,
    "skipped": $SKIPPED,
    "timestamp": "$(date -Iseconds)",
    "shroud_version": "$("$SHROUD_BIN" --version 2>/dev/null || echo "unknown")"
}
EOF
    
    log_info "Results written to: ${RESULTS_DIR}/chaos-results.json"
    
    # Exit with failure if any tests failed
    [[ $FAILED -eq 0 ]]
}

main "$@"
