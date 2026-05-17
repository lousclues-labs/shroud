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
# Status: SCAFFOLD ONLY. Each per-distro arm exits non-zero with a
# fail-loud message. Before this project can be released through
# lousclues-pkg, the operator must replace the TODO blocks below
# with the real cargo + fpm (or dpkg-buildpackage / rpmbuild)
# invocations.
#
# Reference:
#   lousclues-labs/lousclues-pkg/docs/operator-runbook-releases.md

set -euo pipefail

: "${DISTRO:?DISTRO must be set (one of: noble, jammy, bookworm, el9, fedora)}"
: "${VERSION:?VERSION must be set (semver, no leading v)}"
: "${OUTDIR:?OUTDIR must be set (absolute path; one .deb or .rpm emitted here)}"

mkdir -p "$OUTDIR"

case "$DISTRO" in
    noble|jammy|bookworm)
        # TODO: implement .deb build for shroud.
        # Suggested approach:
        #   1. Install build deps:
        #        export DEBIAN_FRONTEND=noninteractive
        #        apt-get update -qq
        #        apt-get install -y --no-install-recommends \
        #            curl build-essential pkg-config libssl-dev \
        #            ruby ruby-dev rubygems
        #   2. Install the rust toolchain and fpm:
        #        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        #            | sh -s -- -y --default-toolchain stable --profile minimal
        #        . "$HOME/.cargo/env"
        #        gem install --no-document fpm
        #   3. cargo build --release --locked
        #   4. Stage the binary plus systemd units, configs, and any
        #      other assets this project needs under a staging dir.
        #   5. Emit the artifact with fpm, naming it per the contract:
        #        fpm -s dir -t deb \
        #            -n shroud -v "$VERSION" --license 'GPL-3.0-or-later' \
        #            --maintainer 'lousclues <pkg@lousclues.com>' \
        #            --description 'see lousclues-labs/shroud' \
        #            -p "$OUTDIR/shroud_${VERSION}_amd64-${DISTRO}.deb" \
        #            -C "$STAGE" .
        # shroud specifics:
        #   - single binary: 'shroud' (src/main.rs). Install to
        #     /usr/bin/.
        #   - install *.policy from repo root to
        #     /usr/share/polkit-1/actions/.
        #   - install autostart/*.desktop to /etc/xdg/autostart/.
        #   - install any systemd unit to /lib/systemd/system/.
        #   - runtime deps via fpm --depends 'network-manager'
        #     --depends 'nftables | iptables' --depends 'polkit'.
        echo "ERROR: TODO: implement ${DISTRO} (.deb) build for shroud" >&2
        echo "       See pkg/build.sh source comments for the suggested approach." >&2
        exit 1
        ;;
    el9|fedora)
        # TODO: implement .rpm build for shroud.
        # Suggested approach mirrors the .deb arm but targets rpm:
        #   1. Install build deps:
        #        dnf install -y --setopt=install_weak_deps=False \
        #            curl gcc gcc-c++ pkgconf-pkg-config openssl-devel \
        #            ruby ruby-devel rubygems rpm-build
        #   2. Install the rust toolchain and fpm (same as the .deb arm).
        #   3. cargo build --release --locked
        #   4. Stage assets under a staging dir.
        #   5. Emit the artifact:
        #        fpm -s dir -t rpm \
        #            -n shroud -v "$VERSION" --license 'GPL-3.0-or-later' \
        #            --maintainer 'lousclues <pkg@lousclues.com>' \
        #            -p "$OUTDIR/shroud-${VERSION}-1.${DISTRO}.x86_64.rpm" \
        #            -C "$STAGE" .
        # shroud specifics: same staging as the .deb arm; systemd
        # units go to /usr/lib/systemd/system/. rpm dep names:
        # 'NetworkManager', 'nftables', 'polkit'.
        echo "ERROR: TODO: implement ${DISTRO} (.rpm) build for shroud" >&2
        echo "       See pkg/build.sh source comments for the suggested approach." >&2
        exit 1
        ;;
    *)
        echo "ERROR: unknown DISTRO '${DISTRO}'" >&2
        echo "       supported values: noble, jammy, bookworm, el9, fedora" >&2
        exit 2
        ;;
esac
