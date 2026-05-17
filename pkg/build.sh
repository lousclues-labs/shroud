#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# pkg/build.sh -- source-project contract for lousclues-pkg.
#
# Reads DISTRO, VERSION, OUTDIR from the environment, emits exactly
# one .deb or .rpm into OUTDIR per invocation, and exits non-zero on
# any failure. The release-build workflow in lousclues-labs/lousclues-pkg
# invokes this script inside a container matching each target distro.
# The local merge-blocking gate is .github/workflows/pkg-build.yml.
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
#   0  success. Exactly one artifact in OUTDIR.
#   1  build or packaging failure.
#   2  invalid input. Unknown DISTRO, version mismatch, or missing env.
#
# Optional environment knobs:
#   SOURCE_DATE_EPOCH        reproducible-build mtime and fpm timestamps
#   SHROUD_SKIP_DEPS=1       skip apt-get/dnf install of system build deps
#   SHROUD_SKIP_TOOLCHAIN=1  skip rustup and gem install fpm
#   SHROUD_CARGO_TARGET_DIR  override CARGO_TARGET_DIR. Default: target
#   SHROUD_KEEP_STAGE=1      preserve the staged tree for failure debugging
#   SHROUD_MANIFEST_COMMIT   override manifest git_commit for attestation
#   CARGO_BUILD_JOBS=N       passed through to cargo. Default: auto
#
# Reference: lousclues-labs/lousclues-pkg/docs/operator-runbook-releases.md

set -euo pipefail

# 1. Required env. Contract pinned by .github/workflows/pkg-build.yml.
: "${DISTRO:?DISTRO must be set (one of: noble, jammy, bookworm, el9, fedora)}"
: "${VERSION:?VERSION must be set (semver, no leading v)}"
: "${OUTDIR:?OUTDIR must be set (absolute path; one .deb or .rpm emitted here)}"

# 2. Input validation. Exit code 2 means invalid value.
case "$DISTRO" in
    noble|jammy|bookworm|el9|fedora) ;;
    *)
        echo "ERROR: unknown DISTRO '${DISTRO}'" >&2
        echo "       supported values: noble, jammy, bookworm, el9, fedora" >&2
        exit 2
        ;;
esac

# Reject leading 'v' in VERSION. The contract is semver only.
case "$VERSION" in
    v*)
        echo "ERROR: VERSION must not have a leading 'v' (got '${VERSION}')" >&2
        echo "       Pass the semver string directly, e.g. VERSION=2.2.0 not v2.2.0." >&2
        exit 2
        ;;
esac

