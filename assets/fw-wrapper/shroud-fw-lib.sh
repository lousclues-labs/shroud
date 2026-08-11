# shellcheck shell=sh
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
#
# VPN Shroud firewall wrapper — privileged argument validator.
#
# Sourced by the per-tool stubs (shroud-iptables / shroud-ip6tables /
# shroud-nft) after they export SHROUD_FW_TOOL and SHROUD_FW_REAL. The stubs
# are the only commands granted passwordless sudo, so this validator is the
# trust boundary: it constrains privileged firewall access to Shroud's own
# chains (SHROUD_KILLSWITCH, SHROUD_BOOT_KS) and nftables table
# (shroud_killswitch). Anything outside that surface is refused, so the
# NOPASSWD grant cannot be repurposed to rewrite the wider firewall.
#
# Design notes:
# - No `eval`, no shell interpolation of arguments; the real binary is invoked
#   with the original argv via exec (or a validated stdin pipe for `nft -f -`).
# - `set -u` only (not -e): grep "no match" must not abort the script.

set -u
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
umask 022

KS_CHAIN=SHROUD_KILLSWITCH
BOOT_CHAIN=SHROUD_BOOT_KS
NFT_TABLE=shroud_killswitch

SHROUD_FW_ARGS="$*"

deny() {
    printf 'shroud-fw: denied %s: %s\n' "${SHROUD_FW_TOOL:-?}" "$*" >&2
    if command -v logger >/dev/null 2>&1; then
        logger -t shroud-fw "denied ${SHROUD_FW_TOOL:-?}: $* [argv: ${SHROUD_FW_ARGS}]"
    fi
    exit 2
}

if [ -z "${SHROUD_FW_REAL:-}" ] || [ ! -x "${SHROUD_FW_REAL:-/nonexistent}" ]; then
    deny "real firewall binary not found"
fi

is_our_chain() {
    case "$1" in
        "$KS_CHAIN" | "$BOOT_CHAIN") return 0 ;;
        *) return 1 ;;
    esac
}

# Validate the rule body of an OUTPUT-chain insert/delete. Only a jump to one
# of Shroud's chains is allowed for both IP families; ip6tables additionally
# manages a fixed set of direct OUTPUT rules for IPv6 leak protection.
validate_output_body() {
    body="$*"
    case "$body" in
        "-j $KS_CHAIN" | "-j $BOOT_CHAIN") return 0 ;;
    esac
    if [ "$SHROUD_FW_TOOL" = ip6tables ]; then
        case "$body" in
            "-o lo -j ACCEPT") return 0 ;;
            "-j DROP") return 0 ;;
            "-m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT") return 0 ;;
            "-m state --state ESTABLISHED,RELATED -j ACCEPT") return 0 ;;
            "-o tun+ -j ACCEPT") return 0 ;;
            "-d fe80::/10 -j ACCEPT") return 0 ;;
        esac
    fi
    deny "OUTPUT rule body not in allowlist: $body"
}

validate_xtables() {
    # Optional leading table selector — filter table only.
    if [ "${1:-}" = "-t" ]; then
        [ "${2:-}" = "filter" ] || deny "table must be filter"
        shift 2
    fi
    op="${1:-}"
    case "$op" in
        --version) return 0 ;;
        -L | -S) return 0 ;; # read-only list/show
        -N | -F | -X)
            [ "$#" -eq 2 ] || deny "$op takes exactly one chain"
            is_our_chain "${2:-}" || deny "$op only on shroud chains"
            return 0
            ;;
        -A)
            is_our_chain "${2:-}" || deny "-A only into shroud chains"
            return 0
            ;;
        -C)
            { [ "${2:-}" = OUTPUT ] && [ "${3:-}" = -j ] && is_our_chain "${4:-}" && [ "$#" -eq 4 ]; } ||
                deny "-C only checks OUTPUT jump to a shroud chain"
            return 0
            ;;
        -I)
            [ "${2:-}" = OUTPUT ] || deny "-I only into OUTPUT"
            shift 2
            # optional numeric rule position
            case "${1:-}" in
                '' | *[!0-9]*) : ;;
                *) shift ;;
            esac
            validate_output_body "$@"
            return 0
            ;;
        -D)
            [ "${2:-}" = OUTPUT ] || deny "-D only from OUTPUT"
            shift 2
            validate_output_body "$@"
            return 0
            ;;
        *) deny "operation not allowed: $op" ;;
    esac
}

validate_nft_stdin() {
    buf=$(cat)
    [ -n "$buf" ] || deny "empty nft ruleset on stdin"
    case "$buf" in
        *"flush ruleset"*) deny "nft ruleset must not flush ruleset" ;;
        *include*) deny "nft ruleset must not include files" ;;
    esac
    # Every `table <family> <name>` reference must be our table.
    bad=$(printf '%s\n' "$buf" |
        grep -oE 'table[[:space:]]+[A-Za-z0-9_]+[[:space:]]+[A-Za-z0-9_./-]+' |
        grep -vE "^table[[:space:]]+inet[[:space:]]+${NFT_TABLE}$" || true)
    [ -z "$bad" ] || deny "nft ruleset references foreign table: $bad"
    printf '%s' "$buf" | "$SHROUD_FW_REAL" -f -
    exit $?
}

validate_nft() {
    case "${1:-}" in
        --version) return 0 ;;
        list)
            { [ "${2:-}" = tables ] && [ "$#" -eq 2 ]; } && return 0
            { [ "${2:-}" = table ] && [ "${3:-}" = inet ] && [ "${4:-}" = "$NFT_TABLE" ]; } && return 0
            deny "nft list not allowed: $*"
            ;;
        delete | flush)
            { [ "${2:-}" = table ] && [ "${3:-}" = inet ] && [ "${4:-}" = "$NFT_TABLE" ] && [ "$#" -eq 4 ]; } ||
                deny "nft ${1:-} only on inet ${NFT_TABLE}"
            return 0
            ;;
        add)
            { [ "${3:-}" = inet ] && [ "${4:-}" = "$NFT_TABLE" ]; } || deny "nft add only on inet ${NFT_TABLE}"
            return 0
            ;;
        -f)
            { [ "${2:-}" = "-" ] && [ "$#" -eq 2 ]; } || deny "nft -f only reads stdin (-)"
            validate_nft_stdin
            return 0
            ;;
        *) deny "nft op not allowed: ${1:-}" ;;
    esac
}

case "$SHROUD_FW_TOOL" in
    iptables | ip6tables) validate_xtables "$@" ;;
    nft) validate_nft "$@" ;;
    *) deny "unknown tool: ${SHROUD_FW_TOOL:-?}" ;;
esac

exec "$SHROUD_FW_REAL" "$@"
