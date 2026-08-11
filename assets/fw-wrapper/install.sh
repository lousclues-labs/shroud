#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
#
# Install the scoped firewall wrapper and its sudoers rule.
#
#   sudo ./assets/fw-wrapper/install.sh
#
# Installs:
#   /usr/local/lib/shroud/shroud-fw-lib.sh   (validator, root:root 0644)
#   /usr/local/lib/shroud/shroud-iptables    (stub, root:root 0755)
#   /usr/local/lib/shroud/shroud-ip6tables   (stub, root:root 0755)
#   /usr/local/lib/shroud/shroud-nft         (stub, root:root 0755)
#   /etc/sudoers.d/shroud                     (scoped NOPASSWD, root:root 0440)
#
# The sudoers rule is validated with `visudo -c` before it is placed, so a
# malformed file can never land and lock you out of sudo.

set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "error: must run as root (use sudo)" >&2
    exit 1
fi

unset CDPATH
SRC=$(cd -- "$(dirname -- "$0")" && pwd)
DEST=/usr/local/lib/shroud
KSUSER=${SUDO_USER:-$(id -un)}

install -d -m 0755 -o root -g root "$DEST"
install -m 0644 -o root -g root "$SRC/shroud-fw-lib.sh" "$DEST/shroud-fw-lib.sh"
for t in iptables ip6tables nft; do
    install -m 0755 -o root -g root "$SRC/shroud-$t" "$DEST/shroud-$t"
done

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
sed "s/@USER@/$KSUSER/g" "$SRC/sudoers.shroud-fw.in" > "$tmp"

if ! visudo -c -f "$tmp" >/dev/null; then
    echo "error: generated sudoers failed validation; not installing" >&2
    exit 1
fi

install -m 0440 -o root -g root "$tmp" /etc/sudoers.d/shroud

echo "Installed shroud-fw wrapper and scoped sudoers for user: $KSUSER"
echo "Verify: sudo -n /usr/local/lib/shroud/shroud-nft list tables"
