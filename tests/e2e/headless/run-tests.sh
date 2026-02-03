#!/usr/bin/env bash
# Headless E2E Test Runner
#
# Usage: ./tests/e2e/headless/run-tests.sh [options]
#
# Options:
#   --privileged    Run tests requiring root
#   --unprivileged  Run only unprivileged tests
#   --quick         Skip slow tests
#   --verbose       Show detailed output

set -euo pipefail

# Save our script directory before sourcing test-helpers.sh (which redefines SCRIPT_DIR)
HEADLESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
E2E_DIR="$(dirname "$HEADLESS_DIR")"

# shellcheck source=../lib/test-helpers.sh
source "${E2E_DIR}/lib/test-helpers.sh"

# ============================================================================
# Parse Arguments
# ============================================================================

RUN_PRIVILEGED=false
RUN_UNPRIVILEGED=true
QUICK_MODE=false
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --privileged)
            RUN_PRIVILEGED=true
            shift
            ;;
        --unprivileged)
            RUN_PRIVILEGED=false
            RUN_UNPRIVILEGED=true
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
# Run Tests
# ============================================================================

echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║           Headless E2E Tests                               ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

PASSED=0
FAILED=0
SKIPPED=0

run_test() {
    local script="$1"
    local name
    name=$(basename "$script" .sh | sed 's/test-//')
    
    echo -e "\n${BLUE}>>> Running ${name}...${NC}"
    
    if bash "$script"; then
        echo -e "${GREEN}✓ ${name} passed${NC}"
        ((PASSED++))
    else
        echo -e "${RED}✗ ${name} failed${NC}"
        ((FAILED++))
    fi
}

# Unprivileged tests (always run)
if $RUN_UNPRIVILEGED; then
    run_test "${HEADLESS_DIR}/test-mode-detection.sh"
    run_test "${HEADLESS_DIR}/test-gateway-detection.sh"
fi

# Privileged tests (require root)
if $RUN_PRIVILEGED; then
    if [[ $EUID -ne 0 ]]; then
        echo -e "${RED}Error: Privileged tests require root. Run with sudo.${NC}"
        exit 1
    fi
    
    run_test "${HEADLESS_DIR}/test-boot-killswitch.sh"
    run_test "${HEADLESS_DIR}/test-headless-runtime.sh"
    run_test "${HEADLESS_DIR}/test-gateway.sh"
    run_test "${HEADLESS_DIR}/test-cleanup.sh"
fi

# ============================================================================
# Summary
# ============================================================================

echo ""
echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
echo -e "${GREEN}Passed: ${PASSED}${NC}  ${RED}Failed: ${FAILED}${NC}  ${YELLOW}Skipped: ${SKIPPED}${NC}"
echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"

# Write results
mkdir -p "${E2E_DIR}/results"
cat > "${E2E_DIR}/results/headless.json" << EOF
{
  "passed": ${PASSED},
  "failed": ${FAILED},
  "skipped": ${SKIPPED},
  "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
}
EOF

[[ $FAILED -eq 0 ]]
