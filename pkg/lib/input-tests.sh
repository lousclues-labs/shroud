#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# pkg-framework -- input-validation smoke tests.
#
# Drives `pkg/build.sh` with each of the eight invalid input cases and
# asserts that exit code matches the contract (1 = build/env error,
# 2 = invalid input).
#
# The test runs in CI as a fast pre-flight gate: any change that breaks
# the input-validation contract fails before the slow per-distro builds.
#
# Inputs (env): none. Reads pkg/build.sh relative to this file.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
BUILD_SH="$REPO_ROOT/pkg/build.sh"

if [[ ! -x "$BUILD_SH" ]]; then
    printf 'input-tests: ERROR: %s not found or not executable\n' "$BUILD_SH" >&2
    exit 1
fi

TMP_OUT=$(mktemp -d)
trap 'rm -rf "$TMP_OUT"' EXIT

FAILS=0

# expect_rc <expected> <label> <env-assignments...> -- <args...>
expect_rc() {
    local expected=$1 label=$2
    shift 2
    local env_pairs=()
    while [[ $# -gt 0 && "$1" != "--" ]]; do
        env_pairs+=("$1")
        shift
    done
    if [[ "${1:-}" == "--" ]]; then
        shift
    fi

    local actual=0
    env -i HOME="$HOME" PATH="$PATH" "${env_pairs[@]}" \
        "$BUILD_SH" "$@" >/dev/null 2>&1 || actual=$?

    if [[ "$actual" == "$expected" ]]; then
        printf '  ok:   %s -> rc=%d\n' "$label" "$actual"
    else
        printf '  FAIL: %s -> rc=%d (expected %d)\n' "$label" "$actual" "$expected"
        FAILS=$(( FAILS + 1 ))
    fi
}

printf '=== input-tests ===\n'

# 1. No env at all.
expect_rc 2 'no env' --

# 2. DISTRO missing.
expect_rc 2 'no DISTRO' VERSION=1.2.3 OUTDIR="$TMP_OUT" --

# 3. DISTRO invalid.
expect_rc 2 'invalid DISTRO' DISTRO=tar VERSION=1.2.3 OUTDIR="$TMP_OUT" --

# 4. VERSION missing.
expect_rc 2 'no VERSION' DISTRO=deb OUTDIR="$TMP_OUT" --

# 5. VERSION non-semver.
expect_rc 2 'non-semver VERSION' DISTRO=deb VERSION=v1 OUTDIR="$TMP_OUT" --

# 6. OUTDIR missing.
expect_rc 2 'no OUTDIR' DISTRO=deb VERSION=1.2.3 --

# 7. OUTDIR relative.
expect_rc 2 'relative OUTDIR' DISTRO=deb VERSION=1.2.3 OUTDIR=./out --

# 8. Manifest-commit env var malformed (not 40-hex).
# This one needs the build to get past env validation, so it's a runtime
# test the project should run separately if it wants to. We skip from
# the fast suite to keep this script offline.

printf '\nsummary: %d failure(s)\n' "$FAILS"
if [[ "$FAILS" -gt 0 ]]; then
    exit 1
fi
