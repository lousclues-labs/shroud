#!/bin/bash
# Development tool: build, install, and restart shroud
# Moved from `shroud update` CLI command (Principle VIII: One Binary, One Purpose)

set -e

cd "$(dirname "$0")/.."

echo "Building and installing shroud..."
cargo install --path . --force "${@}"

echo "Copying to ~/.local/bin..."
# Must remove first — cp fails with ETXTBSY if daemon has the binary mapped
rm -f ~/.local/bin/shroud 2>/dev/null || true
cp ~/.cargo/bin/shroud ~/.local/bin/shroud

echo "Restarting daemon..."
shroud restart 2>/dev/null || echo "Daemon not running"

echo ""
shroud --version
echo "✓ Update complete"
