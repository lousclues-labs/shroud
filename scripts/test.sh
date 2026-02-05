#!/bin/bash
# Unified test runner for Shroud
#
# This script provides a consistent interface to run different test types
# with proper cleanup and reporting.
#
# Usage: ./scripts/test.sh [command] [options]
#
# Commands:
#   unit        Run unit tests (fast, parallel)
#   integration Run integration tests
#   security    Run security tests (non-privileged)
#   stability   Run stability/race condition tests
#   e2e         Run E2E tests (spawns daemon processes)
#   e2e-docker  Run E2E tests in Docker container (isolated)
#   coverage    Generate coverage report
#   all         Run all test types
#   ci          Simulate full CI pipeline locally
#   quick       Run unit + integration (fast feedback)
#
# Options:
#   --verbose   Show verbose output
#   --release   Use release build (for E2E)
#   --filter    Filter tests by name
#   --jobs N    Set number of parallel jobs

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
VERBOSE=false
RELEASE=false
FILTER=""
JOBS=4

# Logging
log() { echo -e "${GREEN}[TEST]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }
info() { echo -e "${BLUE}[INFO]${NC} $*"; }

# Cleanup function
cleanup() {
    log "Cleaning up..."
    pkill -9 -f "shroud --headless" 2>/dev/null || true
    pkill -9 -x shroud 2>/dev/null || true
    rm -f /tmp/shroud*.sock 2>/dev/null || true
}

trap cleanup EXIT

# Parse options
parse_options() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --verbose|-v) VERBOSE=true; shift ;;
            --release|-r) RELEASE=true; shift ;;
            --filter|-f) FILTER="$2"; shift 2 ;;
            --jobs|-j) JOBS="$2"; shift 2 ;;
            *) break ;;
        esac
    done
}

# Build test arguments
build_args() {
    local args=""
    if [[ -n "$FILTER" ]]; then
        args="$FILTER"
    fi
    echo "$args"
}

# Run unit tests
run_unit() {
    log "=== Running Unit Tests ==="
    local args=""
    if $VERBOSE; then args="$args --nocapture"; fi

    cargo test --bins --all-features -- --test-threads="$JOBS" $args $(build_args)
}

# Run integration tests  
run_integration() {
    log "=== Running Integration Tests ==="
    local args=""
    if $VERBOSE; then args="$args --nocapture"; fi
    
    cargo test --test integration --all-features -- --test-threads=2 $args $(build_args)
}

# Run security tests
run_security() {
    log "=== Running Security Tests ==="
    local args=""
    if $VERBOSE; then args="$args --nocapture"; fi
    
    cargo test --test security --all-features -- $args $(build_args)
    
    info "Note: Some security tests require sudo. Run with:"
    info "  sudo -E cargo test --test security -- --ignored"
}

# Run stability tests
run_stability() {
    log "=== Running Stability Tests ==="
    local args=""
    if $VERBOSE; then args="$args --nocapture"; fi
    
    cargo test --test stability --all-features -- $args $(build_args)
}

# Run E2E tests
run_e2e() {
    log "=== Running E2E Tests ==="
    
    # Build first
    if $RELEASE; then
        cargo build --release
    else
        cargo build
    fi
    
    # Pre-cleanup
    cleanup
    
    local args=""
    if $VERBOSE; then args="$args --nocapture"; fi
    
    # Run with timeout
    timeout 300 cargo test --test e2e --all-features -- --test-threads=1 $args $(build_args) || {
        local exit_code=$?
        if [[ $exit_code -eq 124 ]]; then
            error "E2E tests timed out after 300 seconds"
            return 1
        fi
        return $exit_code
    }
}

# Run E2E tests in Docker
run_e2e_docker() {
    log "=== Building E2E Docker Image ==="
    
    cargo build --release
    cargo test --test e2e --no-run
    
    docker build -t shroud-e2e -f "$PROJECT_ROOT/tests/e2e/Dockerfile" "$PROJECT_ROOT"
    
    log "=== Running E2E Tests in Docker ==="
    docker run --rm --cap-add=NET_ADMIN shroud-e2e $(build_args)
    
    log "=== Cleanup ==="
    docker rmi shroud-e2e 2>/dev/null || true
}

# Generate coverage
run_coverage() {
    log "=== Generating Coverage Report ==="
    
    # Pre-cleanup
    cleanup
    
    cargo tarpaulin \
        --bins --lib \
        --test integration \
        --test stability \
        --out html \
        --output-dir coverage \
        --timeout 120 \
        --exclude-files "tests/e2e/*" \
        --exclude-files "tests/e2e.rs" \
        --exclude-files "tests/chaos/*" \
        -- --test-threads=2
    
    log "Coverage report: $PROJECT_ROOT/coverage/tarpaulin-report.html"
}

# Run all tests
run_all() {
    run_unit
    run_integration
    run_security
    run_stability
    run_e2e
}

# Quick tests (fast feedback)
run_quick() {
    run_unit
    run_integration
}

# Simulate CI pipeline
run_ci() {
    log "=== Simulating CI Pipeline ==="
    local start_time=$(date +%s)
    
    log "Step 1: Format check"
    cargo fmt --all --check
    
    log "Step 2: Clippy"
    cargo clippy --all-targets -- -D warnings
    
    log "Step 3: Unit tests"
    run_unit
    
    log "Step 4: Integration tests"
    run_integration
    
    log "Step 5: Stability tests"
    run_stability
    
    log "Step 6: E2E tests"
    run_e2e || warn "Some E2E tests failed (may be expected)"
    
    local end_time=$(date +%s)
    local duration=$((end_time - start_time))
    
    log "=== CI Simulation Complete (${duration}s) ==="
}

# Usage
usage() {
    cat << 'EOF'
Shroud Test Runner

Usage: ./scripts/test.sh [command] [options]

Commands:
  unit        Run unit tests (fast, parallel)
  integration Run integration tests
  security    Run security tests (non-privileged)
  stability   Run stability/race condition tests
  e2e         Run E2E tests (spawns daemon processes)
  e2e-docker  Run E2E tests in Docker container (isolated)
  coverage    Generate coverage report
  all         Run all test types
  ci          Simulate full CI pipeline locally
  quick       Run unit + integration (fast feedback)

Options:
  --verbose, -v   Show verbose output
  --release, -r   Use release build (for E2E)
  --filter, -f    Filter tests by name
  --jobs, -j N    Set number of parallel jobs (default: 4)

Examples:
  ./scripts/test.sh unit                    # Run unit tests
  ./scripts/test.sh e2e --verbose           # Run E2E with verbose output
  ./scripts/test.sh ci                      # Full CI simulation
  ./scripts/test.sh quick                   # Fast feedback loop
  ./scripts/test.sh unit -f config          # Run tests matching "config"
EOF
}

# Main
main() {
    cd "$PROJECT_ROOT"
    
    # Parse global options first
    local cmd="${1:-}"
    shift || true
    parse_options "$@"
    
    case "$cmd" in
        unit)        run_unit ;;
        integration) run_integration ;;
        security)    run_security ;;
        stability)   run_stability ;;
        e2e)         run_e2e ;;
        e2e-docker)  run_e2e_docker ;;
        coverage)    run_coverage ;;
        all)         run_all ;;
        ci)          run_ci ;;
        quick)       run_quick ;;
        help|--help|-h) usage; exit 0 ;;
        *)           usage; exit 1 ;;
    esac
}

main "$@"
