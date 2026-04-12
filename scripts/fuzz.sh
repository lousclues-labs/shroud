#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
#
# Run all fuzz targets for a configurable duration.
#
# Usage:
#   ./scripts/fuzz.sh           # 60 seconds per target (default)
#   ./scripts/fuzz.sh 300       # 5 minutes per target
#   ./scripts/fuzz.sh 0         # run indefinitely (Ctrl+C to stop)
#
# Requirements:
#   cargo install cargo-fuzz
#   rustup toolchain install nightly

set -euo pipefail

DURATION="${1:-60}"
TARGETS=(fuzz_ipc_command fuzz_config_parse fuzz_vpn_name)
FAILED=0

echo "═══════════════════════════════════════════════════════════════"
echo "  VPNShroud Fuzz Testing"
echo "═══════════════════════════════════════════════════════════════"
echo ""
echo "Duration: ${DURATION}s per target"
echo "Targets:  ${TARGETS[*]}"
echo ""

for target in "${TARGETS[@]}"; do
    echo "=== Fuzzing $target for ${DURATION}s ==="
    if cargo +nightly fuzz run "$target" -- -max_total_time="$DURATION"; then
        echo "✓ $target: no crashes found"
    else
        echo "✗ $target: CRASH FOUND — check fuzz/artifacts/$target/"
        FAILED=$((FAILED + 1))
    fi
    echo ""
done

echo "═══════════════════════════════════════════════════════════════"
if [ "$FAILED" -eq 0 ]; then
    echo "  ✓ All fuzz targets passed"
else
    echo "  ✗ $FAILED target(s) found crashes"
fi
echo "═══════════════════════════════════════════════════════════════"

exit "$FAILED"
