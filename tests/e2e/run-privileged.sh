#!/usr/bin/env bash
# Run Privileged E2E Tests
#
# This script runs E2E tests that require root privileges.
# It should be run with sudo or as root.
#
# Usage: sudo ./tests/e2e/run-privileged.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Check for root
if [[ $EUID -ne 0 ]]; then
    echo "This script must be run as root (use sudo)"
    exit 1
fi

# Export for lib.sh
export RUN_PRIVILEGED=true

# Source library
source "${SCRIPT_DIR}/lib.sh"

echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║       Shroud Privileged E2E Tests                          ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Build first
echo -e "${BLUE}Building Shroud...${NC}"
cd "$PROJECT_ROOT"
cargo build --release 2>&1 | tail -3
echo ""

# Track results
TOTAL_PASSED=0
TOTAL_FAILED=0
TOTAL_SKIPPED=0
SUITE_RESULTS=()

run_suite() {
    local suite_script="$1"
    local suite_name
    suite_name=$(basename "$suite_script" .sh | sed 's/test-//')
    
    echo -e "\n${BLUE}>>> Running ${suite_name} tests...${NC}\n"
    
    if bash "$suite_script"; then
        SUITE_RESULTS+=("${GREEN}✓ ${suite_name}${NC}")
    else
        SUITE_RESULTS+=("${RED}✗ ${suite_name}${NC}")
    fi
    
    # Collect results
    local json_file="${RESULTS_DIR}/${suite_name}.json"
    if [[ -f "$json_file" ]]; then
        local passed failed skipped
        if command -v jq &>/dev/null; then
            passed=$(jq -r '.passed' "$json_file" 2>/dev/null || echo 0)
            failed=$(jq -r '.failed' "$json_file" 2>/dev/null || echo 0)
            skipped=$(jq -r '.skipped' "$json_file" 2>/dev/null || echo 0)
        else
            # Fallback: parse JSON with grep/sed
            passed=$(grep -o '"passed": *[0-9]*' "$json_file" | grep -o '[0-9]*' || echo 0)
            failed=$(grep -o '"failed": *[0-9]*' "$json_file" | grep -o '[0-9]*' || echo 0)
            skipped=$(grep -o '"skipped": *[0-9]*' "$json_file" | grep -o '[0-9]*' || echo 0)
        fi
        
        TOTAL_PASSED=$((TOTAL_PASSED + passed))
        TOTAL_FAILED=$((TOTAL_FAILED + failed))
        TOTAL_SKIPPED=$((TOTAL_SKIPPED + skipped))
    fi
}

# Run all privileged test suites
run_suite "${SCRIPT_DIR}/test-boot-killswitch.sh"
run_suite "${SCRIPT_DIR}/test-headless-runtime.sh"
run_suite "${SCRIPT_DIR}/test-gateway.sh"
run_suite "${SCRIPT_DIR}/test-cleanup.sh"

# Final cleanup
cleanup_iptables
cleanup_test_interfaces

# Summary
echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║              Privileged Tests Summary                      ║${NC}"
echo -e "${BLUE}╠════════════════════════════════════════════════════════════╣${NC}"
for result in "${SUITE_RESULTS[@]}"; do
    printf "${BLUE}║${NC} %-58b ${BLUE}║${NC}\n" "$result"
done
echo -e "${BLUE}╠════════════════════════════════════════════════════════════╣${NC}"
printf "${BLUE}║${NC} ${GREEN}Passed: %3d${NC}  ${RED}Failed: %3d${NC}  ${YELLOW}Skipped: %3d${NC}  Total: %3d   ${BLUE}║${NC}\n" \
    "$TOTAL_PASSED" "$TOTAL_FAILED" "$TOTAL_SKIPPED" "$((TOTAL_PASSED + TOTAL_FAILED + TOTAL_SKIPPED))"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Exit with failure if any tests failed
[[ $TOTAL_FAILED -eq 0 ]]
