#!/usr/bin/env bash
# E2E Test Library - Common functions for all E2E tests
#
# Usage: source tests/e2e/lib.sh

set -euo pipefail

# ============================================================================
# Configuration
# ============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RESULTS_DIR="${SCRIPT_DIR}/results"
SHROUD_BIN="${PROJECT_ROOT}/target/release/shroud"

# Test counters
declare -g TESTS_PASSED=0
declare -g TESTS_FAILED=0
declare -g TESTS_SKIPPED=0
declare -g CURRENT_SUITE=""
declare -g TEST_RESULTS=()

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# ============================================================================
# Setup and Teardown
# ============================================================================

setup_test_env() {
    mkdir -p "$RESULTS_DIR"
    
    # Check for shroud binary
    if [[ ! -x "$SHROUD_BIN" ]]; then
        # Try debug build
        SHROUD_BIN="${PROJECT_ROOT}/target/debug/shroud"
        if [[ ! -x "$SHROUD_BIN" ]]; then
            echo -e "${RED}ERROR: Shroud binary not found. Run 'cargo build --release' first.${NC}"
            exit 1
        fi
    fi
    
    export SHROUD_BIN
    export PATH="${PROJECT_ROOT}/target/release:${PROJECT_ROOT}/target/debug:$PATH"
}

cleanup_test_env() {
    # Kill any lingering shroud processes from tests
    pkill -f "shroud.*--headless" 2>/dev/null || true
    
    # Clean up IPC socket
    rm -f "${XDG_RUNTIME_DIR:-/tmp}/shroud.sock" 2>/dev/null || true
}

# ============================================================================
# Test Framework
# ============================================================================

begin_suite() {
    local suite_name="$1"
    CURRENT_SUITE="$suite_name"
    TESTS_PASSED=0
    TESTS_FAILED=0
    TESTS_SKIPPED=0
    TEST_RESULTS=()
    
    echo ""
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}  Test Suite: ${suite_name}${NC}"
    echo -e "${BLUE}========================================${NC}"
    echo ""
}

end_suite() {
    local total=$((TESTS_PASSED + TESTS_FAILED + TESTS_SKIPPED))
    
    echo ""
    echo -e "${BLUE}----------------------------------------${NC}"
    echo -e "  Results: ${GREEN}${TESTS_PASSED} passed${NC}, ${RED}${TESTS_FAILED} failed${NC}, ${YELLOW}${TESTS_SKIPPED} skipped${NC}"
    echo -e "${BLUE}----------------------------------------${NC}"
    
    # Write JSON results
    write_json_results
    
    # Return failure if any tests failed
    [[ $TESTS_FAILED -eq 0 ]]
}

run_test() {
    local test_name="$1"
    local test_func="$2"
    local start_time
    local end_time
    local duration
    local result
    local error_msg=""
    
    start_time=$(date +%s%3N)
    
    echo -n "  Testing: ${test_name}... "
    
    # Run the test function
    set +e
    error_msg=$(eval "$test_func" 2>&1)
    result=$?
    set -e
    
    end_time=$(date +%s%3N)
    duration=$((end_time - start_time))
    
    if [[ $result -eq 0 ]]; then
        echo -e "${GREEN}PASS${NC} (${duration}ms)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        TEST_RESULTS+=("{\"name\":\"${test_name}\",\"result\":\"pass\",\"duration_ms\":${duration}}")
    elif [[ $result -eq 77 ]]; then
        echo -e "${YELLOW}SKIP${NC}"
        TESTS_SKIPPED=$((TESTS_SKIPPED + 1))
        TEST_RESULTS+=("{\"name\":\"${test_name}\",\"result\":\"skip\",\"reason\":\"${error_msg}\"}")
    else
        echo -e "${RED}FAIL${NC}"
        if [[ -n "$error_msg" ]]; then
            echo -e "    ${RED}Error: ${error_msg}${NC}"
        fi
        TESTS_FAILED=$((TESTS_FAILED + 1))
        TEST_RESULTS+=("{\"name\":\"${test_name}\",\"result\":\"fail\",\"error\":\"${error_msg}\",\"duration_ms\":${duration}}")
    fi
}

skip_test() {
    local reason="${1:-no reason given}"
    echo "$reason" >&2
    exit 77
}

require_root() {
    if [[ $EUID -ne 0 ]]; then
        skip_test "requires root"
    fi
}

require_command() {
    local cmd="$1"
    if ! command -v "$cmd" &>/dev/null; then
        skip_test "requires $cmd"
    fi
}

# ============================================================================
# Assertions
# ============================================================================

assert_eq() {
    local expected="$1"
    local actual="$2"
    local msg="${3:-}"
    
    if [[ "$expected" != "$actual" ]]; then
        echo "Expected: '$expected', got: '$actual'. $msg" >&2
        return 1
    fi
}

