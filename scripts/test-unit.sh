#!/usr/bin/env bash
# Run unit tests only
#
# Usage: ./scripts/test-unit.sh [options]
#
# Options:
#   --verbose       Show detailed output
#   --nocapture     Show test output (println!)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/common.sh"

EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --verbose)
            SHROUD_TEST_VERBOSE=true
            shift
            ;;
        --nocapture)
            EXTRA_ARGS+=("--nocapture")
            shift
            ;;
        *)
            EXTRA_ARGS+=("$1")
            shift
            ;;
    esac
done

log_section "Unit Tests"

start_timer

ensure_binary

# Run unit tests
output=$(run_cargo_test "--bins" "${EXTRA_ARGS[@]:-}" 2>&1) || {
    echo "$output"
    log_fail "Unit tests failed"
    exit 1
}

result_str=$(parse_cargo_test_output "$output")
passed=$(echo "$result_str" | awk '{print $1}')
failed=$(echo "$result_str" | awk '{print $2}')
ignored=$(echo "$result_str" | awk '{print $3}')

echo "$output" | tail -10

elapsed=$(get_elapsed)
write_json_result "unit" "$passed" "$failed" "$ignored" "$elapsed"

log_pass "Unit tests completed ($passed passed, $ignored ignored) in ${elapsed}s"
