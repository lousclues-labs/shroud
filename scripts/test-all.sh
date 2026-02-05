#!/usr/bin/env bash
# Run all tests (format, lint, unit, integration, security)
#
# Usage: ./scripts/test-all.sh [options]
#
# Options:
#   --privileged    Include privileged tests (requires sudo)
#   --e2e           Also run E2E tests
#   --verbose       Show detailed output
#   --json          Output results as JSON only

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/common.sh"

# Options
RUN_PRIVILEGED=false
RUN_E2E=false
JSON_ONLY=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --privileged)
            RUN_PRIVILEGED=true
            shift
            ;;
        --e2e)
            RUN_E2E=true
            shift
            ;;
        --verbose)
            SHROUD_TEST_VERBOSE=true
            shift
            ;;
        --json)
            JSON_ONLY=true
            shift
            ;;
        *)
            log_error "Unknown option: $1"
            exit 1
            ;;
    esac
done

start_timer

# Track overall results
TOTAL_PASSED=0
TOTAL_FAILED=0
TOTAL_SKIPPED=0

echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                 Shroud Test Suite                          ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# ============================================================================
# Stage 1: Quick checks
# ============================================================================

log_section "Stage 1: Quick Checks"

log_info "Format check..."
if ! cargo fmt --all --check 2>&1; then
    log_fail "Format check failed - run 'cargo fmt'"
    ((TOTAL_FAILED++))
    exit 1
fi
log_pass "Format OK"
((TOTAL_PASSED++))

log_info "Clippy lints..."
if ! cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -5; then
    log_fail "Clippy found issues"
    ((TOTAL_FAILED++))
    exit 1
fi
log_pass "Clippy OK"
((TOTAL_PASSED++))

# ============================================================================
# Stage 2: Unit tests
# ============================================================================

log_section "Stage 2: Unit Tests"

unit_output=$(cargo test --bins --all-features -- --test-threads="${SHROUD_TEST_THREADS}" 2>&1) || {
    echo "$unit_output"
    log_fail "Unit tests failed"
    ((TOTAL_FAILED++))
    exit 1
}
result_str=$(parse_cargo_test_output "$unit_output")
passed=$(echo "$result_str" | awk '{print $1}')
failed=$(echo "$result_str" | awk '{print $2}')
ignored=$(echo "$result_str" | awk '{print $3}')
echo "$unit_output" | tail -5
log_pass "Unit tests passed ($passed tests, $ignored ignored)"
TOTAL_PASSED=$((TOTAL_PASSED + passed))
TOTAL_SKIPPED=$((TOTAL_SKIPPED + ignored))

# ============================================================================
# Stage 3: Integration tests
# ============================================================================

log_section "Stage 3: Integration Tests"

int_output=$(cargo test --test integration --all-features -- --test-threads="${SHROUD_TEST_THREADS}" 2>&1) || {
    echo "$int_output"
    log_fail "Integration tests failed"
    ((TOTAL_FAILED++))
    exit 1
}
result_str=$(parse_cargo_test_output "$int_output")
passed=$(echo "$result_str" | awk '{print $1}')
failed=$(echo "$result_str" | awk '{print $2}')
ignored=$(echo "$result_str" | awk '{print $3}')
echo "$int_output" | tail -5
log_pass "Integration tests passed ($passed tests, $ignored ignored)"
TOTAL_PASSED=$((TOTAL_PASSED + passed))
TOTAL_SKIPPED=$((TOTAL_SKIPPED + ignored))

# ============================================================================
# Stage 4: Security tests (non-privileged)
# ============================================================================

log_section "Stage 4: Security Tests (non-privileged)"

sec_output=$(cargo test --test security --all-features -- --test-threads="${SHROUD_TEST_THREADS}" 2>&1) || {
    echo "$sec_output"
    log_fail "Security tests failed"
    ((TOTAL_FAILED++))
    exit 1
}
result_str=$(parse_cargo_test_output "$sec_output")
passed=$(echo "$result_str" | awk '{print $1}')
failed=$(echo "$result_str" | awk '{print $2}')
ignored=$(echo "$result_str" | awk '{print $3}')
echo "$sec_output" | tail -5
log_pass "Security tests passed ($passed tests, $ignored ignored)"
TOTAL_PASSED=$((TOTAL_PASSED + passed))
TOTAL_SKIPPED=$((TOTAL_SKIPPED + ignored))

# ============================================================================
# Stage 5: Privileged tests (optional)
# ============================================================================

if $RUN_PRIVILEGED; then
    log_section "Stage 5: Security Tests (privileged)"
    
    if [[ $EUID -ne 0 ]]; then
        log_info "Re-running with sudo..."
        priv_output=$(sudo -E cargo test --test security --all-features -- --ignored --test-threads=1 2>&1) || {
            echo "$priv_output"
            log_fail "Privileged tests failed"
            ((TOTAL_FAILED++))
        }
    else
        priv_output=$(cargo test --test security --all-features -- --ignored --test-threads=1 2>&1) || {
            echo "$priv_output"
            log_fail "Privileged tests failed"
            ((TOTAL_FAILED++))
        }
    fi
    
    result_str=$(parse_cargo_test_output "$priv_output")
    passed=$(echo "$result_str" | awk '{print $1}')
    failed=$(echo "$result_str" | awk '{print $2}')
    ignored=$(echo "$result_str" | awk '{print $3}')
    echo "$priv_output" | tail -5
    log_pass "Privileged tests passed ($passed tests)"
    TOTAL_PASSED=$((TOTAL_PASSED + passed))
fi

# ============================================================================
# Stage 6: E2E tests (optional)
# ============================================================================

if $RUN_E2E; then
    log_section "Stage 6: E2E Tests"
    
    log_info "Building release binary..."
    cargo build --release --quiet
    
    if $RUN_PRIVILEGED; then
        "${PROJECT_ROOT}/tests/e2e/run-all.sh" --privileged || {
            log_fail "E2E tests failed"
            ((TOTAL_FAILED++))
        }
    else
        "${PROJECT_ROOT}/tests/e2e/run-all.sh" || {
            log_fail "E2E tests failed"
            ((TOTAL_FAILED++))
        }
    fi
fi

# ============================================================================
# Results
# ============================================================================

elapsed=$(get_elapsed)

# Write aggregated JSON results
write_json_result "all" "$TOTAL_PASSED" "$TOTAL_FAILED" "$TOTAL_SKIPPED" "$elapsed"

echo ""
if [[ $TOTAL_FAILED -eq 0 ]]; then
    echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║               All Tests Passed! ✓                          ║${NC}"
    echo -e "${GREEN}╠════════════════════════════════════════════════════════════╣${NC}"
    printf "${GREEN}║  Passed: %-5d  Failed: %-5d  Skipped: %-5d  Time: %3ds   ║${NC}\n" \
        "$TOTAL_PASSED" "$TOTAL_FAILED" "$TOTAL_SKIPPED" "$elapsed"
    echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
else
    echo -e "${RED}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║               Some Tests Failed ✗                          ║${NC}"
    echo -e "${RED}╠════════════════════════════════════════════════════════════╣${NC}"
    printf "${RED}║  Passed: %-5d  Failed: %-5d  Skipped: %-5d  Time: %3ds   ║${NC}\n" \
        "$TOTAL_PASSED" "$TOTAL_FAILED" "$TOTAL_SKIPPED" "$elapsed"
    echo -e "${RED}╚════════════════════════════════════════════════════════════╝${NC}"
    exit 1
fi
echo ""
