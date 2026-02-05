#!/usr/bin/env bash
# Run privileged integration tests
# Must be run with sudo or as root
#
# Usage: sudo ./scripts/run-privileged-tests.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/common.sh"

check_root

log_section "Privileged Integration Tests"

log_info "These tests require root access for iptables manipulation."
echo ""

start_timer

# Ensure cargo is available
export PATH="${HOME}/.cargo/bin:$PATH"

# Run ignored tests with sudo (already running as root)
output=$(cargo test --all-features -- --ignored --test-threads=1 2>&1) || {
    echo "$output"
    log_fail "Privileged tests failed"
    exit 1
}

result_str=$(parse_cargo_test_output "$output")
passed=$(echo "$result_str" | awk '{print $1}')
failed=$(echo "$result_str" | awk '{print $2}')
ignored=$(echo "$result_str" | awk '{print $3}')

echo "$output" | tail -10

elapsed=$(get_elapsed)
write_json_result "privileged" "$passed" "$failed" "0" "$elapsed"

log_pass "All privileged tests passed ($passed tests) in ${elapsed}s"
