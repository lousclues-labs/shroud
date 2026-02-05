#!/usr/bin/env bash
# Quick test for development iteration
# Runs only unit tests for fast feedback
#
# Usage: ./scripts/test-quick.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/common.sh"

log_section "Quick Unit Tests"

start_timer

ensure_binary

# Run unit tests with maximum parallelism
output=$(cargo test --bins --all-features -- --test-threads="${SHROUD_TEST_THREADS}" 2>&1) || {
    echo "$output"
    log_fail "Quick tests failed"
    exit 1
}

# Parse results
result_str=$(parse_cargo_test_output "$output")
passed=$(echo "$result_str" | awk '{print $1}')
failed=$(echo "$result_str" | awk '{print $2}')
ignored=$(echo "$result_str" | awk '{print $3}')

echo "$output" | tail -5

elapsed=$(get_elapsed)
write_json_result "quick" "$passed" "$failed" "$ignored" "$elapsed"

log_pass "Quick tests completed ($passed passed, $ignored ignored) in ${elapsed}s"
