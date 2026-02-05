#!/bin/bash
# Run E2E tests inside Docker container with proper isolation and cleanup
#
# This script:
# 1. Starts required services (D-Bus)
# 2. Runs tests with timeout
# 3. Guarantees cleanup of any spawned processes
#
# Exit codes:
#   0 - All tests passed
#   1 - Test failures
#   124 - Timeout
#   * - Other errors

set -euo pipefail

# Configuration
TIMEOUT_SECONDS="${SHROUD_TEST_TIMEOUT:-120}"
TEST_THREADS="${SHROUD_TEST_THREADS:-1}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log() {
    echo -e "${GREEN}[E2E]${NC} $*"
}

warn() {
    echo -e "${YELLOW}[E2E]${NC} $*"
}

error() {
    echo -e "${RED}[E2E]${NC} $*" >&2
}

# Cleanup function - always runs
cleanup() {
    local exit_code=$?
    log "Cleaning up..."
    
    # Kill any shroud processes
    pkill -9 shroud 2>/dev/null || true
    pkill -9 e2e-test 2>/dev/null || true
    
    # Clean up sockets
    rm -f /tmp/shroud*.sock 2>/dev/null || true
    rm -f /run/shroud*.sock 2>/dev/null || true
    
    # Stop D-Bus if we started it
    if [[ -f /var/run/dbus/pid ]]; then
        kill "$(cat /var/run/dbus/pid)" 2>/dev/null || true
    fi
    
    log "Cleanup complete"
    exit $exit_code
}

trap cleanup EXIT INT TERM

# Start D-Bus daemon
start_dbus() {
    log "Starting D-Bus..."
    mkdir -p /var/run/dbus
    rm -f /var/run/dbus/pid
    dbus-daemon --system --fork --print-address
    
    # Wait for D-Bus to be ready
    for i in {1..10}; do
        if [[ -S /var/run/dbus/system_bus_socket ]]; then
            log "D-Bus is ready"
            return 0
        fi
        sleep 0.1
    done
    
    warn "D-Bus may not be fully ready"
}

# Run the tests
run_tests() {
    log "Running E2E tests (timeout: ${TIMEOUT_SECONDS}s, threads: ${TEST_THREADS})"
    
    # Build test arguments
    local args=("--test-threads=${TEST_THREADS}")
    
    # Add any passed arguments (e.g., test filter)
    if [[ $# -gt 0 ]]; then
        args+=("$@")
    fi
    
    # Run with timeout
    timeout "${TIMEOUT_SECONDS}" /app/e2e-test "${args[@]}"
}

# Main
main() {
    log "=== Shroud E2E Test Container ==="
    log "Binary: $(shroud --version 2>/dev/null || echo 'not found')"
    
    # Pre-cleanup
    pkill -9 shroud 2>/dev/null || true
    
    # Start services
    start_dbus
    
    # Run tests
    if run_tests "$@"; then
        log "=== All tests passed ==="
        exit 0
    else
        local exit_code=$?
        if [[ $exit_code -eq 124 ]]; then
            error "=== Tests timed out after ${TIMEOUT_SECONDS}s ==="
        else
            error "=== Some tests failed (exit code: $exit_code) ==="
        fi
        exit $exit_code
    fi
}

main "$@"
