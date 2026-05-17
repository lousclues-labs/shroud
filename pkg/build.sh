#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# pkg/build.sh -- source-project contract for lousclues-pkg.
#
# Reads DISTRO, VERSION, OUTDIR from the environment, emits exactly
# one .deb or .rpm into OUTDIR per invocation, and exits non-zero on
# any failure. The release-build.yml workflow in
# lousclues-labs/lousclues-pkg invokes this script inside a container
# matching each target distro.
#
# Supported DISTRO values:
#   noble    -- Ubuntu 24.04   -> .deb
#   jammy    -- Ubuntu 22.04   -> .deb
#   bookworm -- Debian 12      -> .deb
#   el9      -- Rocky 9        -> .rpm
#   fedora   -- Fedora latest  -> .rpm
#
# Artifact naming (single file emitted into $OUTDIR):
#   shroud_${VERSION}_amd64-${DISTRO}.deb
#   shroud-${VERSION}-1.${DISTRO}.x86_64.rpm
#
# Exit codes:
#   0  success (one artifact in $OUTDIR)
#   1  build / packaging failure
#   2  invalid input (unknown DISTRO, version mismatch, missing env)
#
# Optional environment knobs (all default OFF / unset):
#   SHROUD_SKIP_DEPS=1         skip apt/dnf dep install (caller already prepared the image)
#   SHROUD_SKIP_TOOLCHAIN=1    skip rustup + fpm install (caller already prepared them)
#   SHROUD_CARGO_TARGET_DIR=…  reuse a persistent cargo target dir (CI cache mount)
#   CARGO_BUILD_JOBS=N         passed through to cargo (default: auto)
#   SOURCE_DATE_EPOCH=N        passed through to cargo/fpm for reproducible builds
#                              (auto-derived from `git log -1` when unset and inside a repo)
#
# Reference:
#   lousclues-labs/lousclues-pkg/docs/operator-runbook-releases.md

set -euo pipefail

# ----------------------------------------------------------------------------
# 0. Input validation (cheapest checks first; fail before any package install)
# ----------------------------------------------------------------------------

: "${DISTRO:?DISTRO must be set (one of: noble, jammy, bookworm, el9, fedora)}"
: "${VERSION:?VERSION must be set (semver, no leading v)}"
: "${OUTDIR:?OUTDIR must be set (absolute path; one .deb or .rpm emitted here)}"

case "$DISTRO" in
    noble|jammy|bookworm|el9|fedora) ;;
    *)
        echo "ERROR: unknown DISTRO '${DISTRO}'" >&2
        echo "       supported values: noble, jammy, bookworm, el9, fedora" >&2
        exit 2
        ;;
esac

if [[ "$VERSION" == v* ]]; then
    echo "ERROR: VERSION must not have a leading 'v' (got '${VERSION}')" >&2
    exit 2
fi

