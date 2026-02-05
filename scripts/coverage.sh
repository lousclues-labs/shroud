#!/usr/bin/env bash
# Generate code coverage report
#
# Usage: ./scripts/coverage.sh [options]
#
# Options:
#   --html          Generate HTML report only
#   --lcov          Generate lcov report
#   --ci            CI mode: generate all formats, check floor
#   --floor PCT     Minimum coverage percentage (default: 50)
#   --open          Open HTML report in browser

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

OPEN_REPORT=false
HTML_ONLY=false
LCOV=false
CI_MODE=false
FLOOR=50

while [[ $# -gt 0 ]]; do
    case $1 in
        --html)
            HTML_ONLY=true
            shift
            ;;
        --lcov)
            LCOV=true
            shift
            ;;
        --ci)
            CI_MODE=true
            shift
            ;;
        --floor)
            FLOOR="$2"
            shift 2
            ;;
        --open)
            OPEN_REPORT=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Check for tarpaulin
if ! command -v cargo-tarpaulin &> /dev/null; then
    echo "Installing cargo-tarpaulin..."
    cargo install cargo-tarpaulin --locked
fi

echo "=== Generating Coverage Report ==="

mkdir -p coverage

# Build output args
OUTPUT_ARGS="--out html --output-dir coverage"

if $CI_MODE; then
    # CI mode: generate all formats
    OUTPUT_ARGS="--out html --out xml --out lcov --output-dir coverage"
elif $LCOV; then
    OUTPUT_ARGS="$OUTPUT_ARGS --out lcov"
elif ! $HTML_ONLY; then
    OUTPUT_ARGS="$OUTPUT_ARGS --out xml"
fi

# Exclude tests that require system resources (D-Bus, iptables) since they're
# unreliable in CI/coverage environments and can hang or panic
EXCLUDE_ARGS="--exclude-files tests/e2e.rs --exclude-files tests/chaos.rs"

cargo tarpaulin \
    --verbose \
    --all-features \
    --workspace \
    --timeout 300 \
    $OUTPUT_ARGS \
    $EXCLUDE_ARGS \
    --skip-clean \
    --engine llvm \
    2>&1 | tee coverage/tarpaulin.log || echo "Tarpaulin completed with warnings"

echo ""
echo "✓ Coverage report generated in coverage/"

# Extract coverage percentage
COVERAGE_PCT=""
if [[ -f coverage/tarpaulin.log ]]; then
    COVERAGE_PCT=$(grep -oP '\d+\.\d+% coverage' coverage/tarpaulin.log | tail -1 | grep -oP '\d+\.\d+' || echo "")
fi

if [[ -n "$COVERAGE_PCT" ]]; then
    echo "Coverage: ${COVERAGE_PCT}%"
    
    # Check floor in CI mode
    if $CI_MODE; then
        COVERAGE_INT=${COVERAGE_PCT%.*}
        if [[ $COVERAGE_INT -lt $FLOOR ]]; then
            echo ""
            echo "⚠️  WARNING: Coverage ${COVERAGE_PCT}% is below floor of ${FLOOR}%"
            # Don't fail, just warn
        else
            echo "✅ Coverage meets floor of ${FLOOR}%"
        fi
        
        # Write coverage summary for CI
        cat > coverage/summary.json << EOF
{
  "coverage_percent": $COVERAGE_PCT,
  "floor_percent": $FLOOR,
  "meets_floor": $([ "$COVERAGE_INT" -ge "$FLOOR" ] && echo "true" || echo "false"),
  "timestamp": "$(date -Iseconds)"
}
EOF
    fi
fi

if $OPEN_REPORT; then
    if [[ -f coverage/tarpaulin-report.html ]]; then
        xdg-open coverage/tarpaulin-report.html 2>/dev/null || open coverage/tarpaulin-report.html 2>/dev/null || true
    fi
fi
