#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
#
# Binary Size Tracking
#
# Records release binary size and optionally warns if it exceeds a ceiling.
#
# Usage: ./scripts/binary-size.sh [--ceiling BYTES] [--output FILE]
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
SHROUD_BIN="${PROJECT_ROOT}/target/release/shroud"

# Defaults
CEILING_BYTES=""
OUTPUT_FILE=""
FORMAT="text"

while [[ $# -gt 0 ]]; do
    case $1 in
        --ceiling) CEILING_BYTES="$2"; shift 2 ;;
        --output) OUTPUT_FILE="$2"; shift 2 ;;
        --json) FORMAT="json"; shift ;;
        --markdown) FORMAT="markdown"; shift ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Build if needed
if [[ ! -f "$SHROUD_BIN" ]]; then
    echo "Building release binary..."
    (cd "$PROJECT_ROOT" && cargo build --release)
fi

# Get size
SIZE_BYTES=$(stat -c%s "$SHROUD_BIN" 2>/dev/null || stat -f%z "$SHROUD_BIN")
SIZE_KB=$((SIZE_BYTES / 1024))
SIZE_MB=$(echo "scale=2; $SIZE_BYTES / 1048576" | bc)

# Get version
VERSION=$("$SHROUD_BIN" --version 2>/dev/null | awk '{print $2}' || echo "unknown")

# Get commit
COMMIT=$(git -C "$PROJECT_ROOT" rev-parse --short HEAD 2>/dev/null || echo "unknown")

# Check ceiling
OVER_CEILING=false
if [[ -n "$CEILING_BYTES" ]] && [[ $SIZE_BYTES -gt $CEILING_BYTES ]]; then
    OVER_CEILING=true
fi

# Output
case "$FORMAT" in
    json)
        cat << EOF
{
  "binary": "shroud",
  "version": "$VERSION",
  "commit": "$COMMIT",
  "size_bytes": $SIZE_BYTES,
  "size_kb": $SIZE_KB,
  "size_mb": "$SIZE_MB",
  "ceiling_bytes": ${CEILING_BYTES:-null},
  "over_ceiling": $OVER_CEILING,
  "timestamp": "$(date -Iseconds)"
}
EOF
        ;;
    markdown)
        cat << EOF
## Binary Size Report

| Metric | Value |
|--------|-------|
| Binary | \`shroud\` |
| Version | $VERSION |
| Commit | \`$COMMIT\` |
| Size | ${SIZE_MB} MB (${SIZE_KB} KB) |
| Size (bytes) | $SIZE_BYTES |
EOF
        if [[ -n "$CEILING_BYTES" ]]; then
            CEILING_MB=$(echo "scale=2; $CEILING_BYTES / 1048576" | bc)
            echo "| Ceiling | ${CEILING_MB} MB |"
            if $OVER_CEILING; then
                echo ""
                echo "⚠️ **Warning**: Binary size exceeds ceiling!"
            else
                echo ""
                echo "✅ Binary size within ceiling"
            fi
        fi
        ;;
    *)
        echo "═══════════════════════════════════════════════════════════════"
        echo "  BINARY SIZE REPORT"
        echo "═══════════════════════════════════════════════════════════════"
        echo ""
        echo "Binary:   shroud"
        echo "Version:  $VERSION"
        echo "Commit:   $COMMIT"
        echo "Size:     ${SIZE_MB} MB (${SIZE_KB} KB / ${SIZE_BYTES} bytes)"
        
        if [[ -n "$CEILING_BYTES" ]]; then
            CEILING_MB=$(echo "scale=2; $CEILING_BYTES / 1048576" | bc)
            echo "Ceiling:  ${CEILING_MB} MB (${CEILING_BYTES} bytes)"
            if $OVER_CEILING; then
                echo ""
                echo "⚠️  WARNING: Binary size exceeds ceiling!"
            else
                echo ""
                echo "✅ Binary size within ceiling"
            fi
        fi
        echo ""
        ;;
esac

# Write to file if specified
if [[ -n "$OUTPUT_FILE" ]]; then
    mkdir -p "$(dirname "$OUTPUT_FILE")"
    case "$FORMAT" in
        json)
            cat << EOF > "$OUTPUT_FILE"
{
  "binary": "shroud",
  "version": "$VERSION",
  "commit": "$COMMIT",
  "size_bytes": $SIZE_BYTES,
  "size_kb": $SIZE_KB,
  "size_mb": "$SIZE_MB",
  "ceiling_bytes": ${CEILING_BYTES:-null},
  "over_ceiling": $OVER_CEILING,
  "timestamp": "$(date -Iseconds)"
}
EOF
            ;;
        markdown)
            {
                echo "## Binary Size Report"
                echo ""
                echo "| Metric | Value |"
                echo "|--------|-------|"
                echo "| Binary | \`shroud\` |"
                echo "| Version | $VERSION |"
                echo "| Commit | \`$COMMIT\` |"
                echo "| Size | ${SIZE_MB} MB |"
            } > "$OUTPUT_FILE"
            ;;
        *)
            echo "$SIZE_BYTES" > "$OUTPUT_FILE"
            ;;
    esac
    echo "Report written to: $OUTPUT_FILE"
fi

# Exit with error if over ceiling
if $OVER_CEILING; then
    exit 1
fi
exit 0
