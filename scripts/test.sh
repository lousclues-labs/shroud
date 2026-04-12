#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
# Unified test runner for VPNShroud
set -euo pipefail

case "${1:-all}" in
    unit)
        echo "Running unit tests..."
        cargo test --bins --all-features -- --test-threads=4
        ;;
    integration)
        echo "Running integration tests..."
        cargo test --test integration --all-features -- --test-threads=2
        ;;
    security)
        echo "Running security tests (non-privileged)..."
        cargo test --test security --all-features
        echo ""
        echo "For privileged tests: sudo -E cargo test --test security -- --ignored"
        ;;
    regression)
        echo "Running regression tests..."
        cargo test --test regression --all-features
        ;;
    coverage)
        echo "Generating coverage..."
        mkdir -p coverage
        cargo tarpaulin --all-features --out html --output-dir coverage
        echo ""
        echo "Report: coverage/tarpaulin-report.html"
        ;;
    all)
        echo "Running all tests..."
        cargo test --all-features -- --test-threads=4
        ;;
    ci)
        echo "Running CI checks..."
        cargo fmt --all --check
        cargo clippy --all-targets -- -D warnings
        cargo test --all-features -- --test-threads=4
        ;;
    *)
        echo "VPNShroud Test Runner"
        echo ""
        echo "Usage: $0 {unit|integration|security|regression|coverage|all|ci}"
        echo ""
        echo "Commands:"
        echo "  unit        - Run unit tests only"
        echo "  integration - Run integration tests only"
        echo "  security    - Run security tests (non-privileged)"
        echo "  regression  - Run regression tests only"
        echo "  coverage    - Generate coverage report"
        echo "  all         - Run all tests"
        echo "  ci          - Run full CI checks (fmt, clippy, tests)"
        exit 1
        ;;
esac

echo ""
echo "✓ Tests complete"