# Reject relative OUTDIR. The release pipeline always passes an absolute
# path. Refusing relative paths prevents accidental writes into the repo
# tree on local invocations.
case "$OUTDIR" in
    /*) ;;
    *)
        echo "ERROR: OUTDIR must be an absolute path (got '${OUTDIR}')" >&2
        exit 2
        ;;
esac

# VERSION must match Cargo.toml. Catches operator typos and prevents
# producing a package whose internal version disagrees with the source
# tree it was built from.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CARGO_TOML="$REPO_DIR/Cargo.toml"
if [ ! -f "$CARGO_TOML" ]; then
    echo "ERROR: Cargo.toml not found at $CARGO_TOML" >&2
    exit 2
fi
CARGO_VERSION=$(awk -F'"' '/^version[[:space:]]*=/{print $2; exit}' "$CARGO_TOML")
if [ -z "$CARGO_VERSION" ]; then
    echo "ERROR: could not parse version from $CARGO_TOML" >&2
    exit 2
fi
if [ "$VERSION" != "$CARGO_VERSION" ]; then
    echo "ERROR: VERSION '$VERSION' does not match Cargo.toml version '$CARGO_VERSION'" >&2
    echo "       The packaging pipeline must build the version the source tree advertises." >&2
    exit 2
fi

mkdir -p "$OUTDIR"

# 3. Global environment. Hermetic, reproducible, quiet.
umask 022
export LC_ALL=C
export TZ=UTC
export CARGO_NET_RETRY=10
export CARGO_INCREMENTAL=0
export CARGO_TERM_COLOR=always
export RUSTFLAGS="${RUSTFLAGS:-} -C debuginfo=0 -C strip=symbols"

if [ -n "${SHROUD_CARGO_TARGET_DIR:-}" ]; then
    export CARGO_TARGET_DIR="$SHROUD_CARGO_TARGET_DIR"
else
    export CARGO_TARGET_DIR="$REPO_DIR/target"
fi

# SOURCE_DATE_EPOCH: prefer caller's value. Else use the last git commit
# time. Else use a pinned constant that matches the workflow fallback.
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
    if SDE=$(cd "$REPO_DIR" && git log -1 --pretty=%ct 2>/dev/null) && [ -n "$SDE" ]; then
        export SOURCE_DATE_EPOCH="$SDE"
    else
        export SOURCE_DATE_EPOCH=1700000000
    fi
fi

# Staging area. Cleaned on exit. Preserved if SHROUD_KEEP_STAGE=1.
STAGE="$(mktemp -d -t shroud-stage.XXXXXX)"
trap '[ "${SHROUD_KEEP_STAGE:-0}" = "1" ] || rm -rf "$STAGE"' EXIT INT TERM

# 4. Helpers.
log()    { printf '[pkg/build.sh] %s\n' "$*" >&2; }
section(){ printf '\n[pkg/build.sh] -- %s --\n' "$*" >&2; }
run()    { log "+ $*"; "$@"; }

# install_to <mode> <src> <dst-under-STAGE>
install_to() {
    local mode="$1" src="$2" dst="$STAGE/$3"
    install -Dm"$mode" "$src" "$dst"
}

section "shroud package build"
log "distro: $DISTRO"
log "version: $VERSION"
log "outdir: $OUTDIR"
log "repo: $REPO_DIR"
log "epoch: $SOURCE_DATE_EPOCH"
log "cargo target: $CARGO_TARGET_DIR"

# 5. System build dependencies.
install_deb_deps() {
    [ "${SHROUD_SKIP_DEPS:-0}" = "1" ] && { log "SHROUD_SKIP_DEPS=1. Skipping apt-get install."; return 0; }
    section "install_deb_deps"
    export DEBIAN_FRONTEND=noninteractive
    run apt-get update -qq -o Acquire::Languages=none
    # strip-nondeterminism post-processes .deb to scrub embedded
    # timestamps that fpm does not honour SOURCE_DATE_EPOCH for
    # (ar entry mtimes, gzip headers, tar entry mtimes/ordering).
    run apt-get install -y -qq --no-install-recommends \
        ca-certificates curl build-essential pkg-config libdbus-1-dev \
        ruby ruby-dev rubygems strip-nondeterminism
}

install_rpm_deps() {
    [ "${SHROUD_SKIP_DEPS:-0}" = "1" ] && { log "SHROUD_SKIP_DEPS=1. Skipping dnf install."; return 0; }
    section "install_rpm_deps"
    # NOTE on strip-nondeterminism (deliberately NOT installed on RPM):
    #   The Debian `strip-nondeterminism` tool is Debian-native -- the
    #   underlying File::StripNondeterminism distribution has never been
    #   uploaded to CPAN (verified: metacpan download_url returns 404).
    #   The binary is also not packaged for Fedora or EPEL.
    #
    #   AND: even if installed, strip-nondeterminism has NO handler for
    #   the .rpm file format (its handlers/ tree covers ar/cpio/gzip/zip/
    #   jar/png/etc. only). Running it on an .rpm is a no-op.
    #
    #   RPM reproducibility therefore rides entirely on the rpmbuild
    #   macros passed via fpm in fpm_rpm():
    #     - use_source_date_epoch_as_buildtime 1  (rpm 4.14+)
    #     - clamp_mtime_to_source_date_epoch 1    (rpm 4.18+, fedora)
    #     - _buildhost reproducible.shroud.local  (all rpm)
    #   plus the SOURCE_DATE_EPOCH env var which rpm 4.14+ honours.
    #
    # --allowerasing handles curl-minimal to curl on EL9 and Fedora.
    # rubygem-json is required because fpm loads package/python.rb and
    # that file requires json at load time on RHEL-family rubygems.
    run dnf install -y -q --allowerasing \
        --setopt=install_weak_deps=False \
        --setopt=tsflags=nodocs \
        ca-certificates curl gcc gcc-c++ pkgconf-pkg-config dbus-devel \
        ruby ruby-devel rubygems rubygem-json rpm-build
}

# 6. Rust toolchain + fpm.
ensure_toolchain() {
    [ "${SHROUD_SKIP_TOOLCHAIN:-0}" = "1" ] && { log "SHROUD_SKIP_TOOLCHAIN=1. Assuming cargo and fpm on PATH."; return 0; }
    section "ensure_toolchain"
    if ! command -v cargo >/dev/null 2>&1; then
        run curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
            -o /tmp/rustup-init.sh
        run sh /tmp/rustup-init.sh -y --default-toolchain stable --profile minimal --no-modify-path
        # shellcheck source=/dev/null
        . "$HOME/.cargo/env"
        export PATH="$HOME/.cargo/bin:$PATH"
    fi
    if ! command -v fpm >/dev/null 2>&1; then
        run gem install --no-document fpm
        # Default rubygems bin dir varies. Surface it for fpm calls.
        local gem_bin
        gem_bin=$(gem environment gemdir)/bin
        export PATH="$gem_bin:$PATH"
    fi
    run cargo --version
    run fpm --version
}

# 7. Build the binary.
build_binary() {
    section "build_binary"
    # Two-step pattern to keep the compile step provably offline while
    # still tolerating a cold registry cache (the default state on a
    # fresh GitHub Actions runner where no prior cargo invocation has
    # populated ~/.cargo/registry/index).
    #
    #   1. cargo fetch --locked       network allowed, lockfile pinned
    #   2. cargo build --frozen --offline   network refused, lockfile pinned
    #
    # --locked alone (no --frozen) on step 1 means: do not regenerate
    # Cargo.lock. Resolution must match the committed lock exactly.
    # --frozen on step 2 implies --locked AND --offline, which gives
    # us the audit-friendly "this compile touched no network" property.
    run cargo fetch --locked --manifest-path "$REPO_DIR/Cargo.toml"
    run cargo build --release --frozen --offline \
        --bin shroud \
        --manifest-path "$REPO_DIR/Cargo.toml"
}

# 8. Stage the on-disk layout.
# Arg 1: unit_dir (deb: /lib/systemd/system, rpm: /usr/lib/systemd/system)
stage_assets() {
    section "stage_assets unit_dir=$1"
    local unit_dir="$1"
    local unit_rel="${unit_dir#/}"
    local bin_dir=usr/bin
    local doc_dir=usr/share/doc/shroud

    local bin_src="$CARGO_TARGET_DIR/release/shroud"
    if [ ! -x "$bin_src" ]; then
        echo "ERROR: built binary not found at $bin_src" >&2
        return 1
    fi
    install_to 755 "$bin_src" "$bin_dir/shroud"

    # Systemd unit. Rewrite /usr/local/bin/shroud to the package path.
    install_to 644 "$REPO_DIR/assets/shroud.service" "$unit_rel/shroud.service"
    sed -i 's|/usr/local/bin/shroud|/usr/bin/shroud|g' "$STAGE/$unit_rel/shroud.service"

    # Sudoers rule for passwordless kill switch operations.
    install_to 440 "$REPO_DIR/assets/sudoers.d/shroud" "etc/sudoers.d/shroud"

    # Polkit policy. Some desktops prefer this path for local actions.
    install_to 644 "$REPO_DIR/assets/com.shroud.killswitch.policy" \
        "usr/share/polkit-1/actions/com.shroud.killswitch.policy"

    # Desktop entry. Used as the launcher and as the autostart source.
    install_to 644 "$REPO_DIR/autostart/shroud.desktop" \
        "usr/share/applications/shroud.desktop"

    # Example headless config. It is documentation, not live config.
    install_to 644 "$REPO_DIR/assets/shroud-headless.conf.example" \
        "$doc_dir/shroud-headless.conf.example"

    install_to 644 "$REPO_DIR/README.md" "$doc_dir/README.md"
    # changelog.gz: dpkg policy requires changelog.Debian.gz for native
    # changelogs. For upstream-only software, /usr/share/doc/<pkg>/changelog.gz
    # satisfies both the lintian rule and the rpm convention.
    gzip -9nc "$REPO_DIR/CHANGELOG.md" > "$STAGE/$doc_dir/changelog.gz"
    chmod 0644 "$STAGE/$doc_dir/changelog.gz"
    if [ "${FPM_TARGET:-}" = "deb" ]; then
        emit_debian_copyright > "$STAGE/$doc_dir/copyright"
        chmod 0644 "$STAGE/$doc_dir/copyright"
    else
        install_to 644 "$REPO_DIR/LICENSE" "$doc_dir/LICENSE"
    fi

    if [ -d "$REPO_DIR/docs" ]; then
        local doc
        for doc in "$REPO_DIR"/docs/*.md; do
            [ -e "$doc" ] || continue
            install_to 644 "$doc" "$doc_dir/docs/$(basename "$doc")"
        done
    fi

    # Pin every staged file's mtime to SOURCE_DATE_EPOCH so the tar/cpio
    # archive fpm produces is byte-stable run-to-run. find -exec touch is
    # the GNU-coreutils-portable form.
    find "$STAGE" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
}

# 9. Debian copyright file. Machine-readable format 1.0.
emit_debian_copyright() {
    cat <<EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: shroud
Upstream-Contact: lousclues-labs/shroud <https://github.com/lousclues-labs/shroud>
Source: https://github.com/lousclues-labs/shroud

Files: *
Copyright: 2026 Louis Nelson Jr. and lousclues-labs contributors
License: GPL-3.0-or-later
 This program is free software: you can redistribute it and/or modify
 it under the terms of the GNU General Public License as published by
 the Free Software Foundation, either version 3 of the License, or
 (at your option) any later version.
 .
 This program is distributed in the hope that it will be useful, but
 WITHOUT ANY WARRANTY; without even the implied warranty of
 MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU
 General Public License for more details.
 .
 On Debian systems, the complete text of the GNU General Public
 License version 3 can be found in /usr/share/common-licenses/GPL-3.
EOF
}

# 10. postinst scriptlet.
# Shroud has two post-install responsibilities:
#   1. validate the sudoers rule before sudo refuses to load it.
#   2. daemon-reload so systemd picks up unit changes, then refresh the
#      desktop database when the tool exists.
#
# Shroud deliberately does not use setcap here. Its privilege model is
# sudoers plus polkit. That keeps the tray binary in normal dynamic-linker
# mode and avoids AT_SECURE environment sanitising. If Shroud ever moves
# to file caps, copy vigil's post-setcap smoke test before leaving caps on
# the installed binary.
emit_postinst() {
    cat <<'EOF'
#!/bin/sh
# shroud postinst -- validate the sudoers rule, then daemon-reload.
set -eu

# Validate the dropped sudoers rule before sudo has a chance to refuse
# to load it. visudo -cf returns non-zero on syntax error and prints to
# stdout. We surface that to the user without failing the install.
# sudo will refuse the file on its own. The operator can fix and re-run
# `dpkg --configure -a` or `rpm -V`.
if command -v visudo >/dev/null 2>&1; then
    if ! visudo -cf /etc/sudoers.d/shroud >/dev/null; then
        echo "shroud: WARN: /etc/sudoers.d/shroud failed visudo syntax check." >&2
        echo "shroud:       sudo will refuse to load it. Fix and re-run." >&2
    fi
fi

# Pick up unit changes if systemd is the init.
if [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload >/dev/null 2>&1 || true
fi

# Refresh the desktop database so the new .desktop entry shows up
# without requiring a session restart. Best-effort. Absence is fine
# on headless installs and during chroot/container builds.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications 2>/dev/null || true
fi

exit 0
EOF
}

# 11. fpm invocations.
PKG_MAINTAINER='lousclues <pkg@lousclues.com>'
PKG_VENDOR='lousclues-labs'
PKG_URL='https://github.com/lousclues-labs/shroud'
PKG_DESC='Provider-agnostic VPN connection manager for Linux with kill switch, auto-reconnect, and system tray integration'
PKG_SUMMARY='Provider-agnostic VPN connection manager for Linux'
PKG_LICENSE='GPL-3.0-or-later'

fpm_deb() {
    section "fpm_deb"
    local out="$OUTDIR/shroud_${VERSION}_amd64-${DISTRO}.deb"
    local postinst
    postinst="$(mktemp -t shroud-postinst.XXXXXX)"
    emit_postinst > "$postinst"
    chmod 0755 "$postinst"
    # Pin the tempfile mtime. fpm captures it into control.tar.gz and
    # mktemp leaves it at wall-clock time, which breaks reproducibility.
    touch -h -d "@$SOURCE_DATE_EPOCH" "$postinst"

    rm -f "$out"
    run fpm \
        --input-type dir \
        --output-type deb \
        --force \
        --name shroud \
        --version "$VERSION" \
        --iteration "1.${DISTRO}" \
        --architecture amd64 \
        --license "$PKG_LICENSE" \
        --maintainer "$PKG_MAINTAINER" \
        --vendor "$PKG_VENDOR" \
        --url "$PKG_URL" \
        --description "$PKG_DESC" \
        --depends 'network-manager' \
        --depends 'dbus' \
        --depends 'iptables' \
        --deb-recommends 'nftables' \
        --deb-recommends 'polkit' \
        --deb-suggests 'network-manager-openvpn' \
        --config-files /etc/sudoers.d/shroud \
        --deb-no-default-config-files \
        --after-install "$postinst" \
        --package "$out" \
        --chdir "$STAGE" \
        .

    rm -f "$postinst"
    echo "$out"
}

fpm_rpm() {
    section "fpm_rpm"
    local out="$OUTDIR/shroud-${VERSION}-1.${DISTRO}.x86_64.rpm"
    local postinst
    postinst="$(mktemp -t shroud-postinst.XXXXXX)"
    emit_postinst > "$postinst"
    chmod 0755 "$postinst"
    # Same pin as fpm_deb. rpmbuild captures the file's mtime into
    # the cpio archive and SOURCE_DATE_EPOCH alone does not cover it.
    touch -h -d "@$SOURCE_DATE_EPOCH" "$postinst"

    rm -f "$out"
    run fpm \
        --input-type dir \
        --output-type rpm \
        --force \
        --name shroud \
        --version "$VERSION" \
        --iteration "1.${DISTRO}" \
        --architecture x86_64 \
        --license "$PKG_LICENSE" \
        --maintainer "$PKG_MAINTAINER" \
        --vendor "$PKG_VENDOR" \
        --url "$PKG_URL" \
        --description "$PKG_DESC" \
        --depends 'NetworkManager' \
        --depends 'dbus' \
        --depends 'iptables' \
        --rpm-dist "$DISTRO" \
        --rpm-os linux \
        --rpm-summary "$PKG_SUMMARY" \
        --rpm-rpmbuild-define 'use_source_date_epoch_as_buildtime 1' \
        --rpm-rpmbuild-define 'clamp_mtime_to_source_date_epoch 1' \
        --rpm-rpmbuild-define '_buildhost reproducible.shroud.local' \
        --config-files /etc/sudoers.d/shroud \
        --after-install "$postinst" \
        --package "$out" \
        --chdir "$STAGE" \
        .

    rm -f "$postinst"
    echo "$out"
}

# 12. Self-validation + manifest.
# The workflow runs an exhaustive layout check on the installed package.
# This self-check runs against the staged tree before fpm packs it, so
# failures surface inside this script with stage paths preserved when
# SHROUD_KEEP_STAGE=1.
validate_stage() {
    section "validate_stage"
    local unit_dir="$1"
    local missing=0
    local failures=0
    check() {
        if [ ! -e "$STAGE/$1" ]; then
            echo "stage MISSING: /$1" >&2
            missing=$((missing + 1))
            failures=$((failures + 1))
        fi
    }

    check usr/bin/shroud
    check "${unit_dir#/}/shroud.service"
    check etc/sudoers.d/shroud
    check usr/share/polkit-1/actions/com.shroud.killswitch.policy
    check usr/share/applications/shroud.desktop
    check usr/share/doc/shroud/README.md
    check usr/share/doc/shroud/changelog.gz
    check usr/share/doc/shroud/shroud-headless.conf.example
    if [ "${FPM_TARGET:-}" = "deb" ]; then
        check usr/share/doc/shroud/copyright
    else
        check usr/share/doc/shroud/LICENSE
    fi

    local sudoers="$STAGE/etc/sudoers.d/shroud"
    if [ -e "$sudoers" ]; then
        local mode
        mode=$(stat -c %a "$sudoers")
        if [ "$mode" != "440" ]; then
            echo "stage MODE: /etc/sudoers.d/shroud is $mode, expected 440" >&2
            failures=$((failures + 1))
        fi
    fi

    if [ "$failures" -gt 0 ]; then
        echo "ERROR: $missing missing file(s) in staged tree" >&2
        echo "ERROR: $failures staged tree validation failure(s)" >&2
        exit 1
    fi
    log "stage OK"
}

# Reproducibility post-processor.
#
# For .deb: fpm doesn't honour SOURCE_DATE_EPOCH in its internal
# tar/gzip/ar invocations -- ar entry mtimes, gzip header timestamps,
# and tar entry ordering all vary run-to-run. strip-nondeterminism is
# the standard tool for this (Debian reproducible-builds project) and
# ships in apt repos. install_deb_deps installs it.
#
# For .rpm: strip-nondeterminism has no .rpm handler -- running it on
# an .rpm is a no-op. RPM reproducibility is handled inside rpmbuild
# via the macros passed in fpm_rpm(). This step then logs and skips on
# RPM hosts where strip-nondeterminism is absent, which is expected.
make_reproducible() {
    local artifact="$1"
    section "make_reproducible"
    if command -v strip-nondeterminism >/dev/null 2>&1; then
        run strip-nondeterminism --timestamp="$SOURCE_DATE_EPOCH" "$artifact"
    else
        log "strip-nondeterminism not on PATH (expected on .rpm). rpmbuild macros handle reproducibility"
    fi
}

validate_artifact() {
    local artifact="$1"
    local ext="$2"
    section "validate_artifact"

    if [ ! -f "$artifact" ]; then
        echo "ERROR: expected artifact not produced: $artifact" >&2
        return 1
    fi

    local count
    count=$(find "$OUTDIR" -maxdepth 1 -type f -name "*.${ext}" | wc -l)
    if [ "$count" -ne 1 ]; then
        echo "ERROR: contract violation. Expected exactly 1 .${ext} in $OUTDIR, found $count" >&2
        find "$OUTDIR" -maxdepth 1 -type f -name "*.${ext}" -printf '       %p\n' >&2
        return 1
    fi
    log "artifact OK: $artifact"
}

validate_git_commit_hex() {
    printf '%s' "$1" | grep -Eq '^[0-9a-f]{40}$'
}

# Sidecar JSON next to the artifact. The lousclues-pkg pipeline consumes
# it when it pins attestations to the source commit. Format stays tiny.
emit_manifest() {
    local artifact="$1"
    local sha256
    sha256=$(sha256sum "$artifact" | awk '{print $1}')
    local size
    size=$(stat -c '%s' "$artifact")
    local git_commit
    # Resolution precedence:
    #   1. SHROUD_MANIFEST_COMMIT  -- explicit override. lousclues-pkg
    #      sets this to the exact tag commit so downstream attestations
    #      pin to source even when this script runs from a non-repo copy.
    #   2. SOURCE_SHA              -- generic source commit from the
    #      lousclues-pkg release-build orchestrator.
    #   3. git rev-parse HEAD      -- the common case, deterministic
    #      across CI re-runs of the same SHA.
    #   4. "unknown"              -- last resort, allowed ONLY when
    #      not running in CI. In CI a missing git_commit silently breaks
    #      attestation reproducibility downstream, so fail loudly.
    if [ -n "${SHROUD_MANIFEST_COMMIT:-}" ]; then
        git_commit="$SHROUD_MANIFEST_COMMIT"
        if ! validate_git_commit_hex "$git_commit"; then
            echo "ERROR: SHROUD_MANIFEST_COMMIT must be 40 lowercase hex characters" >&2
            exit 2
        fi
    elif [ -n "${SOURCE_SHA:-}" ]; then
        git_commit="$SOURCE_SHA"
        if ! validate_git_commit_hex "$git_commit"; then
            echo "ERROR: SOURCE_SHA must be 40 lowercase hex characters" >&2
            exit 2
        fi
    else
        git_commit=$(cd "$REPO_DIR" && git rev-parse HEAD 2>/dev/null || echo "")
        if [ -n "$git_commit" ] && ! validate_git_commit_hex "$git_commit"; then
            echo "ERROR: git rev-parse HEAD returned malformed commit: $git_commit" >&2
            exit 1
        elif [ -z "$git_commit" ]; then
            if [ -n "${CI:-}" ]; then
                echo "::error::emit_manifest: git_commit unresolved in CI. Set SHROUD_MANIFEST_COMMIT, SOURCE_SHA, or fix checkout depth." >&2
                exit 1
            fi
            git_commit="unknown"
        fi
    fi
    cat > "${artifact}.manifest.json" <<EOF
{
  "artifact": "$(basename "$artifact")",
  "sha256": "$sha256",
  "size_bytes": $size,
  "version": "$VERSION",
  "distro": "$DISTRO",
  "source_date_epoch": $SOURCE_DATE_EPOCH,
  "git_commit": "$git_commit"
}
EOF
    log "manifest written: ${artifact}.manifest.json"
    log "sha256: $sha256"
}

print_summary() {
    local artifact="$1"
    local sha size
    sha=$(sha256sum "$artifact" | awk '{print $1}')
    size=$(stat -c '%s' "$artifact")

    section "done"
    log "path: $artifact"
    log "size: $size bytes"
    log "sha256: $sha"

    # Machine-readable line for the wrapping workflow to grep on.
    echo "ARTIFACT=$artifact SHA256=$sha SIZE=$size MANIFEST=${artifact}.manifest.json"
}

# 13. Per-distro dispatch.
case "$DISTRO" in
    noble|jammy|bookworm)
        export FPM_TARGET=deb
        install_deb_deps
        ensure_toolchain
        build_binary
        stage_assets /lib/systemd/system
        validate_stage /lib/systemd/system
        artifact=$(fpm_deb | tail -n1)
        make_reproducible "$artifact"
        validate_artifact "$artifact" deb
        emit_manifest "$artifact"
        print_summary "$artifact"
        ;;
    el9|fedora)
        export FPM_TARGET=rpm
        install_rpm_deps
        ensure_toolchain
        build_binary
        stage_assets /usr/lib/systemd/system
        validate_stage /usr/lib/systemd/system
        artifact=$(fpm_rpm | tail -n1)
        make_reproducible "$artifact"
        validate_artifact "$artifact" rpm
        emit_manifest "$artifact"
        print_summary "$artifact"
        ;;
esac