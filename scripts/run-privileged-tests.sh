#!/bin/bash
# Run privileged integration tests
# Must be run with sudo or as root

set -e

echo "Running privileged integration tests..."
echo "These tests require root access for iptables manipulation."
echo ""

# Ensure cargo is available
export PATH="$HOME/.cargo/bin:$PATH"

# Run ignored tests with sudo
sudo -E env "PATH=$PATH" cargo test --all-features -- --ignored --test-threads=1

echo ""
echo "All privileged tests passed!"
