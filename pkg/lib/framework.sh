#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# pkg-framework -- shared deb/rpm builder for lousclues-pkg projects.
#
# This library is vendored into a source project's `pkg/lib/framework.sh`
# via `pkg-framework sync`. The source project's `pkg/build.sh` is a
# thin wrapper that:
#
#   1. sources `pkg/project.sh` (the manifest -- declares PKG_* vars and
#      optional `project_*` hook functions);
#   2. sources `pkg/lib/framework.sh` (this file);
#   3. calls `run_pkg_build "$@"`.
#
# The framework exposes ONE public entry point (`run_pkg_build`) plus the
# `pkg_*` helper namespace. Everything prefixed with a single underscore
# is internal.
#
# Contract with the project manifest (`pkg/project.sh`) is documented in
# `pkg-framework/lib/project.sh.example` and locked by sha256 via
# `pkg-framework verify` running in source-repo CI.
#
# Exit codes: 0 ok; 1 build failure or missing dependency; 2 invalid
# input or missing required manifest field.

# Refuse to source twice (catches duplicate `source` lines in wrappers).
if [[ -n "${PKG_FRAMEWORK_LOADED:-}" ]]; then
    return 0
fi
PKG_FRAMEWORK_LOADED=1

# -------------------------------------------------------------------------
# Defaults (override in pkg/project.sh)
# -------------------------------------------------------------------------
: "${FPM_VERSION:=1.16.0}"
: "${SOURCE_DATE_EPOCH_DEFAULT:=1700000000}"
: "${PKG_DEB_SECTION:=net}"
: "${PKG_DEB_PRIORITY:=optional}"
: "${PKG_RPM_GROUP:=Applications/System}"

# Will be populated by _pkg_compute_paths.
PKG_FRAMEWORK_VERSION_RUNTIME=""
REPO_ROOT=""
STAGE=""
DEB_OUT=""
RPM_OUT=""

# -------------------------------------------------------------------------
# Section 1 -- Path discovery + framework guard
# -------------------------------------------------------------------------
_pkg_compute_paths() {
    # ${BASH_SOURCE[0]} is the framework.sh path (vendored at
    # <repo>/pkg/lib/framework.sh). REPO_ROOT is two dirs up.
    local lib_dir
    lib_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

    REPO_ROOT="$(cd -- "$lib_dir/../.." && pwd)"

    if [[ -r "$lib_dir/VERSION" ]]; then
        PKG_FRAMEWORK_VERSION_RUNTIME="$(tr -d '\r\n' < "$lib_dir/VERSION")"
    else
        PKG_FRAMEWORK_VERSION_RUNTIME="unknown"
    fi
}

_pkg_check_framework_version() {
    # FRAMEWORK_VERSION is set by the project manifest. The sync tool
    # writes it; mismatches indicate the vendored copy drifted from the
    # pinned version.
    if [[ -z "${FRAMEWORK_VERSION:-}" ]]; then
        printf 'pkg-framework: WARN: FRAMEWORK_VERSION not pinned in pkg/project.sh\n' >&2
        return 0
    fi
    if [[ "$FRAMEWORK_VERSION" != "$PKG_FRAMEWORK_VERSION_RUNTIME" ]]; then
        printf 'pkg-framework: ERROR: manifest pins FRAMEWORK_VERSION=%s but vendored copy is %s\n' \
            "$FRAMEWORK_VERSION" "$PKG_FRAMEWORK_VERSION_RUNTIME" >&2
        printf 'pkg-framework: re-run `pkg-framework sync` in the source repo.\n' >&2
        return 2
    fi
}

# -------------------------------------------------------------------------
# Section 2 -- Helpers (logging, exec, retry, file install)
# -------------------------------------------------------------------------
# v1.2.2: log/section/run write to stderr globally. The framework
# uses these for human progress chatter; the previous shape mixed
# them with command stdout, and any helper that did `x=$(_pkg_foo)`
# captured the chatter into x. _pkg_fpm_deb / _pkg_fpm_rpm were the
# most painful instance (the artifact path got buried in the build
# log). Stdout is now reserved for machine-readable return values.
log() { printf '[%s] %s\n' "$(date -u +%H:%M:%SZ)" "$*" >&2; }

section() {
    printf '\n=== %s ===\n' "$*" >&2
}

run() {
    printf '$ %s\n' "$*" >&2
    "$@"
}

retry() {
    # retry <max-attempts> <sleep-seconds> <cmd...>
    local n=$1 s=$2 attempt=0
    shift 2
    while (( attempt < n )); do
        if "$@"; then
            return 0
        fi
        attempt=$(( attempt + 1 ))
        printf 'pkg-framework: retry %d/%d after %ds (cmd: %s)\n' \
            "$attempt" "$n" "$s" "$*" >&2
        sleep "$s"
    done
    return 1
}

