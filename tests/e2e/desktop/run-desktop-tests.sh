#!/usr/bin/env bash
#
# Desktop Mode E2E Test Runner
#
# Usage:
#   ./run-desktop-tests.sh              # Run non-privileged tests
#   sudo ./run-desktop-tests.sh --privileged   # Run all tests
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
E2E_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT_ROOT="$(dirname "$(dirname "$E2E_DIR")")"
RESULTS_DIR="${SCRIPT_DIR}/results"

# Source test helpers
source "${E2E_DIR}/lib.sh" 2>/dev/null || {
    # Inline minimal helpers if file doesn't exist
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    NC='\033[0m'
    
    log_pass() { echo -e "${GREEN}✓ PASS${NC}: $1"; }
    log_fail() { echo -e "${RED}✗ FAIL${NC}: $1"; }
    log_info() { echo -e "${YELLOW}→${NC} $1"; }
    log_skip() { echo -e "${YELLOW}○ SKIP${NC}: $1"; }
}

# Flags
PRIVILEGED=false
QUICK=false
VERBOSE=false
SUITE=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --privileged|-p) PRIVILEGED=true; shift ;;
        --quick|-q) QUICK=true; shift ;;
        --verbose|-v) VERBOSE=true; shift ;;
        --suite|-s) SUITE="$2"; shift 2 ;;
        --help|-h)
            echo "Desktop E2E Test Runner"
            echo ""
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --privileged, -p   Run tests requiring root"
            echo "  --quick, -q        Skip slow tests"
            echo "  --verbose, -v      Show detailed output"
            echo "  --suite, -s NAME   Run only specified suite"
            echo "  --help, -h         Show this help"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Setup
mkdir -p "$RESULTS_DIR"
SHROUD_BIN="${PROJECT_ROOT}/target/release/shroud"

# Build if needed
if [[ ! -f "$SHROUD_BIN" ]]; then
    log_info "Building Shroud..."
    (cd "$PROJECT_ROOT" && cargo build --release)
fi

# Test suites
SUITES=(
    "test-mode-detection.sh"
    "test-ipc.sh"
    "test-commands.sh"
    "test-tray-actions.sh"
    "test-state.sh"
    "test-stress.sh"
)

PRIVILEGED_SUITES=(
    "test-killswitch.sh"
    "test-cleanup.sh"
)

echo ""
echo "========================================"
echo "  SHROUD DESKTOP E2E TESTS"
echo "========================================"
echo ""
echo "  Binary: $SHROUD_BIN"
echo "  Privileged: $PRIVILEGED"
echo ""

TOTAL_PASSED=0
TOTAL_FAILED=0
TOTAL_SKIPPED=0

run_suite() {
    local suite="$1"
    local suite_path="${SCRIPT_DIR}/${suite}"
    
    if [[ -n "$SUITE" ]] && [[ "$suite" != *"$SUITE"* ]]; then
        return
    fi
    
    if [[ ! -f "$suite_path" ]]; then
        log_skip "$suite (not found)"
        ((TOTAL_SKIPPED++))
        return
    fi
    
    chmod +x "$suite_path"
    
    echo ""
    echo "────────────────────────────────────────"
    echo "  Running: $suite"
    echo "────────────────────────────────────────"
    
    if SHROUD_BIN="$SHROUD_BIN" RESULTS_DIR="$RESULTS_DIR" "$suite_path"; then
        log_pass "$suite"
        ((TOTAL_PASSED++))
    else
        log_fail "$suite"
        ((TOTAL_FAILED++))
    fi
}

# Run non-privileged suites
for suite in "${SUITES[@]}"; do
    run_suite "$suite"
done

# Run privileged suites if requested
if [[ "$PRIVILEGED" == "true" ]]; then
    if [[ $EUID -ne 0 ]]; then
        log_fail "Privileged tests require root. Use: sudo $0 --privileged"
    else
        for suite in "${PRIVILEGED_SUITES[@]}"; do
            run_suite "$suite"
        done
    fi
else
    log_info "Skipping privileged tests (use --privileged to run)"
    TOTAL_SKIPPED=$((TOTAL_SKIPPED + ${#PRIVILEGED_SUITES[@]}))
fi

# Summary
echo ""
echo "========================================"
echo "  RESULTS"
echo "========================================"
echo ""
echo -e "  Passed:  ${GREEN}${TOTAL_PASSED}${NC}"
echo -e "  Failed:  ${RED}${TOTAL_FAILED}${NC}"
echo -e "  Skipped: ${YELLOW}${TOTAL_SKIPPED}${NC}"
echo ""

# Write JSON results
cat > "${RESULTS_DIR}/desktop-summary.json" << EOF
{
    "suite": "desktop",
    "passed": $TOTAL_PASSED,
    "failed": $TOTAL_FAILED,
    "skipped": $TOTAL_SKIPPED,
    "timestamp": "$(date -Iseconds)"
}
EOF

[[ $TOTAL_FAILED -eq 0 ]]