assert_ne() {
    local not_expected="$1"
    local actual="$2"
    local msg="${3:-}"
    
    if [[ "$not_expected" == "$actual" ]]; then
        echo "Did not expect: '$not_expected'. $msg" >&2
        return 1
    fi
}

assert_contains() {
    local haystack="$1"
    local needle="$2"
    local msg="${3:-}"
    
    if [[ "$haystack" != *"$needle"* ]]; then
        echo "Expected to contain: '$needle'. $msg" >&2
        return 1
    fi
}

assert_not_contains() {
    local haystack="$1"
    local needle="$2"
    local msg="${3:-}"
    
    if [[ "$haystack" == *"$needle"* ]]; then
        echo "Expected NOT to contain: '$needle'. $msg" >&2
        return 1
    fi
}

assert_success() {
    local exit_code="$1"
    local msg="${2:-}"
    
    if [[ "$exit_code" -ne 0 ]]; then
        echo "Expected success (0), got: $exit_code. $msg" >&2
        return 1
    fi
}

assert_failure() {
    local exit_code="$1"
    local msg="${2:-}"
    
    if [[ "$exit_code" -eq 0 ]]; then
        echo "Expected failure (non-zero), got: 0. $msg" >&2
        return 1
    fi
}

assert_file_exists() {
    local file="$1"
    local msg="${2:-}"
    
    if [[ ! -f "$file" ]]; then
        echo "File does not exist: $file. $msg" >&2
        return 1
    fi
}

assert_chain_exists() {
    local chain="$1"
    local msg="${2:-}"
    
    if ! sudo iptables -L "$chain" -n &>/dev/null; then
        echo "iptables chain does not exist: $chain. $msg" >&2
        return 1
    fi
}

assert_chain_not_exists() {
    local chain="$1"
    local msg="${2:-}"
    
    if sudo iptables -L "$chain" -n &>/dev/null; then
        echo "iptables chain should not exist: $chain. $msg" >&2
        return 1
    fi
}

# ============================================================================
# Helpers
# ============================================================================

shroud() {
    "$SHROUD_BIN" "$@"
}

wait_for_file() {
    local file="$1"
    local timeout="${2:-5}"
    local elapsed=0
    
    while [[ ! -e "$file" && $elapsed -lt $timeout ]]; do
        sleep 0.1
        elapsed=$((elapsed + 1))
    done
    
    [[ -e "$file" ]]
}

wait_for_daemon() {
    local timeout="${1:-10}"
    local elapsed=0
    
    while ! shroud ping &>/dev/null && [[ $elapsed -lt $((timeout * 10)) ]]; do
        sleep 0.1
        ((elapsed++))
    done
    
    shroud ping &>/dev/null
}

wait_for_daemon_stop() {
    local timeout="${1:-5}"
    local elapsed=0
    
    while shroud ping &>/dev/null && [[ $elapsed -lt $((timeout * 10)) ]]; do
        sleep 0.1
        ((elapsed++))
    done
    
    ! shroud ping &>/dev/null
}

get_ip_forward_state() {
    cat /proc/sys/net/ipv4/ip_forward
}

write_json_results() {
    local timestamp
    timestamp=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    
    local tests_json
    tests_json=$(IFS=,; echo "${TEST_RESULTS[*]}")
    
    cat > "${RESULTS_DIR}/${CURRENT_SUITE}.json" << EOF
{
  "suite": "${CURRENT_SUITE}",
  "passed": ${TESTS_PASSED},
  "failed": ${TESTS_FAILED},
  "skipped": ${TESTS_SKIPPED},
  "tests": [${tests_json}],
  "timestamp": "${timestamp}"
}
EOF
}

# ============================================================================
# Cleanup Helpers
# ============================================================================

cleanup_iptables() {
    # Remove shroud-related chains
    for chain in SHROUD_KILLSWITCH SHROUD_BOOT_KS SHROUD_GATEWAY SHROUD_GATEWAY_KS; do
        sudo iptables -D OUTPUT -j "$chain" 2>/dev/null || true
        sudo iptables -D FORWARD -j "$chain" 2>/dev/null || true
        sudo iptables -F "$chain" 2>/dev/null || true
        sudo iptables -X "$chain" 2>/dev/null || true
        
        sudo ip6tables -D OUTPUT -j "$chain" 2>/dev/null || true
        sudo ip6tables -D FORWARD -j "$chain" 2>/dev/null || true
        sudo ip6tables -F "$chain" 2>/dev/null || true
        sudo ip6tables -X "$chain" 2>/dev/null || true
    done
    
    # Remove NAT rules
    while sudo iptables -t nat -D POSTROUTING -j MASQUERADE 2>/dev/null; do :; done
}

cleanup_test_interfaces() {
    sudo ip link del tun-test 2>/dev/null || true
    sudo ip link del wg-test 2>/dev/null || true
}

# Initialize when sourced
setup_test_env
