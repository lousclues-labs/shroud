#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# pkg-framework -- installed-tree layout check.
#
# Runs in the source-repo workflow AFTER a build artifact has been
# installed into a fresh distro container. Verifies that the installed
# filesystem matches what the manifest declares.
#
# Inputs (env from workflow):
#   ACTUAL  -- the package version that should be installed
#   EXT     -- deb or rpm (informational; manifest checks are the same)
#   ROOT    -- optional filesystem root prefix (default "" = real /).
#              Set to a temp dir for testing without installing.
#
# Reads pkg/project.sh for PKG_NAME, PKG_BINARIES, PKG_LAYOUT_CHECKS,
# PKG_SYSTEMD_UNITS. Calls project_install_layout_check_extra() if the
# project defines one (smoke runtime, getcap, etc.).
#
# Exits 0 if all checks pass, 1 otherwise. All failures are aggregated
# and reported at the end so the operator sees every problem at once.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
PROJECT_SH="${PROJECT_SH:-$REPO_ROOT/pkg/project.sh}"
ROOT="${ROOT:-}"

if [[ ! -r "$PROJECT_SH" ]]; then
    printf 'layout-check: ERROR: %s not readable\n' "$PROJECT_SH" >&2
    exit 1
fi

# shellcheck disable=SC1090
source "$PROJECT_SH"

FAILS=()

fail() {
    FAILS+=("$1")
    printf '  FAIL: %s\n' "$1"
}

ok() {
    printf '  ok:   %s\n' "$1"
}

# -------------------------------------------------------------------------
# Section A -- Required base checks
# -------------------------------------------------------------------------
printf '\n=== layout-check: binaries ===\n'
if [[ -n "${PKG_BINARIES+x}" ]]; then
    for bin in "${PKG_BINARIES[@]}"; do
        if [[ -x "$ROOT/usr/bin/$bin" ]]; then
            ok "/usr/bin/$bin (exists, executable)"
        else
            fail "/usr/bin/$bin missing or not executable"
        fi
    done
fi

printf '\n=== layout-check: documentation ===\n'
DOC_DIR="$ROOT/usr/share/doc/$PKG_NAME"
if [[ -d "$DOC_DIR" ]]; then
    ok "$DOC_DIR exists"
else
    fail "$DOC_DIR missing"
fi
if [[ -r "$DOC_DIR/README.md" ]]; then
    ok "$DOC_DIR/README.md"
fi
if [[ -r "$DOC_DIR/changelog.gz" ]]; then
    ok "$DOC_DIR/changelog.gz"
else
    fail "$DOC_DIR/changelog.gz missing"
fi
case "${EXT:-}" in
    deb)
        if [[ -r "$DOC_DIR/copyright" ]]; then
            ok "$DOC_DIR/copyright"
        else
            fail "$DOC_DIR/copyright missing"
        fi
        ;;
    rpm)
        if [[ -r "$DOC_DIR/LICENSE" ]]; then
            ok "$DOC_DIR/LICENSE"
        fi
        ;;
esac

# -------------------------------------------------------------------------
# Section B -- Systemd units
# -------------------------------------------------------------------------
if [[ -n "${PKG_SYSTEMD_UNITS+x}" && "${#PKG_SYSTEMD_UNITS[@]}" -gt 0 ]]; then
    printf '\n=== layout-check: systemd units ===\n'
    for unit in "${PKG_SYSTEMD_UNITS[@]}"; do
        found=""
        for prefix in "$ROOT/lib/systemd/system" "$ROOT/usr/lib/systemd/system"; do
            if [[ -r "$prefix/$unit" ]]; then
                found="$prefix/$unit"
                break
            fi
        done
        if [[ -n "$found" ]]; then
            ok "$found"
            # Reject any /home or /tmp ExecStart paths (dev regression).
            if grep -qE '^ExecStart=.*(/home/|/tmp/|target/release/)' "$found"; then
                fail "$found contains dev path in ExecStart"
            fi
            # systemd-analyze verify is best-effort. The container may
            # not have systemd-analyze; skip silently if absent.
            if command -v systemd-analyze >/dev/null 2>&1; then
                if ! systemd-analyze verify "$found" >/dev/null 2>&1; then
                    # Soft-fail: log + warn, don't add to FAILS. Some
                    # distros' systemd-analyze rejects valid syntax in
                    # rootless containers.
                    printf '  warn: systemd-analyze verify %s reported issues\n' "$found"
                fi
            fi
        else
            fail "systemd unit $unit not present under /lib or /usr/lib"
        fi
    done
fi

# -------------------------------------------------------------------------
# Section C -- Project-declared layout checks
# -------------------------------------------------------------------------
if [[ -n "${PKG_LAYOUT_CHECKS+x}" && "${#PKG_LAYOUT_CHECKS[@]}" -gt 0 ]]; then
    printf '\n=== layout-check: PKG_LAYOUT_CHECKS ===\n'
    for entry in "${PKG_LAYOUT_CHECKS[@]}"; do
        path="${entry%%:*}"
        mode=""
        if [[ "$entry" == *:* ]]; then
            mode="${entry#*:}"
        fi
        full="$ROOT/$path"
        if [[ ! -e "$full" ]]; then
            fail "$full missing"
            continue
        fi
        if [[ -n "$mode" ]]; then
            actual=$(stat -c '%a' "$full")
            if [[ "$actual" == "$mode" ]]; then
                ok "$full (mode $actual)"
            else
                fail "$full mode $actual, expected $mode"
            fi
        else
            ok "$full (exists)"
        fi
    done
fi

# -------------------------------------------------------------------------
# Section D -- Version assertion
# -------------------------------------------------------------------------
if [[ -n "${ACTUAL:-}" && -z "$ROOT" ]]; then
    printf '\n=== layout-check: package version ===\n'
    # Try dpkg first, fall back to rpm.
    installed=""
    if command -v dpkg-query >/dev/null 2>&1; then
        installed=$(dpkg-query -W -f='${Version}' "$PKG_NAME" 2>/dev/null || true)
    fi
    if [[ -z "$installed" ]] && command -v rpm >/dev/null 2>&1; then
        installed=$(rpm -q --qf '%{VERSION}' "$PKG_NAME" 2>/dev/null || true)
    fi
    if [[ -z "$installed" ]]; then
        fail "could not query installed version of $PKG_NAME"
    elif [[ "$installed" == "$ACTUAL" ]]; then
        ok "installed version $installed matches expected $ACTUAL"
    else
        fail "installed version $installed != expected $ACTUAL"
    fi
fi

# -------------------------------------------------------------------------
# Section E -- Project-supplied extra checks
# -------------------------------------------------------------------------
if declare -F project_install_layout_check_extra >/dev/null; then
    printf '\n=== layout-check: project_install_layout_check_extra ===\n'
    if ! project_install_layout_check_extra; then
        fail "project_install_layout_check_extra reported failures"
    fi
fi

# -------------------------------------------------------------------------
# Section F -- Summary
# -------------------------------------------------------------------------
printf '\n=== layout-check: summary ===\n'
if [[ "${#FAILS[@]}" -eq 0 ]]; then
    printf 'all checks passed\n'
    exit 0
fi
printf '%d failure(s):\n' "${#FAILS[@]}"
for f in "${FAILS[@]}"; do
    printf '  - %s\n' "$f"
done
exit 1
