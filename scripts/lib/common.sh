#!/usr/bin/env bash
# Common library for all Shroud test scripts
# Source this at the top of every script in scripts/
#
# Usage: source "${SCRIPT_DIR}/lib/common.sh"

set -euo pipefail
IFS=$'\n\t'

# ============================================================================
# Configuration
# ============================================================================

# Determine paths (SCRIPT_DIR must be set by sourcing script)
if [[ -z "${SCRIPT_DIR:-}" ]]; then
    echo "ERROR: SCRIPT_DIR must be set before sourcing lib/common.sh" >&2
    exit 1
fi

PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SHROUD_BIN="${SHROUD_BIN:-${PROJECT_ROOT}/target/release/shroud}"
RESULTS_DIR="${RESULTS_DIR:-${PROJECT_ROOT}/target/test-results}"

# Test configuration (override via environment)
SHROUD_TEST_THREADS="${SHROUD_TEST_THREADS:-4}"
SHROUD_TEST_TIMEOUT="${SHROUD_TEST_TIMEOUT:-300}"
SHROUD_TEST_VERBOSE="${SHROUD_TEST_VERBOSE:-false}"

# ============================================================================
# Colors (only if terminal supports it)
# ============================================================================

if [[ -t 1 ]] && [[ "${TERM:-dumb}" != "dumb" ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' BLUE='' BOLD='' NC=''
fi

# ============================================================================
# Logging
# ============================================================================

log_info()    { echo -e "${BLUE}→${NC} $*"; }
log_pass()    { echo -e "${GREEN}✓ PASS${NC}: $*"; }
log_fail()    { echo -e "${RED}✗ FAIL${NC}: $*"; }
log_skip()    { echo -e "${YELLOW}⊘ SKIP${NC}: $*"; }
log_warn()    { echo -e "${YELLOW}⚠ WARN${NC}: $*"; }
log_section() { echo -e "\n${BOLD}═══ $* ═══${NC}"; }
log_error()   { echo -e "${RED}ERROR${NC}: $*" >&2; }
log_debug()   { [[ "$SHROUD_TEST_VERBOSE" == "true" ]] && echo -e "  DEBUG: $*" || true; }

# ============================================================================
# Error Handling
# ============================================================================

_cleanup_on_error() {
    local exit_code=$?
    local line_no=$1
    log_error "Script failed at line $line_no with exit code $exit_code"
    # Kill any shroud processes we may have started
    pkill -f "shroud" 2>/dev/null || true
    exit $exit_code
}

trap '_cleanup_on_error $LINENO' ERR
trap 'log_info "Interrupted"; exit 130' INT TERM

# ============================================================================
# Utilities
# ============================================================================

# Ensure shroud binary is built
ensure_binary() {
    if [[ ! -x "$SHROUD_BIN" ]]; then
        log_info "Building Shroud (release)..."
        (cd "$PROJECT_ROOT" && cargo build --release --quiet)
    fi
}

# Create results directory if needed
ensure_results_dir() {
    mkdir -p "$RESULTS_DIR"
}

# Get shroud version for reporting
get_shroud_version() {
    if [[ -x "$SHROUD_BIN" ]]; then
        "$SHROUD_BIN" --version 2>/dev/null | head -1 || echo "unknown"
    else
        echo "not-built"
    fi
}

# Write JSON test result summary
write_json_result() {
    local suite_name="$1"
    local passed="${2:-0}"
    local failed="${3:-0}"
    local skipped="${4:-0}"
    local duration="${5:-0}"
    
    ensure_results_dir
    local output_file="${RESULTS_DIR}/${suite_name}.json"
    
    cat > "$output_file" << EOF
{
  "suite": "${suite_name}",
  "passed": ${passed},
  "failed": ${failed},
  "skipped": ${skipped},
  "total": $((passed + failed + skipped)),
  "duration_secs": ${duration},
  "timestamp": "$(date -Iseconds)",
  "shroud_version": "$(get_shroud_version)",
  "hostname": "$(hostname -s 2>/dev/null || echo 'unknown')",
  "success": $([ "$failed" -eq 0 ] && echo "true" || echo "false")
}
EOF
    log_info "Results written to: ${output_file}"
}

# Write detailed test results as JSON array
write_json_details() {
    local suite_name="$1"
    shift
    local results=("$@")
    
    ensure_results_dir
    local output_file="${RESULTS_DIR}/${suite_name}-details.json"
    
    # Join array elements with commas
    local json_array
    json_array=$(printf '%s\n' "${results[@]}" | paste -sd ',' -)
    
    echo "[${json_array}]" > "$output_file"
    log_debug "Detailed results written to: ${output_file}"
}

# Run cargo test with standard options
run_cargo_test() {
    local test_type="${1:-}"
    shift || true
    local extra_args=("$@")
    
    local thread_arg="--test-threads=${SHROUD_TEST_THREADS}"
    local verbose_arg=""
    [[ "$SHROUD_TEST_VERBOSE" == "true" ]] && verbose_arg="--verbose"
    
    if [[ -n "$test_type" ]]; then
        cargo test $verbose_arg "$test_type" --all-features -- "$thread_arg" "${extra_args[@]}"
    else
        cargo test $verbose_arg --all-features -- "$thread_arg" "${extra_args[@]}"
    fi
}

# Check if running as root
check_root() {
    if [[ $EUID -ne 0 ]]; then
        log_error "This script requires root privileges"
        exit 1
    fi
}

# Check root or skip (returns 1 if not root)
check_root_or_skip() {
    if [[ $EUID -ne 0 ]]; then
        log_skip "Requires root - skipping"
        return 1
    fi
    return 0
}

# Check if a command exists
command_exists() {
    command -v "$1" &>/dev/null
}

# Ensure a command exists or exit
require_command() {
    local cmd="$1"
    local install_hint="${2:-}"
    
    if ! command_exists "$cmd"; then
        log_error "Required command not found: $cmd"
        [[ -n "$install_hint" ]] && log_info "Install with: $install_hint"
        exit 1
    fi
}

# Parse cargo test output for pass/fail counts
# Returns: "passed failed ignored" space-separated
parse_cargo_test_output() {
    local output="$1"
    local passed=0
    local failed=0
    local ignored=0
    
    # Parse the summary line: "test result: ok. X passed; Y failed; Z ignored"
    local summary_line
    summary_line=$(echo "$output" | grep -E "test result:" | head -1)
    
    if [[ -n "$summary_line" ]]; then
        # Extract each count - handle the case where grep might return empty
        local p f i
        p=$(echo "$summary_line" | grep -oE "[0-9]+ passed" | grep -oE "^[0-9]+")
        f=$(echo "$summary_line" | grep -oE "[0-9]+ failed" | grep -oE "^[0-9]+")
        i=$(echo "$summary_line" | grep -oE "[0-9]+ ignored" | grep -oE "^[0-9]+")
        
        passed="${p:-0}"
        failed="${f:-0}"
        ignored="${i:-0}"
    fi
    
    # Ensure we output exactly 3 numbers
    printf "%d %d %d" "$passed" "$failed" "$ignored"
}

# Timer utilities
_start_time=0

start_timer() {
    _start_time=$(date +%s)
}

get_elapsed() {
    local end_time
    end_time=$(date +%s)
    echo $((end_time - _start_time))
}

# ============================================================================
# Test Result Recording (for shell-based tests)
# ============================================================================

# Global arrays for test tracking
declare -a _TEST_RESULTS=()
_PASSED=0
_FAILED=0
_SKIPPED=0

# Reset counters (call at start of test suite)
reset_counters() {
    _TEST_RESULTS=()
    _PASSED=0
    _FAILED=0
    _SKIPPED=0
}

# Record a test result
record_result() {
    local name="$1"
    local result="$2"  # pass, fail, skip
    local message="${3:-}"
    
    # Escape message for JSON
    message="${message//\\/\\\\}"
    message="${message//\"/\\\"}"
    message="${message//$'\n'/\\n}"
    
    _TEST_RESULTS+=("{\"name\":\"${name}\",\"result\":\"${result}\",\"message\":\"${message}\"}")
    
    case "$result" in
        pass) ((_PASSED++)) || true; log_pass "$name" ;;
        fail) ((_FAILED++)) || true; log_fail "$name: $message" ;;
        skip) ((_SKIPPED++)) || true; log_skip "$name: $message" ;;
    esac
}

