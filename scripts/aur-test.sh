#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
#
# aur-test.sh — Dry-run test for AUR package build
#
# Builds the package from local source (no GitHub tag needed),
# lints with namcap, and optionally installs.
#
# Usage:
#   ./scripts/aur-test.sh              # Build + lint (no install)
#   ./scripts/aur-test.sh --install    # Build + lint + install
#   ./scripts/aur-test.sh --clean      # Remove test artifacts

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
AUR_DIR="$REPO_DIR/aur"
TEST_DIR="/tmp/shroud-aur-test"
PKGNAME="vpn-shroud"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()  { echo -e "${BLUE}::${NC} $*"; }
ok()    { echo -e "${GREEN}✓${NC} $*"; }
warn()  { echo -e "${YELLOW}⚠${NC} $*"; }
fail()  { echo -e "${RED}✗${NC} $*"; }

# ============================================================================
# Prerequisites
# ============================================================================

check_prereqs() {
    info "Checking prerequisites..."
    local missing=()

    command -v makepkg  >/dev/null 2>&1 || missing+=("makepkg (base-devel)")
    command -v cargo    >/dev/null 2>&1 || missing+=("cargo (rust)")
    command -v rustc    >/dev/null 2>&1 || missing+=("rustc (rust)")
    command -v git      >/dev/null 2>&1 || missing+=("git")

    if [[ ${#missing[@]} -gt 0 ]]; then
        fail "Missing required tools:"
        for tool in "${missing[@]}"; do
            echo "  - $tool"
        done
        echo ""
        echo "Install with: sudo pacman -S --needed base-devel rust"
        exit 1
    fi
    ok "Core tools present (makepkg, cargo, rustc, git)"

    # namcap is optional but recommended
    if ! command -v namcap >/dev/null 2>&1; then
        warn "namcap not installed — linting will be skipped"
        warn "Install with: sudo pacman -S namcap"
        HAS_NAMCAP=false
    else
        ok "namcap available for linting"
        HAS_NAMCAP=true
    fi

    # Check PKGBUILD exists
    if [[ ! -f "$AUR_DIR/PKGBUILD" ]]; then
        fail "PKGBUILD not found at $AUR_DIR/PKGBUILD"
        exit 1
    fi
    ok "PKGBUILD found"

    if [[ ! -f "$AUR_DIR/shroud.install" ]]; then
        fail "shroud.install not found at $AUR_DIR/shroud.install"
        exit 1
    fi
    ok "shroud.install found"
}

# ============================================================================
# Extract version from PKGBUILD
# ============================================================================

get_pkgver() {
    grep '^pkgver=' "$AUR_DIR/PKGBUILD" | cut -d= -f2
}

# ============================================================================
# Create source tarball from local git tree
# ============================================================================

create_local_tarball() {
    local pkgver="$1"
    local tarball="$TEST_DIR/$PKGNAME-$pkgver.tar.gz"

    info "Creating source tarball from local git tree..."

    # Use git archive to create a clean tarball (respects .gitignore)
    cd "$REPO_DIR"
    git archive --format=tar.gz --prefix="shroud-$pkgver/" HEAD > "$tarball"

    ok "Source tarball: $tarball"
    echo "    Size: $(du -h "$tarball" | cut -f1)"
}

# ============================================================================
# Prepare test PKGBUILD pointing to local tarball
# ============================================================================

prepare_pkgbuild() {
    local pkgver="$1"

    info "Preparing PKGBUILD for local build..."

    # Copy PKGBUILD and install file to test dir
    cp "$AUR_DIR/PKGBUILD" "$TEST_DIR/PKGBUILD"
    cp "$AUR_DIR/shroud.install" "$TEST_DIR/shroud.install"

    # Replace the GitHub source URL with a local file reference
    local tarball_name="$PKGNAME-$pkgver.tar.gz"
    sed -i "s|source=(.*)|source=(\"$tarball_name\")|" "$TEST_DIR/PKGBUILD"

    # Generate correct checksums for the local tarball
    cd "$TEST_DIR"
    local checksum
    checksum=$(sha256sum "$tarball_name" | cut -d' ' -f1)
    sed -i "s|sha256sums=(.*)|sha256sums=('$checksum')|" "$TEST_DIR/PKGBUILD"

    ok "PKGBUILD patched for local source"
    echo "    SHA256: $checksum"
}

# ============================================================================
# Build
# ============================================================================

run_build() {
    info "Building package with makepkg..."
    echo ""

    cd "$TEST_DIR"

    # -s: install missing dependencies (needs sudo)
    # -f: force rebuild if package already exists
    # --noconfirm: don't prompt
    if makepkg -sf --noconfirm 2>&1; then
        echo ""
        ok "Package built successfully!"

        local pkg
        pkg=$(ls -1 "$TEST_DIR"/*.pkg.tar.zst 2>/dev/null | head -1)
        if [[ -n "$pkg" ]]; then
            echo "    Package: $pkg"
            echo "    Size:    $(du -h "$pkg" | cut -f1)"
        fi
        return 0
    else
        echo ""
        fail "Package build FAILED"
        return 1
    fi
}

# ============================================================================
# Lint with namcap
# ============================================================================

run_namcap() {
    if [[ "$HAS_NAMCAP" != "true" ]]; then
        warn "Skipping namcap lint (not installed)"
        return 0
    fi

    info "Running namcap lint..."
    echo ""

    local errors=0

    # Lint PKGBUILD
    echo "  --- PKGBUILD ---"
    cd "$TEST_DIR"
    if namcap PKGBUILD 2>&1; then
        ok "PKGBUILD lint passed"
    else
        warn "PKGBUILD lint produced warnings (review above)"
        errors=$((errors + 1))
    fi

    echo ""

    # Lint built package
    local pkg
    pkg=$(ls -1 "$TEST_DIR"/*.pkg.tar.zst 2>/dev/null | head -1)
    if [[ -n "$pkg" ]]; then
        echo "  --- Package ---"
        if namcap "$pkg" 2>&1; then
            ok "Package lint passed"
        else
            warn "Package lint produced warnings (review above)"
            errors=$((errors + 1))
        fi
    fi

    return 0
}

# ============================================================================
# Inspect package contents
# ============================================================================

inspect_package() {
    local pkg
    pkg=$(ls -1 "$TEST_DIR"/*.pkg.tar.zst 2>/dev/null | head -1)
    if [[ -z "$pkg" ]]; then
        return 0
    fi

    echo ""
    info "Package contents:"
    echo ""
    tar -tf "$pkg" | grep -v '/$' | sort | while read -r f; do
        echo "  $f"
    done

    echo ""
    info "Package info:"
    pacman -Qip "$pkg" 2>/dev/null || true
}

# ============================================================================
# Generate .SRCINFO
# ============================================================================

generate_srcinfo() {
    info "Generating .SRCINFO..."

    cd "$AUR_DIR"
    # Need to temporarily set up for makepkg --printsrcinfo
    # This uses the original PKGBUILD (with GitHub source)
    if makepkg --printsrcinfo > "$AUR_DIR/.SRCINFO" 2>/dev/null; then
        ok ".SRCINFO generated at aur/.SRCINFO"
    else
        warn "Could not auto-generate .SRCINFO (source not available)"
        warn "Generate manually after tagging: cd aur && makepkg --printsrcinfo > .SRCINFO"
    fi
}

# ============================================================================
# Test install (optional)
# ============================================================================

test_install() {
    local pkg
    pkg=$(ls -1 "$TEST_DIR"/*.pkg.tar.zst 2>/dev/null | head -1)
    if [[ -z "$pkg" ]]; then
        fail "No package found to install"
        return 1
    fi

    info "Installing package..."
    sudo pacman -U --noconfirm "$pkg"
    ok "Package installed"

    echo ""
    info "Verifying installation..."

    # Check binary
    if command -v shroud >/dev/null 2>&1; then
        ok "Binary found: $(which shroud)"
        local ver
        ver=$(shroud version 2>/dev/null | head -1 || echo "(could not get version)")
        echo "    Version: $ver"
    else
        fail "Binary 'shroud' not found in PATH"
    fi

    # Check service file
    if [[ -f /usr/lib/systemd/system/shroud.service ]]; then
        ok "Systemd service installed"
        # Verify the path was fixed
        if grep -q '/usr/bin/shroud' /usr/lib/systemd/system/shroud.service; then
            ok "Service uses /usr/bin/shroud (correct)"
        else
            fail "Service still references /usr/local/bin/shroud"
        fi
    else
        fail "Systemd service not found"
    fi

    # Check sudoers
    if [[ -f /etc/sudoers.d/shroud ]]; then
        ok "Sudoers rule installed"
    else
        fail "Sudoers rule not found"
    fi

    # Check desktop entry
    if [[ -f /usr/share/applications/shroud.desktop ]]; then
        ok "Desktop entry installed"
    else
        fail "Desktop entry not found"
    fi

    echo ""
    info "To uninstall: sudo pacman -R vpn-shroud"
}

# ============================================================================
# Clean
# ============================================================================

clean() {
    info "Cleaning test artifacts..."
    rm -rf "$TEST_DIR"
    ok "Cleaned $TEST_DIR"
}

# ============================================================================
# AUR SSH connectivity test
# ============================================================================

test_aur_ssh() {
    info "Testing AUR SSH connectivity..."
    if ssh -T aur@aur.archlinux.org 2>&1 | grep -qi "welcome"; then
        ok "AUR SSH authentication works"
    else
        warn "AUR SSH connection failed — check ~/.ssh/config and your AUR account"
        echo "    Ensure your public key is added at https://aur.archlinux.org/account"
        echo "    And ~/.ssh/config has:"
        echo ""
        echo "    Host aur.archlinux.org"
        echo "        IdentityFile ~/.ssh/id_rsa"
        echo "        User aur"
    fi
}

# ============================================================================
# Main
# ============================================================================

main() {
    local do_install=false
    local do_clean=false

    for arg in "$@"; do
        case "$arg" in
            --install) do_install=true ;;
            --clean)   do_clean=true ;;
            --help|-h)
                echo "Usage: $0 [--install] [--clean]"
                echo ""
                echo "  --install   Build and install the package"
                echo "  --clean     Remove test artifacts"
                echo ""
                echo "Without flags: build + lint (dry run, no install)"
                exit 0
                ;;
            *)
                echo "Unknown option: $arg"
                exit 1
                ;;
        esac
    done

    echo ""
    echo "╔══════════════════════════════════════════════╗"
    echo "║      VPNShroud AUR Package — Dry Run Test      ║"
    echo "╚══════════════════════════════════════════════╝"
    echo ""

    if $do_clean; then
        clean
        exit 0
    fi

    check_prereqs

    local pkgver
    pkgver=$(get_pkgver)
    info "Package version: $pkgver"
    echo ""

    # Test AUR SSH
    test_aur_ssh
    echo ""

    # Set up test directory
    rm -rf "$TEST_DIR"
    mkdir -p "$TEST_DIR"
    info "Test directory: $TEST_DIR"
    echo ""

    # Create tarball and build
    create_local_tarball "$pkgver"
    prepare_pkgbuild "$pkgver"
    echo ""

    if run_build; then
        echo ""
        run_namcap
        inspect_package

        if $do_install; then
            echo ""
            test_install
        fi

        echo ""
        echo "════════════════════════════════════════════════"
        ok "DRY RUN PASSED"
        echo ""
        echo "  Next steps to publish to AUR:"
        echo ""
        echo "  1. Tag the release (if not done):"
        echo "       git tag -a v$pkgver -m 'Release $pkgver'"
        echo "       git push origin v$pkgver"
        echo ""
        echo "  2. Update sha256sums in aur/PKGBUILD:"
        echo "       cd aur && updpkgsums"
        echo ""
        echo "  3. Generate .SRCINFO:"
        echo "       cd aur && makepkg --printsrcinfo > .SRCINFO"
        echo ""
        echo "  4. Clone AUR repo and push:"
        echo "       git clone ssh://aur@aur.archlinux.org/vpn-shroud.git /tmp/vpn-shroud-aur"
        echo "       cp aur/PKGBUILD aur/shroud.install /tmp/vpn-shroud-aur/"
        echo "       cd /tmp/vpn-shroud-aur"
        echo "       makepkg --printsrcinfo > .SRCINFO"
        echo "       git add PKGBUILD .SRCINFO shroud.install"
        echo "       git commit -m 'Initial upload: vpn-shroud $pkgver'"
        echo "       git push"
        echo ""
        echo "  5. Verify at: https://aur.archlinux.org/packages/vpn-shroud"
        echo "════════════════════════════════════════════════"
    else
        echo ""
        echo "════════════════════════════════════════════════"
        fail "DRY RUN FAILED — fix errors above before publishing"
        echo "════════════════════════════════════════════════"
        exit 1
    fi
}

main "$@"