install_to() {
    # install_to <src> <dest> <mode>
    install -D -m "$3" "$1" "$2"
}

validate_git_commit_hex() {
    # accepts only 40-char lowercase hex (full SHA-1 commit) or "unknown"
    case "$1" in
        unknown) return 0 ;;
        *)
            if [[ "$1" =~ ^[0-9a-f]{40}$ ]]; then
                return 0
            fi
            return 1
            ;;
    esac
}

# -------------------------------------------------------------------------
# Section 3 -- Manifest validation
# -------------------------------------------------------------------------
_pkg_require_var() {
    local name=$1
    if [[ -z "${!name:-}" ]]; then
        printf 'pkg-framework: ERROR: pkg/project.sh missing required %s\n' \
            "$name" >&2
        return 2
    fi
}

_pkg_require_array() {
    local name=$1
    # Indirect array reference: ${name[@]} expansion.
    local -n ref=$name 2>/dev/null || {
        printf 'pkg-framework: ERROR: pkg/project.sh missing required array %s\n' \
            "$name" >&2
        return 2
    }
    if [[ "${#ref[@]}" -eq 0 ]]; then
        printf 'pkg-framework: ERROR: pkg/project.sh array %s is empty\n' \
            "$name" >&2
        return 2
    fi
}

_pkg_validate_manifest() {
    local rc=0
    for v in PKG_NAME PKG_PREFIX PKG_SUMMARY PKG_DESCRIPTION PKG_VENDOR \
             PKG_MAINTAINER PKG_HOMEPAGE_URL PKG_SOURCE_URL \
             PKG_LICENSE_SPDX PKG_LICENSE_NAME \
             PKG_COPYRIGHT_HOLDERS PKG_COPYRIGHT_YEAR; do
        _pkg_require_var "$v" || rc=2
    done
    for a in PKG_BINARIES PKG_DEB_DEPENDS; do
        _pkg_require_array "$a" || rc=2
    done

    # PKG_NAME must be lowercase alnum + hyphen (debian package-name rules).
    if [[ -n "${PKG_NAME:-}" && ! "$PKG_NAME" =~ ^[a-z0-9][a-z0-9-]*$ ]]; then
        printf 'pkg-framework: ERROR: PKG_NAME=%s violates debian naming (lowercase alnum + hyphen)\n' \
            "$PKG_NAME" >&2
        rc=2
    fi

    # PKG_PREFIX must be uppercase alnum + underscore (env-var prefix).
    if [[ -n "${PKG_PREFIX:-}" && ! "$PKG_PREFIX" =~ ^[A-Z][A-Z0-9_]*$ ]]; then
        printf 'pkg-framework: ERROR: PKG_PREFIX=%s must be UPPERCASE alnum + underscore\n' \
            "$PKG_PREFIX" >&2
        rc=2
    fi

    return $rc
}