# Get current counts
get_passed() { echo "$_PASSED"; }
get_failed() { echo "$_FAILED"; }
get_skipped() { echo "$_SKIPPED"; }

# Write all recorded results
write_recorded_results() {
    local suite_name="$1"
    local duration="${2:-0}"
    
    write_json_result "$suite_name" "$_PASSED" "$_FAILED" "$_SKIPPED" "$duration"
    
    if [[ ${#_TEST_RESULTS[@]} -gt 0 ]]; then
        write_json_details "$suite_name" "${_TEST_RESULTS[@]}"
    fi
}

# ============================================================================
# Cleanup utilities
# ============================================================================

# Kill any running shroud processes
kill_shroud() {
    pkill -f "shroud" 2>/dev/null || true
    sleep 0.5
}

# Remove test artifacts
cleanup_test_artifacts() {
    rm -f /tmp/shroud-test-* 2>/dev/null || true
    rm -f /run/user/*/shroud*.sock 2>/dev/null || true
}

# Full cleanup
full_cleanup() {
    kill_shroud
    cleanup_test_artifacts
}

# ============================================================================
# Initialization
# ============================================================================

# Change to project root
cd "$PROJECT_ROOT"

log_debug "Common library loaded (PROJECT_ROOT=$PROJECT_ROOT)"
