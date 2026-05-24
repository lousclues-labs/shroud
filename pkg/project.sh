#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# pkg/project.sh -- shroud's pkg-framework manifest.
#
# Sourced by pkg/build.sh BEFORE pkg/lib/framework.sh. Data + hooks
# only; no top-level side effects.
#
# Drift-gated: every vendored file under pkg/lib/ and the workflow
# template at .github/workflows/pkg-build.yml are sha256-pinned by
# `pkg-framework verify`. Project-specific behavior lives here.

# Every PKG_* scalar/array below is read by pkg/lib/framework.sh after
# this file is sourced. shellcheck cannot trace the indirection so it
# flags each one as unused (SC2034); disable the warning for the whole
# file rather than annotating every line.
# shellcheck disable=SC2034

# =========================================================================
# Required scalars
# =========================================================================

PKG_NAME=shroud
PKG_PREFIX=SHROUD

PKG_SUMMARY="Provider-agnostic VPN connection manager for Linux"

# The framework passes this to fpm as a single --description arg (v1.2.2+
# nameref path), so multi-line paragraphs survive into both deb and rpm
# metadata intact.
PKG_DESCRIPTION="Provider-agnostic VPN connection manager for Linux with kill
switch, auto-reconnect, and system tray integration.

Shroud orchestrates NetworkManager VPN profiles through nmcli, enforces a
strict nftables / iptables kill switch when the tunnel is down, and surfaces
state through a KDE / Plasma StatusNotifierItem tray plus desktop
notifications. A headless mode runs the same supervisor under systemd
for servers and headless workstations."

PKG_VENDOR="lousclues-labs"
PKG_MAINTAINER="lousclues <pkg@lousclues.com>"

PKG_HOMEPAGE_URL="https://github.com/lousclues-labs/shroud"
PKG_SOURCE_URL="https://github.com/lousclues-labs/shroud"

PKG_LICENSE_SPDX="GPL-3.0-or-later"
PKG_LICENSE_NAME="GPL-3.0-or-later"

PKG_COPYRIGHT_HOLDERS="Louis Nelson Jr. and lousclues-labs contributors"
PKG_COPYRIGHT_YEAR="2026"

# =========================================================================
# Required arrays
# =========================================================================

# One binary. The cargo package is `vpn-shroud` (Cargo.toml [package]),
# but the produced [[bin]] target is `shroud` (Cargo.toml [[bin]]).
PKG_BINARIES=(shroud)

# Debian runtime deps. Names follow debian/ubuntu conventions.
PKG_DEB_DEPENDS=(
    network-manager
    dbus
    iptables
)

# =========================================================================
# Optional arrays
# =========================================================================

# RPM runtime deps. Package names diverge from debian: NetworkManager is
# capitalized on rpm distros.
PKG_RPM_REQUIRES=(
    NetworkManager
    dbus
    iptables
)

# Staged-tree assertions. Walked by both the framework's pre-fpm
# _pkg_validate_stage and the post-install layout-check.sh. Modes are
# unpadded (e.g. 755, not 0755) per framework convention.
PKG_LAYOUT_CHECKS=(
    "usr/bin/shroud:755"
    "lib/systemd/system/shroud.service:644"
    "etc/sudoers.d/shroud:440"
    "usr/share/polkit-1/actions/com.shroud.killswitch.policy:644"
    "usr/share/applications/shroud.desktop:644"
    "usr/share/doc/shroud/shroud-headless.conf.example:644"
)

# The systemd unit lives at assets/shroud.service (not systemd/), and
# the staged copy must rewrite the dev-only /usr/local/bin path. Both
# requirements push the install through project_stage_extra rather than
# PKG_SYSTEMD_UNITS. The postinst body below runs daemon-reload manually
# (the framework only appends it automatically when PKG_SYSTEMD_UNITS is
# populated).

# Extra files shipped under /usr/share/doc/shroud/. The framework copies
# each to /usr/share/doc/<name>/<basename>; the docs/ tree is staged by
# project_stage_extra below so the docs/ subdir is preserved.
PKG_EXTRA_DOC_FILES=(
    assets/shroud-headless.conf.example
)

# Mark the sudoers fragment as a debian conffile so apt prompts on
# upgrade rather than silently overwriting an operator's local edit.
PKG_DEB_CONFIG_FILES=(
    "/etc/sudoers.d/shroud"
)

# =========================================================================
# Optional scalars
# =========================================================================

FRAMEWORK_VERSION=1.2.4

# Hermetic build (v1.2.2+). Matches the pre-framework behavior, which
# ran `cargo fetch --locked` then `cargo build --release --frozen
# --offline`. With this on, the framework does the same dance and the
# compile step cannot touch the network.
PKG_CARGO_OFFLINE=1

# =========================================================================
# Hooks
# =========================================================================

