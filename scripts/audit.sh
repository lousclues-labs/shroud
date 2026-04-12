#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
# Security audit for VPNShroud dependencies
#
# This script checks for known vulnerabilities in dependencies
# using cargo-audit (RustSec Advisory Database).
#
# Usage: ./scripts/audit.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

echo "=================================="
echo " VPNShroud Dependency Security Audit"
echo "=================================="
echo ""

# Check if cargo-audit is installed
if ! command -v cargo-audit &> /dev/null; then
    echo "cargo-audit not found. Installing..."
    cargo install cargo-audit
    echo ""
fi

# Run the audit
echo "Checking dependencies against RustSec Advisory Database..."
echo ""

# Run audit with all features
if cargo audit; then
    echo ""
    echo "✓ No known vulnerabilities found"
    exit 0
else
    echo ""
    echo "⚠ Vulnerabilities detected! Review above and update dependencies."
    echo ""
    echo "To fix:"
    echo "  1. Update affected dependencies in Cargo.toml"
    echo "  2. Run 'cargo update' to get latest compatible versions"
    echo "  3. Run './scripts/audit.sh' again to verify"
    echo ""
    echo "If a vulnerability cannot be fixed immediately:"
    echo "  - Document it in SECURITY.md"
    echo "  - Create a tracking issue"
    echo "  - Assess actual risk to VPNShroud users"
    exit 1
fi
