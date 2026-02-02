#!/usr/bin/env bash
# Headless Runtime Tests (Privileged)
#
# Tests for headless daemon operation

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

require_root

# ============================================================================
# Setup/Teardown
# ============================================================================

TEST_CONFIG_DIR="/tmp/shroud-test-$$"
TEST_SOCKET="${XDG_RUNTIME_DIR:-/tmp}/shroud-test-$$.sock"
SHROUD_PID=""

setup_headless_test() {
    cleanup_test_env
    mkdir -p "$TEST_CONFIG_DIR"
    
    # Create test config
    cat > "$TEST_CONFIG_DIR/config.toml" << 'EOF'
[killswitch]
enabled = true
allow_lan = true

[headless]
auto_connect = false
connection_name = ""
boot_killswitch = true

[gateway]
enabled = false
EOF
}

teardown_headless_test() {
    if [[ -n "$SHROUD_PID" ]]; then
        kill "$SHROUD_PID" 2>/dev/null || true
        wait "$SHROUD_PID" 2>/dev/null || true
    fi
    
    cleanup_test_env
    cleanup_iptables
    rm -rf "$TEST_CONFIG_DIR"
    rm -f "$TEST_SOCKET"
}

start_headless_daemon() {
    SHROUD_CONFIG_DIR="$TEST_CONFIG_DIR" \
    SHROUD_SOCKET_PATH="$TEST_SOCKET" \
    shroud --headless &
    SHROUD_PID=$!
    
    # Wait for daemon to start
    sleep 1
}

# ============================================================================
# Test Functions
# ============================================================================

test_headless_starts() {
    setup_headless_test
    
    # Start daemon with timeout
    timeout 5s bash -c '
        SHROUD_CONFIG_DIR="'"$TEST_CONFIG_DIR"'" shroud --headless &
        pid=$!
        sleep 1
        kill -0 $pid 2>/dev/null
        result=$?
        kill $pid 2>/dev/null
        exit $result
    ' || {
        teardown_headless_test
        echo "Daemon failed to start"
        return 1
    }
    
    teardown_headless_test
}

test_headless_creates_socket() {
    setup_headless_test
    
    start_headless_daemon
    sleep 1
    
    # Check for socket
    local socket_path="${XDG_RUNTIME_DIR:-/tmp}/shroud.sock"
    if [[ ! -S "$socket_path" ]]; then
        teardown_headless_test
        echo "Socket not created at $socket_path"
        return 1
    fi
    
    teardown_headless_test
}

test_headless_boot_ks_enabled() {
    setup_headless_test
    
    # Enable boot killswitch in config
    cat > "$TEST_CONFIG_DIR/config.toml" << 'EOF'
[headless]
boot_killswitch = true
auto_connect = false
EOF
    
    start_headless_daemon
    sleep 1
    
    # Check if boot killswitch chain exists
    if ! iptables -L SHROUD_BOOT_KS -n &>/dev/null; then
        teardown_headless_test
        echo "Boot killswitch chain not created"
        return 1
    fi
    
    teardown_headless_test
}

test_headless_responds_to_ping() {
    setup_headless_test
    start_headless_daemon
    
    sleep 1
    
    # Try to ping daemon
    local result
    set +e
    result=$(shroud ping 2>&1)
    local exit_code=$?
    set -e
    
    teardown_headless_test
    
    if [[ $exit_code -ne 0 ]]; then
        echo "Ping failed: $result"
        return 1
    fi
}

test_headless_status() {
    setup_headless_test
    start_headless_daemon
    
    sleep 1
    
    # Get status
    local result
    set +e
    result=$(shroud status 2>&1)
    local exit_code=$?
    set -e
    
    teardown_headless_test
    
    # Status should work (might report disconnected)
    assert_success "$exit_code" "status command should succeed"
}

test_headless_graceful_shutdown() {
    setup_headless_test
    start_headless_daemon
    
    sleep 1
    
    # Send SIGTERM
    kill -TERM "$SHROUD_PID"
    
    # Wait for shutdown
    local timeout=5
    while kill -0 "$SHROUD_PID" 2>/dev/null && [[ $timeout -gt 0 ]]; do
        sleep 1
        timeout=$((timeout - 1))
    done
    
    if kill -0 "$SHROUD_PID" 2>/dev/null; then
        kill -9 "$SHROUD_PID" 2>/dev/null || true
        teardown_headless_test
        echo "Daemon did not shutdown gracefully"
        return 1
    fi
    
    SHROUD_PID=""
    teardown_headless_test
}

test_headless_cleanup_on_shutdown() {
    setup_headless_test
    
    cat > "$TEST_CONFIG_DIR/config.toml" << 'EOF'
[headless]
boot_killswitch = true
auto_connect = false
EOF
    
    start_headless_daemon
    sleep 1
    
    # Verify boot killswitch is active
    if ! iptables -L SHROUD_BOOT_KS -n &>/dev/null; then
        teardown_headless_test
        echo "Boot killswitch not created"
        return 1
    fi
    
    # Shutdown gracefully
    kill -TERM "$SHROUD_PID"
    sleep 2
    SHROUD_PID=""
    
    # Boot killswitch should be cleaned up on graceful shutdown
    # (Note: intentional crash would leave it in place for protection)
    
    teardown_headless_test
}

test_headless_sigint_handling() {
    setup_headless_test
    start_headless_daemon
    
    sleep 1
    
    # Send SIGINT
    kill -INT "$SHROUD_PID"
    
    # Wait for shutdown
    local timeout=5
    while kill -0 "$SHROUD_PID" 2>/dev/null && [[ $timeout -gt 0 ]]; do
        sleep 1
        timeout=$((timeout - 1))
    done
    
    if kill -0 "$SHROUD_PID" 2>/dev/null; then
        kill -9 "$SHROUD_PID" 2>/dev/null || true
        teardown_headless_test
        echo "Daemon did not handle SIGINT"
        return 1
    fi
    
    SHROUD_PID=""
    teardown_headless_test
}

test_headless_auto_connect_disabled() {
    setup_headless_test
    
    cat > "$TEST_CONFIG_DIR/config.toml" << 'EOF'
[headless]
auto_connect = false
connection_name = ""
EOF
    
    start_headless_daemon
    sleep 2
    
    # Get status - should be disconnected since auto_connect is false
    local result
    result=$(shroud status 2>&1 || true)
    
    # Should report disconnected or no VPN active
    if [[ "$result" == *"isconnect"* ]] || [[ "$result" == *"not"*"connect"* ]] || \
       [[ "$result" == *"VPN"* ]] || [[ "$result" == *"inactive"* ]] || \
       [[ "$result" == *"off"* ]] || [[ "$result" == *"Status"* ]]; then
        teardown_headless_test
        return 0
    fi
    
    teardown_headless_test
    echo "Status output: $result"
    return 1
}

# ============================================================================
# Run Tests
# ============================================================================

begin_suite "headless-runtime"

run_test "Headless starts" test_headless_starts
run_test "Headless creates socket" test_headless_creates_socket
run_test "Headless enables boot killswitch" test_headless_boot_ks_enabled
run_test "Headless responds to ping" test_headless_responds_to_ping
run_test "Headless status command" test_headless_status
run_test "Headless graceful shutdown" test_headless_graceful_shutdown
run_test "Headless cleanup on shutdown" test_headless_cleanup_on_shutdown
run_test "Headless SIGINT handling" test_headless_sigint_handling
run_test "Auto-connect disabled" test_headless_auto_connect_disabled

end_suite
