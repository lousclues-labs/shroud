#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
# Development tool: build, install, and restart shroud
# Moved from `shroud update` CLI command (Principle VIII: One Binary, One Purpose)

set -e

cd "$(dirname "$0")/.."

INSTALL_DIR="$HOME/.local/bin"
BINARY="$INSTALL_DIR/shroud"
SERVICE="app-shroud@autostart.service"

echo "Building shroud..."
# Build and install exactly like setup.sh. Using `cargo install` here would
# place a second copy in ~/.cargo/bin, which precedes ~/.local/bin on a default
# PATH and would then shadow every subsequent build.
cargo build --release "${@}"

echo "Installing to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"
# Atomic binary replacement: write beside the target then rename.
# This avoids the rm+cp pattern that triggers /proc/self/exe "(deleted)"
# and breaks the restart path. mv on the same filesystem is atomic.
install -m 755 target/release/shroud "$INSTALL_DIR/.shroud.new"
mv "$INSTALL_DIR/.shroud.new" "$BINARY"

# Clear out a copy left by older versions of this script, which otherwise keeps
# winning on PATH and makes this update look like it had no effect.
if [ -e "$HOME/.cargo/bin/shroud" ]; then
    stale="$HOME/.cargo/bin/shroud.stale.$(date +%Y%m%d%H%M%S)"
    echo "Moving stale $HOME/.cargo/bin/shroud aside (it shadows $BINARY on PATH)"
    mv "$HOME/.cargo/bin/shroud" "$stale"
fi

echo "Restarting daemon..."
"$BINARY" quit 2>/dev/null || true

# `quit` is best-effort: a stale socket can leave the old daemon running, and it
# would then race the new instance for shroud.sock. Confirm it is gone first.
for _ in $(seq 1 10); do
    pgrep -x shroud > /dev/null || break
    sleep 0.5
done
if pgrep -x shroud > /dev/null; then
    echo "Old daemon did not exit on quit; stopping it"
    pkill -x shroud || true
    sleep 1
fi

# Prefer the user service: it keeps exactly one instance owning the IPC socket
# and starts the daemon as a session child so the tray can register. Launching
# with nohup alongside an enabled autostart unit produces two daemons racing
# the same socket.
if systemctl --user cat "$SERVICE" > /dev/null 2>&1; then
    systemctl --user restart "$SERVICE"
else
    nohup "$BINARY" > /dev/null 2>&1 &
fi

sleep 2
if "$BINARY" ping > /dev/null 2>&1; then
    echo "Daemon restarted successfully"
else
    echo "Warning: Daemon may not have started. Run 'shroud' manually."
fi

echo ""
"$BINARY" --version
echo "✓ Update complete"