# Stage shroud-specific assets: the systemd unit (with the dev-path
# rewrite), the sudoers fragment, the polkit policy, the .desktop file,
# and the docs/ tree.
project_stage_extra() {
    local root=$1

    # Systemd unit. The on-disk file at assets/shroud.service hardcodes
    # /usr/local/bin/shroud for dev installs via setup.sh; the packaged
    # copy needs /usr/bin/shroud. Stage to lib/systemd/system on every
    # distro -- the layout-check helper accepts /lib or /usr/lib.
    install -D -m 0644 "$REPO_ROOT/assets/shroud.service" \
        "$root/lib/systemd/system/shroud.service"
    sed -i 's|/usr/local/bin/shroud|/usr/bin/shroud|g' \
        "$root/lib/systemd/system/shroud.service"

    # Sudoers rule for passwordless kill-switch nft/iptables operations.
    # Mode 440 is mandatory; sudo refuses anything else under
    # /etc/sudoers.d/.
    install -D -m 0440 "$REPO_ROOT/assets/sudoers.d/shroud" \
        "$root/etc/sudoers.d/shroud"

    # Polkit policy for the killswitch action. Some desktops prefer
    # this path for local privileged actions.
    install -D -m 0644 "$REPO_ROOT/assets/com.shroud.killswitch.policy" \
        "$root/usr/share/polkit-1/actions/com.shroud.killswitch.policy"

    # Desktop entry. Doubles as the launcher and the autostart source.
    install -D -m 0644 "$REPO_ROOT/autostart/shroud.desktop" \
        "$root/usr/share/applications/shroud.desktop"

    # Documentation tree. Preserve the docs/ subdirectory layout the
    # pre-framework build shipped (operators link to it from
    # docs/RELEASING.md and friends).
    if [[ -d "$REPO_ROOT/docs" ]]; then
        local doc
        for doc in "$REPO_ROOT"/docs/*.md; do
            [[ -e "$doc" ]] || continue
            install -D -m 0644 "$doc" \
                "$root/usr/share/doc/shroud/docs/$(basename "$doc")"
        done
    fi
}

# Extra pre-fpm stage assertions. PKG_LAYOUT_CHECKS already covers
# path + mode; this hook adds checks that need shell logic.
project_validate_stage_extra() {
    local root=$1
    local rc=0

    # Regression guard: the staged systemd unit must NOT reference the
    # dev-only /usr/local/bin path. Catches a project_stage_extra
    # rewrite regression before fpm packs the archive.
    if grep -q '/usr/local/bin/shroud' \
            "$root/lib/systemd/system/shroud.service"; then
        printf 'project_validate_stage_extra: shroud.service still references /usr/local/bin/shroud\n' >&2
        rc=1
    fi

    return "$rc"
}

# Body of the deb/rpm postinst. The framework wraps this with
# `#!/bin/sh`, `set -e`, and `exit 0`. systemd daemon-reload is NOT
# auto-appended (PKG_SYSTEMD_UNITS is empty by design), so we run it
# here.
project_postinst_body() {
    cat <<'EOF'
# Validate the sudoers fragment we just dropped. sudo refuses to load a
# syntactically broken fragment; surface the error to the operator
# without failing the install -- the operator can fix and re-run
# `dpkg --configure -a` or `rpm -V` once corrected.
if command -v visudo >/dev/null 2>&1; then
    if ! visudo -cf /etc/sudoers.d/shroud >/dev/null; then
        echo 'shroud: WARN: /etc/sudoers.d/shroud failed visudo syntax check.' >&2
        echo 'shroud:       sudo will refuse to load it. Fix and re-run.' >&2
    fi
fi

# Pick up unit changes if systemd is the init.
if [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload >/dev/null 2>&1 || true
fi

# Refresh the desktop database so the .desktop entry shows up without
# requiring a session restart. Best-effort: missing on headless and
# inside chroot/container builds.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications 2>/dev/null || true
fi
EOF
}

# Extra deb fpm flags. The pre-framework build shipped these soft
# dependency hints; preserve them.
project_fpm_deb_extra_args() {
    cat <<'EOF'
--deb-recommends
nftables
--deb-recommends
polkit
--deb-suggests
network-manager-openvpn
EOF
}

# Post-install runtime smoke. Runs inside the install-test container
# after the package is installed.
project_install_layout_check_extra() {
    local rc=0

    # Binary runs and answers --help. Catches the worst class of
    # packaging regression: the binary is present but fails to load
    # (missing dynamic dep, wrong arch, stripped too aggressively).
    if [[ -x /usr/bin/shroud ]]; then
        if ! /usr/bin/shroud --help >/dev/null 2>&1; then
            printf 'project_install_layout_check_extra: /usr/bin/shroud --help failed\n' >&2
            rc=1
        fi
    fi

    # Regression guard: installed systemd unit must not reference the
    # dev-only /usr/local/bin path.
    for cand in /lib/systemd/system/shroud.service \
                /usr/lib/systemd/system/shroud.service; do
        [[ -f "$cand" ]] || continue
        if grep -q '/usr/local/bin/shroud' "$cand"; then
            printf 'project_install_layout_check_extra: %s references /usr/local/bin\n' "$cand" >&2
            rc=1
        fi
    done

    # Sudoers fragment must parse under visudo when visudo is present
    # (debian/ubuntu containers ship it; minimal fedora images may
    # not).
    if command -v visudo >/dev/null 2>&1; then
        if ! visudo -cf /etc/sudoers.d/shroud >/dev/null 2>&1; then
            printf 'project_install_layout_check_extra: visudo -cf /etc/sudoers.d/shroud failed\n' >&2
            rc=1
        fi
    fi

    return "$rc"
}
