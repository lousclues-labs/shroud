#!/usr/bin/env bash
# E2E Test Runner - Runs all E2E tests
#
# Usage: ./tests/e2e/run-all.sh [options]
#
# Options:
#   --privileged    Run tests requiring root (default: skip)
#   --quick         Skip slow tests
#   --verbose       Show detailed output
#   --suite NAME    Run only the specified suite

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

# ============================================================================
# Parse Arguments
# ============================================================================

RUN_PRIVILEGED=false
QUICK_MODE=false
VERBOSE=false
SELECTED_SUITE=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --privileged)
            RUN_PRIVILEGED=true
            shift
            ;;
        --quick)
            QUICK_MODE=true
            shift
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --suite)
            SELECTED_SUITE="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

export RUN_PRIVILEGED
export QUICK_MODE
export VERBOSE

# ============================================================================
# Build Project
# ============================================================================

echo -e "${BLUE}Building Shroud...${NC}"
cd "$PROJECT_ROOT"
cargo build --release 2>&1 | tail -5
echo ""

# ============================================================================
# Collect Results
# ============================================================================

TOTAL_PASSED=0
TOTAL_FAILED=0
TOTAL_SKIPPED=0
SUITE_RESULTS=()

run_suite() {
    local suite_script="$1"
    local suite_name
    suite_name=$(basename "$suite_script" .sh | sed 's/test-//')
    
    if [[ -n "$SELECTED_SUITE" && "$suite_name" != "$SELECTED_SUITE" ]]; then
        return 0
    fi
    
    echo -e "\n${BLUE}>>> Running ${suite_name} tests...${NC}\n"
    
    if bash "$suite_script"; then
        SUITE_RESULTS+=("${GREEN}✓ ${suite_name}${NC}")
    else
        SUITE_RESULTS+=("${RED}✗ ${suite_name}${NC}")
    fi
    
    # Collect results from JSON if available
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

# ============================================================================
# Run Test Suites
# ============================================================================

echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║           Shroud E2E Test Suite                            ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Non-privileged tests
run_suite "${SCRIPT_DIR}/test-mode-detection.sh"
run_suite "${SCRIPT_DIR}/test-gateway-detection.sh"

# Privileged tests (require root)
if $RUN_PRIVILEGED; then
    if [[ $EUID -ne 0 ]]; then
        echo -e "${YELLOW}Warning: Re-running with sudo for privileged tests...${NC}"
        exec sudo -E "$0" --privileged "${@:2}"
    fi
    
    run_suite "${SCRIPT_DIR}/test-boot-killswitch.sh"
    run_suite "${SCRIPT_DIR}/test-headless-runtime.sh"
    run_suite "${SCRIPT_DIR}/test-gateway.sh"
    run_suite "${SCRIPT_DIR}/test-cleanup.sh"
else
    echo -e "${YELLOW}Skipping privileged tests (use --privileged to run)${NC}"
fi

# ============================================================================
# Summary
# ============================================================================

echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                    Final Summary                           ║${NC}"
echo -e "${BLUE}╠════════════════════════════════════════════════════════════╣${NC}"
for result in "${SUITE_RESULTS[@]}"; do
    printf "${BLUE}║${NC} %-58b ${BLUE}║${NC}\n" "$result"
done
echo -e "${BLUE}╠════════════════════════════════════════════════════════════╣${NC}"
printf "${BLUE}║${NC} ${GREEN}Passed: %3d${NC}  ${RED}Failed: %3d${NC}  ${YELLOW}Skipped: %3d${NC}  Total: %3d   ${BLUE}║${NC}\n" \
    "$TOTAL_PASSED" "$TOTAL_FAILED" "$TOTAL_SKIPPED" "$((TOTAL_PASSED + TOTAL_FAILED + TOTAL_SKIPPED))"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Write combined results
cat > "${RESULTS_DIR}/summary.json" << EOF
{
  "total_passed": ${TOTAL_PASSED},
  "total_failed": ${TOTAL_FAILED},
  "total_skipped": ${TOTAL_SKIPPED},
  "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
}
EOF

# Exit with failure if any tests failed
[[ $TOTAL_FAILED -eq 0 ]]
