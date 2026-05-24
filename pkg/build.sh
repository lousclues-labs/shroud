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

# 1. Load the project manifest.
# shellcheck source=project.sh disable=SC1091
source "$HERE/project.sh"

# 2. Load the framework library (vendored).
# shellcheck source=lib/framework.sh disable=SC1091
source "$HERE/lib/framework.sh"

# 3. Hand off.
run_pkg_build "$@"
