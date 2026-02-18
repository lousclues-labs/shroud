#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
# Development tool: build, install, and restart shroud
# Moved from `shroud update` CLI command (Principle VIII: One Binary, One Purpose)

set -e

cd "$(dirname "$0")/.."

echo "Building and installing shroud..."
cargo install --path . --force "${@}"

echo "Copying to ~/.local/bin..."
# Atomic binary replacement: copy to temp file then rename.
# This avoids the rm+cp pattern that triggers /proc/self/exe "(deleted)"
# and breaks the restart path. mv on the same filesystem is atomic.
cp ~/.cargo/bin/shroud ~/.local/bin/.shroud.new
chmod 755 ~/.local/bin/.shroud.new
mv ~/.local/bin/.shroud.new ~/.local/bin/shroud

echo "Restarting daemon..."
# Stop the old daemon gracefully, then start the new binary directly.
# We can't rely on IPC 'restart' because the old daemon may have a different
# version of resolve_restart_path() that can't find the new binary.
shroud quit 2>/dev/null || true
sleep 1
# Start the new daemon (the binary at ~/.local/bin/shroud is now the new version)
nohup ~/.local/bin/shroud > /dev/null 2>&1 &
sleep 1
# Verify it's running
if shroud ping > /dev/null 2>&1; then
    echo "Daemon restarted successfully"
else
    echo "Warning: Daemon may not have started. Run 'shroud' manually."
fi

echo ""
shroud --version
echo "✓ Update complete"
