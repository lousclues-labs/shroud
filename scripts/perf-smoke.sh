#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
#
# Performance Smoke Test
#
# Uses hyperfine to benchmark basic CLI operations.
# Outputs JSON and Markdown artifacts.
#
# Usage: ./scripts/perf-smoke.sh [--runs N] [--output-dir DIR]
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
SHROUD_BIN="${PROJECT_ROOT}/target/release/shroud"

# Defaults
RUNS=10
OUTPUT_DIR="${PROJECT_ROOT}/target/perf"

while [[ $# -gt 0 ]]; do
    case $1 in
        --runs) RUNS="$2"; shift 2 ;;
        --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Check for hyperfine
if ! command -v hyperfine &> /dev/null; then
    echo "hyperfine not found. Install with: cargo install hyperfine"
    echo ""
    echo "Falling back to simple timing..."
    
    # Simple fallback timing
    mkdir -p "$OUTPUT_DIR"
    
    echo "═══════════════════════════════════════════════════════════════"
    echo "  PERFORMANCE SMOKE (simple timing)"
    echo "═══════════════════════════════════════════════════════════════"
    echo ""
    
    # Time --version
    VERSION_TIME=$( { time -p "$SHROUD_BIN" --version > /dev/null 2>&1; } 2>&1 | grep real | awk '{print $2}' )
    echo "shroud --version: ${VERSION_TIME}s"
    
    # Time --help
    HELP_TIME=$( { time -p "$SHROUD_BIN" --help > /dev/null 2>&1; } 2>&1 | grep real | awk '{print $2}' )
    echo "shroud --help:    ${HELP_TIME}s"
    
    # Write simple JSON
    cat > "$OUTPUT_DIR/perf-smoke.json" << EOF
{
  "benchmarks": [
    {"command": "shroud --version", "mean_seconds": $VERSION_TIME},
    {"command": "shroud --help", "mean_seconds": $HELP_TIME}
  ],
  "note": "Simple timing (hyperfine not available)"
}
EOF
    
    # Write simple Markdown
    cat > "$OUTPUT_DIR/perf-smoke.md" << EOF
## Performance Smoke Test (Simple Timing)

| Command | Time |
|---------|------|
| \`shroud --version\` | ${VERSION_TIME}s |
| \`shroud --help\` | ${HELP_TIME}s |

*Note: hyperfine not available, using simple timing*
EOF
    
    exit 0
fi

# Build if needed
if [[ ! -f "$SHROUD_BIN" ]]; then
    echo "Building release binary..."
    (cd "$PROJECT_ROOT" && cargo build --release)
fi

mkdir -p "$OUTPUT_DIR"

echo "═══════════════════════════════════════════════════════════════"
echo "  PERFORMANCE SMOKE TEST"
echo "═══════════════════════════════════════════════════════════════"
echo ""
echo "Binary:  $SHROUD_BIN"
echo "Runs:    $RUNS"
echo "Output:  $OUTPUT_DIR"
echo ""

# Run benchmarks
hyperfine \
    --warmup 3 \
    --runs "$RUNS" \
    --export-json "$OUTPUT_DIR/perf-smoke.json" \
    --export-markdown "$OUTPUT_DIR/perf-smoke.md" \
    "$SHROUD_BIN --version" \
    "$SHROUD_BIN --help" \
    "$SHROUD_BIN list 2>/dev/null || true"

echo ""
echo "Results written to:"
echo "  JSON:     $OUTPUT_DIR/perf-smoke.json"
echo "  Markdown: $OUTPUT_DIR/perf-smoke.md"

# Print summary
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  SUMMARY"
echo "═══════════════════════════════════════════════════════════════"
cat "$OUTPUT_DIR/perf-smoke.md"