# -------------------------------------------------------------------------
# Section 4 -- Input + env validation (called per build invocation)
# -------------------------------------------------------------------------
_pkg_validate_env() {
    local rc=0
    if [[ -z "${DISTRO:-}" ]]; then
        printf 'pkg-framework: ERROR: DISTRO not set (expected: deb|rpm)\n' >&2
        rc=2
    elif [[ "$DISTRO" != "deb" && "$DISTRO" != "rpm" ]]; then
        printf 'pkg-framework: ERROR: DISTRO=%s invalid (expected: deb|rpm)\n' \
            "$DISTRO" >&2
        rc=2
    fi

    if [[ -z "${VERSION:-}" ]]; then
        printf 'pkg-framework: ERROR: VERSION not set\n' >&2
        rc=2
    elif [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.+-]+)?$ ]]; then
        printf 'pkg-framework: ERROR: VERSION=%s violates semver\n' "$VERSION" >&2
        rc=2
    fi

    if [[ -z "${OUTDIR:-}" ]]; then
        printf 'pkg-framework: ERROR: OUTDIR not set\n' >&2
        rc=2
    elif [[ "$OUTDIR" != /* ]]; then
        printf 'pkg-framework: ERROR: OUTDIR=%s must be an absolute path\n' \
            "$OUTDIR" >&2
        rc=2
    fi

    return $rc
}

_pkg_validate_version_matches_cargo() {
    # Cross-check VERSION against Cargo.toml [package] version. Skips
    # silently if Cargo.toml is absent (allows non-Rust fixtures).
    local cargo="$REPO_ROOT/Cargo.toml"
    if [[ ! -r "$cargo" ]]; then
        return 0
    fi
    local cargo_ver
    cargo_ver=$(awk '
        /^\[package\]/ { in_pkg = 1; next }
        /^\[/ { in_pkg = 0; next }
        in_pkg && /^version[[:space:]]*=/ {
            gsub(/[" ]/, "", $0)
            sub(/^version=/, "", $0)
            print
            exit
        }' "$cargo")
    if [[ -z "$cargo_ver" ]]; then
        printf 'pkg-framework: WARN: could not parse [package].version from Cargo.toml\n' >&2
        return 0
    fi
    if [[ "$VERSION" != "$cargo_ver" ]]; then
        printf 'pkg-framework: ERROR: VERSION=%s does not match Cargo.toml [package].version=%s\n' \
            "$VERSION" "$cargo_ver" >&2
        return 2
    fi
}

# -------------------------------------------------------------------------
# Section 5 -- Global env setup
# -------------------------------------------------------------------------
_pkg_setup_globals() {
    umask 022

    # Reproducible mtimes: SOURCE_DATE_EPOCH is honored by fpm + rust +
    # strip-nondeterminism. Default is overridable via project manifest.
    export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$SOURCE_DATE_EPOCH_DEFAULT}"

    # Locale + tz: C.UTF-8 avoids locale-sensitive sort order in
    # tarball entries; UTC mtimes match SOURCE_DATE_EPOCH.
    export LC_ALL=C.UTF-8
    export LANG=C.UTF-8
    export TZ=UTC

    # Per-distro cargo target dir keeps deb and rpm builds isolated.
    # The workflow caches each separately.
    export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target/$DISTRO}"

    # Tell rustc to embed only relative paths in panic strings + debug
    # info. Required for byte-for-byte reproducibility.
    export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$REPO_ROOT=. --remap-path-prefix=$HOME=/build/home"

    # Output dirs (under STAGE).
    mkdir -p "$OUTDIR"

    # STAGE: scratch tree that mirrors the installed filesystem. fpm
    # tars this verbatim. Cleaned via trap on exit.
    STAGE="$(mktemp -d -t pkg-framework-stage.XXXXXX)"
    DEB_OUT="$STAGE/deb-root"
    RPM_OUT="$STAGE/rpm-root"
    mkdir -p "$DEB_OUT" "$RPM_OUT" "$STAGE/scripts"

    # Single cleanup trap. Caller may override by exporting
    # PKG_KEEP_STAGE=1 (debug aid).
    trap '_pkg_cleanup' EXIT INT TERM

    log "REPO_ROOT=$REPO_ROOT"
    log "STAGE=$STAGE"
    log "DISTRO=$DISTRO VERSION=$VERSION OUTDIR=$OUTDIR"
    log "SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH"
    log "FRAMEWORK_VERSION=$PKG_FRAMEWORK_VERSION_RUNTIME"
}

_pkg_cleanup() {
    local rc=$?
    if [[ -n "${PKG_KEEP_STAGE:-}" ]]; then
        printf 'pkg-framework: PKG_KEEP_STAGE set; leaving %s\n' "$STAGE" >&2
    elif [[ -n "$STAGE" && -d "$STAGE" ]]; then
        rm -rf "$STAGE"
    fi
    exit "$rc"
}

# -------------------------------------------------------------------------
# Section 6 -- Dependency install (apt / dnf)
# -------------------------------------------------------------------------
_pkg_install_deb_deps() {
    section "install deb build deps"
    if declare -F project_pre_install_deb_deps >/dev/null; then
        project_pre_install_deb_deps
    fi

    local base_deps=(
        ca-certificates
        curl
        build-essential
        pkg-config
        ruby
        ruby-dev
        rubygems
        rpm
        file
        binutils
        coreutils
        sed
        gawk
        grep
        jq
        xz-utils
        zstd
        strip-nondeterminism
    )
    local extras=()
    if [[ -n "${PKG_EXTRA_DEB_BUILD_DEPS+x}" ]]; then
        extras=("${PKG_EXTRA_DEB_BUILD_DEPS[@]}")
    fi

    run sudo apt-get update -qq
    run sudo apt-get install -y --no-install-recommends \
        "${base_deps[@]}" "${extras[@]}"

    _pkg_ensure_fpm

    if declare -F project_post_install_deb_deps >/dev/null; then
        project_post_install_deb_deps
    fi
}

_pkg_install_rpm_deps() {
    section "install rpm build deps"
    if declare -F project_pre_install_rpm_deps >/dev/null; then
        project_pre_install_rpm_deps
    fi

    local base_deps=(
        ca-certificates
        curl
        gcc
        gcc-c++
        make
        pkgconf-pkg-config
        rpm-build
        ruby
        ruby-devel
        rubygems
        file
        binutils
        coreutils
        sed
        gawk
        grep
        jq
        xz
        zstd
    )
    local extras=()
    if [[ -n "${PKG_EXTRA_RPM_BUILD_DEPS+x}" ]]; then
        extras=("${PKG_EXTRA_RPM_BUILD_DEPS[@]}")
    fi

    run sudo dnf -y install "${base_deps[@]}" "${extras[@]}"

    _pkg_ensure_fpm

    if declare -F project_post_install_rpm_deps >/dev/null; then
        project_post_install_rpm_deps
    fi
}

_pkg_ensure_fpm() {
    if command -v fpm >/dev/null 2>&1; then
        local current
        current=$(fpm --version 2>/dev/null | head -1 || echo unknown)
        if [[ "$current" == "$FPM_VERSION" ]]; then
            log "fpm $FPM_VERSION already installed"
            return 0
        fi
        log "fpm $current present; pinning to $FPM_VERSION"
    fi
    retry 3 5 sudo gem install --no-document fpm -v "$FPM_VERSION"
}

# -------------------------------------------------------------------------
# Section 7 -- Toolchain (rustup) + binary build
# -------------------------------------------------------------------------
_pkg_ensure_toolchain() {
    section "ensure rust toolchain"
    if ! command -v cargo >/dev/null 2>&1; then
        log "installing rustup"
        retry 3 5 sh -c \
            'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal'
        # shellcheck disable=SC1091
        . "$HOME/.cargo/env"
    fi
    run rustc --version
    run cargo --version
}

_pkg_build_binaries() {
    section "build binaries (release, reproducible)"
    if declare -F project_pre_build >/dev/null; then
        project_pre_build
    fi

    local bin
    local build_args=(--release --locked)
    if [[ -n "${PKG_CARGO_FEATURES:-}" ]]; then
        build_args+=(--no-default-features --features "$PKG_CARGO_FEATURES")
    fi

    # v1.2.2: optional hermetic build. With PKG_CARGO_OFFLINE=1, we
    # pre-fetch deps with --locked, then compile with --frozen
    # --offline so the build step cannot touch the network. Useful
    # for reproducibility audits and for sandboxed CI. Default off
    # to preserve v1.2.x backward compatibility.
    if [[ "${PKG_CARGO_OFFLINE:-0}" == "1" ]]; then
        run cargo fetch --locked
        build_args+=(--frozen --offline)
    fi

    for bin in "${PKG_BINARIES[@]}"; do
        run cargo build "${build_args[@]}" --bin "$bin"
    done

    # Strip debug symbols (matches debian + rpm policy).
    for bin in "${PKG_BINARIES[@]}"; do
        local p="$CARGO_TARGET_DIR/release/$bin"
        if [[ -f "$p" ]]; then
            run strip --strip-unneeded "$p"
        fi
    done

    if declare -F project_post_build >/dev/null; then
        project_post_build
    fi
}

# -------------------------------------------------------------------------
# Section 8 -- Stage assets (the installed filesystem tree)
# -------------------------------------------------------------------------
_pkg_stage_assets() {
    local root=$1   # DEB_OUT or RPM_OUT

    section "stage assets into $root"

    # Binaries.
    local bin
    for bin in "${PKG_BINARIES[@]}"; do
        install_to "$CARGO_TARGET_DIR/release/$bin" "$root/usr/bin/$bin" 0755
    done

    # systemd units.
    if [[ -n "${PKG_SYSTEMD_UNITS+x}" ]]; then
        local unit src
        for unit in "${PKG_SYSTEMD_UNITS[@]}"; do
            src="$REPO_ROOT/systemd/$unit"
            if [[ ! -r "$src" ]]; then
                printf 'pkg-framework: ERROR: PKG_SYSTEMD_UNITS lists %s but %s not found\n' \
                    "$unit" "$src" >&2
                return 1
            fi
            install_to "$src" "$root/lib/systemd/system/$unit" 0644
        done
    fi

    # Doc tree.
    local doc_dir="$root/usr/share/doc/$PKG_NAME"
    install -d -m 0755 "$doc_dir"

    if [[ -r "$REPO_ROOT/README.md" ]]; then
        install_to "$REPO_ROOT/README.md" "$doc_dir/README.md" 0644
    fi

    if [[ -r "$REPO_ROOT/CHANGELOG.md" ]]; then
        # changelog.gz with reproducible timestamp.
        local tmp_changelog
        tmp_changelog=$(mktemp)
        gzip -n -9 < "$REPO_ROOT/CHANGELOG.md" > "$tmp_changelog"
        install_to "$tmp_changelog" "$doc_dir/changelog.gz" 0644
        rm -f "$tmp_changelog"
    fi

    # debian/copyright for deb; LICENSE for rpm.
    if [[ "$root" == "$DEB_OUT" ]]; then
        _pkg_emit_debian_copyright > "$doc_dir/copyright"
        chmod 0644 "$doc_dir/copyright"
    else
        if [[ -r "$REPO_ROOT/LICENSE" ]]; then
            install_to "$REPO_ROOT/LICENSE" "$doc_dir/LICENSE" 0644
        fi
    fi

    # Extra doc files declared by the project (relative repo paths).
    if [[ -n "${PKG_EXTRA_DOC_FILES+x}" ]]; then
        local f base
        for f in "${PKG_EXTRA_DOC_FILES[@]}"; do
            if [[ ! -r "$REPO_ROOT/$f" ]]; then
                printf 'pkg-framework: ERROR: PKG_EXTRA_DOC_FILES lists %s but file not found\n' \
                    "$f" >&2
                return 1
            fi
            base=$(basename "$f")
            install_to "$REPO_ROOT/$f" "$doc_dir/$base" 0644
        done
    fi

    # Project hook for everything else (configs, polkit, sudoers,
    # autostart, apt/dnf hooks, etc.).
    if declare -F project_stage_extra >/dev/null; then
        # Hook receives the staging root as $1 so it can place files
        # under any prefix.
        project_stage_extra "$root"
    fi
}

# -------------------------------------------------------------------------
# Section 9 -- debian/copyright (machine-readable format 1.0)
# -------------------------------------------------------------------------
_pkg_emit_debian_copyright() {
    # v1.2.2: emit a real debian copyright file. The previous shape
    # pointed the license body at changelog.gz, which was wrong.
    # For GPL-3.0-only we reference the common-licenses path that
    # ships in every debian/ubuntu base. For other licenses we
    # inline the LICENSE file body when it is present in REPO_ROOT;
    # otherwise we point at the source URL.
    local license_block=""
    case "$PKG_LICENSE_SPDX" in
        GPL-3.0-only|GPL-3.0+|GPL-3)
            license_block=$'On Debian systems, the complete text of the GNU\n General Public License version 3 can be found in\n /usr/share/common-licenses/GPL-3.'
            ;;
        GPL-2.0-only|GPL-2.0+|GPL-2)
            license_block=$'On Debian systems, the complete text of the GNU\n General Public License version 2 can be found in\n /usr/share/common-licenses/GPL-2.'
            ;;
        LGPL-3.0-only|LGPL-3.0+)
            license_block=$'On Debian systems, the complete text of the GNU\n Lesser General Public License version 3 can be found in\n /usr/share/common-licenses/LGPL-3.'
            ;;
        Apache-2.0)
            license_block=$'On Debian systems, the complete text of the\n Apache License version 2.0 can be found in\n /usr/share/common-licenses/Apache-2.0.'
            ;;
        *)
            if [[ -r "${REPO_ROOT:-}/LICENSE" ]]; then
                # Indent each line of LICENSE with a single space per
                # debian copyright format. Blank lines become ".".
                license_block=$(sed -e 's/^$/./' -e 's/^/ /' "${REPO_ROOT}/LICENSE")
            else
                license_block=" The full text of the $PKG_LICENSE_NAME license is available at"$'\n'" $PKG_SOURCE_URL."
            fi
            ;;
    esac

    cat <<EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: $PKG_NAME
Upstream-Contact: $PKG_MAINTAINER
Source: $PKG_SOURCE_URL

Files: *
Copyright: $PKG_COPYRIGHT_YEAR $PKG_COPYRIGHT_HOLDERS
License: $PKG_LICENSE_SPDX

License: $PKG_LICENSE_SPDX
$license_block
EOF
}

# -------------------------------------------------------------------------
# Section 10 -- Maintainer scripts (postinst / prerm / postrm)
# -------------------------------------------------------------------------
_pkg_emit_postinst() {
    local out=$1
    {
        cat <<'SHEAD'
#!/bin/sh
# Auto-generated by pkg-framework. Do not edit; modify
# project_postinst_body() in pkg/project.sh instead.
set -e

SHEAD
        if declare -F project_postinst_body >/dev/null; then
            project_postinst_body
            printf '\n'
        fi

        # systemd-aware service reload (default behavior; project may
        # override by returning early from project_postinst_body, but the
        # framework includes this so projects that ship units get the
        # right behavior for free).
        if [[ -n "${PKG_SYSTEMD_UNITS+x}" && "${#PKG_SYSTEMD_UNITS[@]}" -gt 0 ]]; then
            cat <<'SSYS'
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload >/dev/null 2>&1 || true
fi

SSYS
        fi

        printf 'exit 0\n'
    } > "$out"
    chmod 0755 "$out"
}

_pkg_emit_prerm() {
    local out=$1
    if ! declare -F project_prerm_body >/dev/null; then
        return 0
    fi
    {
        printf '#!/bin/sh\nset -e\n\n'
        project_prerm_body
        printf '\nexit 0\n'
    } > "$out"
    chmod 0755 "$out"
}

_pkg_emit_postrm() {
    local out=$1
    if ! declare -F project_postrm_body >/dev/null; then
        return 0
    fi
    {
        printf '#!/bin/sh\nset -e\n\n'
        project_postrm_body
        printf '\nexit 0\n'
    } > "$out"
    chmod 0755 "$out"
}

# -------------------------------------------------------------------------
# Section 11 -- fpm invocations
# -------------------------------------------------------------------------
# v1.2.2: build common fpm args into a caller-provided array name
# (bash nameref) instead of returning newline-delimited stdout. The
# old shape corrupted PKG_DESCRIPTION when it contained literal
# newlines (each line after the first became a positional fpm path).
# Docs promise multi-line descriptions; this is what makes them work.
_pkg_fpm_common_args() {
    local -n _out=$1
    _out+=(
        --name        "$PKG_NAME"
        --version     "$VERSION"
        --vendor      "$PKG_VENDOR"
        --maintainer  "$PKG_MAINTAINER"
        --url         "$PKG_HOMEPAGE_URL"
        --license     "$PKG_LICENSE_SPDX"
        --description "$PKG_DESCRIPTION"
        --architecture native
    )
}

_pkg_fpm_deb() {
    section "build deb"

    local postinst="$STAGE/scripts/postinst"
    local prerm="$STAGE/scripts/prerm"
    local postrm="$STAGE/scripts/postrm"
    _pkg_emit_postinst "$postinst"
    _pkg_emit_prerm "$prerm"
    _pkg_emit_postrm "$postrm"

    local out_file="$OUTDIR/${PKG_NAME}_${VERSION}${PKG_DEB_ARTIFACT_SUFFIX:-}_amd64.deb"
    rm -f "$out_file"

    # Build args list.
    local args=(
        -s dir
        -t deb
        --deb-no-default-config-files
        --deb-compression xz
        --deb-priority "$PKG_DEB_PRIORITY"
        --category "$PKG_DEB_SECTION"
        --chdir "$DEB_OUT"
        --package "$out_file"
        --after-install "$postinst"
    )

    if [[ -f "$prerm" ]]; then
        args+=(--before-remove "$prerm")
    fi
    if [[ -f "$postrm" ]]; then
        args+=(--after-remove "$postrm")
    fi

    local d
    for d in "${PKG_DEB_DEPENDS[@]}"; do
        args+=(--depends "$d")
    done

    if [[ -n "${PKG_DEB_CONFIG_FILES+x}" ]]; then
        local cf
        for cf in "${PKG_DEB_CONFIG_FILES[@]}"; do
            args+=(--config-files "$cf")
        done
    fi

    if [[ -n "${PKG_DEB_CONFLICTS+x}" ]]; then
        local c
        for c in "${PKG_DEB_CONFLICTS[@]}"; do
            args+=(--conflicts "$c")
        done
    fi

    if [[ -n "${PKG_DEB_REPLACES+x}" ]]; then
        local r
        for r in "${PKG_DEB_REPLACES[@]}"; do
            args+=(--replaces "$r")
        done
    fi

    # Common args. v1.2.2: fill via nameref so multi-line
    # PKG_DESCRIPTION survives intact.
    local common_arr=()
    _pkg_fpm_common_args common_arr

    # Project-supplied extras.
    if declare -F project_fpm_deb_extra_args >/dev/null; then
        local extra
        while IFS= read -r extra; do
            [[ -n "$extra" ]] && args+=("$extra")
        done < <(project_fpm_deb_extra_args)
    fi

    # v1.2.4: redirect fpm's own stdout to stderr. fpm prints a
    # `Created package` log line on success; if we let it flow it
    # ends up in `artifact=$(_pkg_fpm_deb)` alongside the path we
    # actually want to return. Stdout from this function is the
    # return channel; everything else goes to stderr.
    run fpm "${args[@]}" "${common_arr[@]}" . >&2

    log "wrote $out_file"
    printf '%s' "$out_file"
}

_pkg_fpm_rpm() {
    section "build rpm"

    local postinst="$STAGE/scripts/postinst"
    local prerm="$STAGE/scripts/prerm"
    local postrm="$STAGE/scripts/postrm"
    _pkg_emit_postinst "$postinst"
    _pkg_emit_prerm "$prerm"
    _pkg_emit_postrm "$postrm"

    local rpm_release="${PKG_RPM_RELEASE:-1}"
    local out_file="$OUTDIR/${PKG_NAME}-${VERSION}-${rpm_release}.x86_64.rpm"
    rm -f "$out_file"

    local args=(
        -s dir
        -t rpm
        --rpm-compression xzmt
        --rpm-os linux
        --rpm-summary "$PKG_SUMMARY"
        --category "$PKG_RPM_GROUP"
        --chdir "$RPM_OUT"
        --package "$out_file"
        --iteration "$rpm_release"
        --after-install "$postinst"
    )

    if [[ -f "$prerm" ]]; then
        args+=(--before-remove "$prerm")
    fi
    if [[ -f "$postrm" ]]; then
        args+=(--after-remove "$postrm")
    fi

    # rpm depends: prefer explicit PKG_RPM_REQUIRES, else translate from
    # PKG_DEB_DEPENDS (best-effort: strip " (>= ...)" qualifiers, drop
    # debian-only names).
    local depends_list=()
    if [[ -n "${PKG_RPM_REQUIRES+x}" && "${#PKG_RPM_REQUIRES[@]}" -gt 0 ]]; then
        depends_list=("${PKG_RPM_REQUIRES[@]}")
    else
        local raw
        for raw in "${PKG_DEB_DEPENDS[@]}"; do
            local clean="${raw%% *}"
            depends_list+=("$clean")
        done
    fi

    local d
    for d in "${depends_list[@]}"; do
        args+=(--depends "$d")
    done

    if [[ -n "${PKG_RPM_CONFLICTS+x}" ]]; then
        local c
        for c in "${PKG_RPM_CONFLICTS[@]}"; do
            args+=(--conflicts "$c")
        done
    fi

    local common_arr=()
    _pkg_fpm_common_args common_arr

    if declare -F project_fpm_rpm_extra_args >/dev/null; then
        local extra
        while IFS= read -r extra; do
            [[ -n "$extra" ]] && args+=("$extra")
        done < <(project_fpm_rpm_extra_args)
    fi

    # v1.2.4: redirect fpm's own stdout to stderr (see _pkg_fpm_deb).
    run fpm "${args[@]}" "${common_arr[@]}" . >&2

    log "wrote $out_file"
    printf '%s' "$out_file"
}

# -------------------------------------------------------------------------
# Section 12 -- Stage validation (run BEFORE fpm)
# -------------------------------------------------------------------------
_pkg_validate_stage() {
    local root=$1
    section "validate stage tree $root"

    local fails=()

    local bin
    for bin in "${PKG_BINARIES[@]}"; do
        local p="$root/usr/bin/$bin"
        if [[ ! -x "$p" ]]; then
            fails+=("missing or non-executable: usr/bin/$bin")
        fi
    done

    if [[ -n "${PKG_SYSTEMD_UNITS+x}" ]]; then
        local unit
        for unit in "${PKG_SYSTEMD_UNITS[@]}"; do
            if [[ ! -f "$root/lib/systemd/system/$unit" ]]; then
                fails+=("missing systemd unit: lib/systemd/system/$unit")
            fi
        done
    fi

    # PKG_LAYOUT_CHECKS entries: "path:mode" (mode optional).
    if [[ -n "${PKG_LAYOUT_CHECKS+x}" ]]; then
        local entry path mode actual
        for entry in "${PKG_LAYOUT_CHECKS[@]}"; do
            path="${entry%%:*}"
            mode=""
            if [[ "$entry" == *:* ]]; then
                mode="${entry#*:}"
            fi
            if [[ ! -e "$root/$path" ]]; then
                fails+=("missing: $path")
                continue
            fi
            if [[ -n "$mode" ]]; then
                actual=$(stat -c '%a' "$root/$path")
                if [[ "$actual" != "$mode" ]]; then
                    fails+=("mode mismatch: $path expected $mode got $actual")
                fi
            fi
        done
    fi

    if declare -F project_validate_stage_extra >/dev/null; then
        project_validate_stage_extra "$root" || \
            fails+=("project_validate_stage_extra reported failures")
    fi

    if [[ "${#fails[@]}" -gt 0 ]]; then
        printf 'pkg-framework: stage validation FAILED:\n' >&2
        local f
        for f in "${fails[@]}"; do
            printf '  - %s\n' "$f" >&2
        done
        return 1
    fi
    log "stage tree OK"
}

# -------------------------------------------------------------------------
# Section 13 -- Reproducibility pass + artifact validation
# -------------------------------------------------------------------------
_pkg_make_reproducible() {
    local artifact=$1
    section "normalize for reproducibility: $artifact"
    case "$artifact" in
        *.deb)
            if command -v strip-nondeterminism >/dev/null 2>&1; then
                run strip-nondeterminism "$artifact"
            else
                printf 'pkg-framework: WARN: strip-nondeterminism not available\n' >&2
            fi
            ;;
        *.rpm)
            # fpm + SOURCE_DATE_EPOCH already produce stable rpm headers.
            log "rpm reproducibility relies on SOURCE_DATE_EPOCH only"
            ;;
    esac
}

_pkg_validate_artifact() {
    local artifact=$1
    section "validate artifact $artifact"

    if [[ ! -s "$artifact" ]]; then
        printf 'pkg-framework: ERROR: %s missing or empty\n' "$artifact" >&2
        return 1
    fi

    case "$artifact" in
        *.deb)
            run dpkg-deb -I "$artifact"
            # v1.2.2: capture the full listing into a variable, then
            # head from it. Piping `dpkg-deb -c | head -20` SIGPIPEs
            # the producer when the package has more than 20 entries;
            # under set -o pipefail that fails the build for a
            # perfectly valid artifact.
            local listing
            listing=$(dpkg-deb -c "$artifact")
            printf '%s\n' "$listing" | head -20 >&2
            ;;
        *.rpm)
            run rpm -qpi "$artifact"
            local listing
            listing=$(rpm -qpl "$artifact")
            printf '%s\n' "$listing" | head -20 >&2
            ;;
    esac
}

# -------------------------------------------------------------------------
# Section 14 -- Manifest sidecar
# -------------------------------------------------------------------------
_pkg_emit_manifest() {
    local artifact=$1
    local manifest="${artifact}.manifest.json"

    local sha256 size
    sha256=$(sha256sum "$artifact" | awk '{print $1}')
    size=$(stat -c '%s' "$artifact")

    # Manifest commit: env var named ${PKG_PREFIX}_MANIFEST_COMMIT lets
    # the workflow inject the source commit without polluting the
    # framework's namespace. Falls back to "unknown".
    local commit_var="${PKG_PREFIX}_MANIFEST_COMMIT"
    local commit="${!commit_var:-unknown}"
    if ! validate_git_commit_hex "$commit"; then
        printf 'pkg-framework: ERROR: %s=%s is not a valid 40-char hex commit\n' \
            "$commit_var" "$commit" >&2
        return 2
    fi

    local now
    now=$(date -u -d "@$SOURCE_DATE_EPOCH" +%FT%TZ)

    cat > "$manifest" <<EOF
{
  "schema": "pkg-framework-manifest/1",
  "framework_version": "$PKG_FRAMEWORK_VERSION_RUNTIME",
  "package": "$PKG_NAME",
  "version": "$VERSION",
  "distro": "$DISTRO",
  "artifact": "$(basename "$artifact")",
  "sha256": "$sha256",
  "size_bytes": $size,
  "source_commit": "$commit",
  "source_date_epoch": $SOURCE_DATE_EPOCH,
  "built_at": "$now"
}
EOF
    log "wrote $manifest"
}

# -------------------------------------------------------------------------
# Section 15 -- Summary
# -------------------------------------------------------------------------
_pkg_print_summary() {
    local artifact=$1
    section "summary"
    printf 'package:   %s\n' "$PKG_NAME"
    printf 'version:   %s\n' "$VERSION"
    printf 'distro:    %s\n' "$DISTRO"
    printf 'artifact:  %s\n' "$artifact"
    printf 'manifest:  %s.manifest.json\n' "$artifact"
    printf 'framework: %s\n' "$PKG_FRAMEWORK_VERSION_RUNTIME"
    printf 'outdir:    %s\n' "$OUTDIR"
}

# -------------------------------------------------------------------------
# Section 16 -- Entry point
# -------------------------------------------------------------------------
run_pkg_build() {
    set -euo pipefail

    _pkg_compute_paths
    _pkg_check_framework_version
    _pkg_validate_manifest
    _pkg_validate_env
    _pkg_validate_version_matches_cargo
    _pkg_setup_globals

    case "$DISTRO" in
        deb) _pkg_install_deb_deps ;;
        rpm) _pkg_install_rpm_deps ;;
    esac

    _pkg_ensure_toolchain
    _pkg_build_binaries

    local root artifact
    case "$DISTRO" in
        deb)
            root="$DEB_OUT"
            _pkg_stage_assets "$root"
            _pkg_validate_stage "$root"
            artifact="$(_pkg_fpm_deb)"
            ;;
        rpm)
            root="$RPM_OUT"
            _pkg_stage_assets "$root"
            _pkg_validate_stage "$root"
            artifact="$(_pkg_fpm_rpm)"
            ;;
    esac

    _pkg_make_reproducible "$artifact"
    _pkg_validate_artifact "$artifact"
    _pkg_emit_manifest "$artifact"
    _pkg_print_summary "$artifact"
}