if [[ "$OUTDIR" != /* ]]; then
    echo "ERROR: OUTDIR must be an absolute path (got '${OUTDIR}')" >&2
    exit 2
fi

mkdir -p "$OUTDIR"

# Resolve paths relative to this script so the working directory does not matter.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Verify the script is sitting in a real shroud checkout.
if [[ ! -f "$REPO_DIR/Cargo.toml" ]]; then
    echo "ERROR: expected $REPO_DIR/Cargo.toml; pkg/build.sh must be run from a shroud checkout" >&2
    exit 2
fi

CARGO_VERSION="$(awk -F\" '/^version *=/ { print $2; exit }' "$REPO_DIR/Cargo.toml")"
if [[ -z "$CARGO_VERSION" ]]; then
    echo "ERROR: could not parse version from $REPO_DIR/Cargo.toml" >&2
    exit 2
fi
if [[ "$CARGO_VERSION" != "$VERSION" ]]; then
    echo "ERROR: VERSION ($VERSION) does not match Cargo.toml version ($CARGO_VERSION)" >&2
    echo "       Update Cargo.toml and Cargo.lock before tagging." >&2
    exit 2
fi

# Predictable file modes inside fpm staging.
umask 022

# Reproducible-build epoch: caller wins; otherwise derive from git if available.
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]] && command -v git >/dev/null 2>&1 \
        && git -C "$REPO_DIR" rev-parse --git-dir >/dev/null 2>&1; then
    SOURCE_DATE_EPOCH="$(git -C "$REPO_DIR" log -1 --pretty=%ct 2>/dev/null || true)"
fi
export SOURCE_DATE_EPOCH

# Faster, more reliable cargo network behaviour for short-lived CI containers.
export CARGO_NET_RETRY="${CARGO_NET_RETRY:-10}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"

# Honour an externally-mounted cargo target cache when provided.
if [[ -n "${SHROUD_CARGO_TARGET_DIR:-}" ]]; then
    export CARGO_TARGET_DIR="$SHROUD_CARGO_TARGET_DIR"
fi

echo "==> shroud package build"
echo "    distro : $DISTRO"
echo "    version: $VERSION"
echo "    outdir : $OUTDIR"
echo "    repo   : $REPO_DIR"
[[ -n "${SOURCE_DATE_EPOCH:-}" ]] && echo "    epoch  : $SOURCE_DATE_EPOCH"
[[ -n "${CARGO_TARGET_DIR:-}"  ]] && echo "    cargo  : CARGO_TARGET_DIR=$CARGO_TARGET_DIR"

# ----------------------------------------------------------------------------
# 1. Staging directory (cleaned on exit; OUTDIR is preserved)
# ----------------------------------------------------------------------------

STAGE="$(mktemp -d -t shroud-pkg-XXXXXXXX)"
cleanup() { rm -rf "$STAGE"; }
# Also catch SIGINT/SIGTERM so CI cancellations don't leave staging behind.
trap cleanup EXIT INT TERM

# ----------------------------------------------------------------------------
# 2. Per-distro dependency install
# ----------------------------------------------------------------------------

install_deb_deps() {
    if [[ "${SHROUD_SKIP_DEPS:-0}" == "1" ]]; then
        echo "==> [deps] SHROUD_SKIP_DEPS=1, skipping apt-get"
        return 0
    fi
    echo "==> [deps] installing build dependencies (apt-get)"
    export DEBIAN_FRONTEND=noninteractive
    # -q quiet, no recommends, no translations (saves several seconds of fetch).
    apt-get update -qq -o Acquire::Languages=none
    apt-get install -y -qq --no-install-recommends \
        ca-certificates curl build-essential pkg-config libdbus-1-dev \
        ruby ruby-dev rubygems
}

install_rpm_deps() {
    if [[ "${SHROUD_SKIP_DEPS:-0}" == "1" ]]; then
        echo "==> [deps] SHROUD_SKIP_DEPS=1, skipping dnf"
        return 0
    fi
    echo "==> [deps] installing build dependencies (dnf)"
    # --allowerasing  : swap curl-minimal for curl on EL9 / Fedora bases.
    # nodocs + no weak deps shave a noticeable amount off cold installs.
    # rubygem-json    : fpm requires `json` at load time (its package/python.rb
    #                   handler is loaded unconditionally); RHEL/Fedora's
    #                   `rubygems` package does not pull it as a default.
    dnf install -y -q --allowerasing \
        --setopt=install_weak_deps=False \
        --setopt=tsflags=nodocs \
        ca-certificates curl gcc gcc-c++ pkgconf-pkg-config dbus-devel \
        ruby ruby-devel rubygems rubygem-json rpm-build
}

# ----------------------------------------------------------------------------
# 3. Toolchain (cargo + fpm) — idempotent, skips when already present
# ----------------------------------------------------------------------------

ensure_toolchain() {
    if [[ "${SHROUD_SKIP_TOOLCHAIN:-0}" == "1" ]]; then
        echo "==> [toolchain] SHROUD_SKIP_TOOLCHAIN=1, skipping"
        return 0
    fi

    if command -v cargo >/dev/null 2>&1; then
        echo "==> [toolchain] cargo already present ($(cargo --version))"
    else
        echo "==> [toolchain] installing rust via rustup (minimal profile)"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
            | sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path
        # shellcheck disable=SC1091
        . "$HOME/.cargo/env"
    fi

    if command -v fpm >/dev/null 2>&1; then
        echo "==> [toolchain] fpm already present ($(fpm --version 2>/dev/null | head -n1))"
    else
        echo "==> [toolchain] installing fpm via gem"
        # --no-document skips rdoc/ri generation (large speedup on cold installs).
        gem install --no-document fpm
    fi
}

# ----------------------------------------------------------------------------
# 4. Build
# ----------------------------------------------------------------------------

build_binary() {
    echo "==> [build] cargo build --release --locked"
    # --locked: fail if Cargo.lock would change. Required for reproducible
    # package builds; also avoids index refreshes.
    ( cd "$REPO_DIR" && cargo build --release --locked )
}

# ----------------------------------------------------------------------------
# 5. Stage assets (shared between deb and rpm; per-distro unit dir differs)
# ----------------------------------------------------------------------------
#
# Layout mirrors the AUR PKGBUILD so package contents are consistent across
# distros. The only knob is the systemd unit directory:
#   - deb (Debian / Ubuntu) : /lib/systemd/system
#   - rpm (Fedora / RHEL)   : /usr/lib/systemd/system

stage_assets() {
    local unit_dir="$1"   # /lib/systemd/system  OR  /usr/lib/systemd/system
    echo "==> [stage] populating $STAGE"

    local target_dir="${CARGO_TARGET_DIR:-$REPO_DIR/target}"
    local bin_src="$target_dir/release/shroud"
    if [[ ! -x "$bin_src" ]]; then
        echo "ERROR: built binary not found at $bin_src" >&2
        return 1
    fi

    install -Dm755 "$bin_src" "$STAGE/usr/bin/shroud"

    # Systemd unit — patch /usr/local/bin/shroud → /usr/bin/shroud to match
    # the system-package install layout.
    install -Dm644 "$REPO_DIR/assets/shroud.service" "$STAGE${unit_dir}/shroud.service"
    sed -i 's|/usr/local/bin/shroud|/usr/bin/shroud|g' "$STAGE${unit_dir}/shroud.service"

    # Sudoers rule for passwordless kill switch (mode 0440 enforced by sudo).
    install -Dm440 "$REPO_DIR/assets/sudoers.d/shroud" "$STAGE/etc/sudoers.d/shroud"

    # Polkit policy (legacy path; still installed for desktops that prefer it).
    install -Dm644 "$REPO_DIR/assets/com.shroud.killswitch.policy" \
        "$STAGE/usr/share/polkit-1/actions/com.shroud.killswitch.policy"

    # Desktop entry — usable as both app launcher and autostart source.
    install -Dm644 "$REPO_DIR/autostart/shroud.desktop" \
        "$STAGE/usr/share/applications/shroud.desktop"

    # Example headless config — landed under /usr/share/doc, never auto-loaded.
    install -Dm644 "$REPO_DIR/assets/shroud-headless.conf.example" \
        "$STAGE/usr/share/doc/shroud/shroud-headless.conf.example"

    install -Dm644 "$REPO_DIR/README.md"    "$STAGE/usr/share/doc/shroud/README.md"
    install -Dm644 "$REPO_DIR/LICENSE"      "$STAGE/usr/share/doc/shroud/LICENSE"
    install -Dm644 "$REPO_DIR/CHANGELOG.md" "$STAGE/usr/share/doc/shroud/CHANGELOG.md"

    if [[ -d "$REPO_DIR/docs" ]]; then
        local doc
        for doc in "$REPO_DIR"/docs/*.md; do
            [[ -e "$doc" ]] || continue
            install -Dm644 "$doc" "$STAGE/usr/share/doc/shroud/docs/$(basename "$doc")"
        done
    fi
}

# ----------------------------------------------------------------------------
# 6. fpm invocations (one per arm)
# ----------------------------------------------------------------------------

PKG_MAINTAINER='lousclues <pkg@lousclues.com>'
PKG_URL='https://github.com/lousclues-labs/shroud'
PKG_DESC='Provider-agnostic VPN connection manager for Linux with kill switch, auto-reconnect, and system tray integration'
PKG_LICENSE='GPL-3.0-or-later'

fpm_deb() {
    local out="$OUTDIR/shroud_${VERSION}_amd64-${DISTRO}.deb"
    echo "==> [package] fpm -t deb → $out"
    rm -f "$out"
    fpm -s dir -t deb \
        --force \
        -n shroud \
        -v "$VERSION" \
        --architecture amd64 \
        --license "$PKG_LICENSE" \
        --maintainer "$PKG_MAINTAINER" \
        --url "$PKG_URL" \
        --description "$PKG_DESC" \
        --depends 'network-manager' \
        --depends 'dbus' \
        --depends 'iptables' \
        --deb-recommends 'nftables' \
        --deb-recommends 'polkit' \
        --deb-suggests  'network-manager-openvpn' \
        --config-files /etc/sudoers.d/shroud \
        --deb-no-default-config-files \
        -p "$out" \
        -C "$STAGE" .
    echo "$out"
}

fpm_rpm() {
    local out="$OUTDIR/shroud-${VERSION}-1.${DISTRO}.x86_64.rpm"
    echo "==> [package] fpm -t rpm → $out"
    rm -f "$out"
    fpm -s dir -t rpm \
        --force \
        -n shroud \
        -v "$VERSION" \
        --architecture x86_64 \
        --rpm-dist "$DISTRO" \
        --license "$PKG_LICENSE" \
        --maintainer "$PKG_MAINTAINER" \
        --url "$PKG_URL" \
        --description "$PKG_DESC" \
        --depends 'NetworkManager' \
        --depends 'dbus' \
        --depends 'iptables' \
        --config-files /etc/sudoers.d/shroud \
        -p "$out" \
        -C "$STAGE" .
    echo "$out"
}

# ----------------------------------------------------------------------------
# 7. Validation + machine-readable summary for the wrapping workflow
# ----------------------------------------------------------------------------

validate_artifact() {
    local out="$1"
    local ext="$2"

    if [[ ! -f "$out" ]]; then
        echo "ERROR: expected artifact not produced: $out" >&2
        return 1
    fi

    # Contract: exactly one $ext in $OUTDIR for this invocation.
    local count
    count=$(find "$OUTDIR" -maxdepth 1 -type f -name "*.${ext}" | wc -l)
    if [[ "$count" -ne 1 ]]; then
        echo "ERROR: contract violation — expected exactly 1 .${ext} in $OUTDIR, found $count" >&2
        find "$OUTDIR" -maxdepth 1 -type f -name "*.${ext}" -printf '       %p\n' >&2
        return 1
    fi

    local sha size
    sha=$(sha256sum "$out" | awk '{print $1}')
    size=$(stat -c %s "$out")

    echo "==> [done] artifact ready"
    echo "    path  : $out"
    echo "    size  : $size bytes"
    echo "    sha256: $sha"

    # Machine-readable line for the wrapping workflow to grep on.
    echo "ARTIFACT=$out SHA256=$sha SIZE=$size"
}

# ----------------------------------------------------------------------------
# 8. Per-distro orchestration
# ----------------------------------------------------------------------------

case "$DISTRO" in
    noble|jammy|bookworm)
        install_deb_deps
        ensure_toolchain
        build_binary
        stage_assets /lib/systemd/system
        out="$(fpm_deb | tail -n1)"
        validate_artifact "$out" deb
        ;;
    el9|fedora)
        install_rpm_deps
        ensure_toolchain
        build_binary
        stage_assets /usr/lib/systemd/system
        out="$(fpm_rpm | tail -n1)"
        validate_artifact "$out" rpm
        ;;
esac
