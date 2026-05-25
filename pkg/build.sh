#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# pkg/build.sh -- entry point for the pkg-framework build.
#
# This is a thin wrapper. All project-specific data and hooks live in
# pkg/project.sh; all packaging logic lives in pkg/lib/framework.sh
# (vendored from lousclues-pkg via `pkg-framework sync`).
#
# Inputs (env, REQUIRED):
#   DISTRO   -- deb | rpm
#   VERSION  -- semver (must match Cargo.toml [package].version)
#   OUTDIR   -- absolute path; artifact + manifest sidecar land here
#
# Inputs (env, OPTIONAL):
#   <PKG_PREFIX>_MANIFEST_COMMIT  -- 40-char hex commit; embedded in
#                                    the manifest sidecar
#   PKG_KEEP_STAGE                -- non-empty: keep stage dir on exit
#
# Outputs:
#   $OUTDIR/<name>_<version>_amd64.deb (or .rpm)
#   $OUTDIR/<artifact>.manifest.json
#
# Exit codes:
#   0  success
#   1  build failure or missing dependency
#   2  invalid input or manifest contract violation

set -euo pipefail

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

# Codename → package-format translation.
#
# The canonical pkg-framework contract expects DISTRO=deb|rpm.
# lousclues-pkg's release-build.yml workflow passes the per-target
# codename (noble|jammy|bookworm|el9|fedora) as $DISTRO so that
# different source projects can use the codename downstream. For
# shroud we map back to the canonical deb/rpm before sourcing the
# framework, and surface the codename via $CODENAME for any hook
# that wants it. The artifact filename emitted by the framework is
# still `<name>_<ver>_amd64.deb` / `<name>-<ver>-<rel>.<arch>.rpm`;
# pkg-signing prepare-artifacts adds the codename suffix later.
case "${DISTRO:-}" in
    noble|jammy|bookworm)            export CODENAME="$DISTRO"; DISTRO=deb ;;
    el9|el10|fc40|fc41|fc42|fedora)  export CODENAME="$DISTRO"; DISTRO=rpm ;;
    deb|rpm|"")                      : ;;  # canonical or unset; framework validates
    *)
        printf 'pkg/build.sh: WARN: unrecognized DISTRO=%s; passing through\n' \
            "$DISTRO" >&2
        ;;
esac
export DISTRO

# 1. Load the project manifest.
# shellcheck source=project.sh disable=SC1091
source "$HERE/project.sh"

# 2. Load the framework library (vendored).
# shellcheck source=lib/framework.sh disable=SC1091
source "$HERE/lib/framework.sh"

# 3. Hand off.
run_pkg_build "$@"
