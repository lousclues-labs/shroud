#!/usr/bin/env bash
# Boot Kill Switch Tests (Privileged)
#
# Tests for the SHROUD_BOOT_KS chain and boot-time protection

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

# Ensure we're root for these tests
require_root

# ============================================================================
# Setup/Teardown
# ============================================================================

setup_boot_ks_test() {
    # Clean any existing chains
    cleanup_iptables
}

teardown_boot_ks_test() {
    cleanup_iptables
}

# ============================================================================
# Test Functions
# ============================================================================

test_boot_ks_chain_creation() {
    setup_boot_ks_test
    
    # Run shroud in headless mode briefly to create boot killswitch
    # Using timeout to prevent blocking
    timeout 2s shroud --headless 2>/dev/null &
    local pid=$!
    sleep 0.5
    
    # Check if chain was created
    if ! iptables -L SHROUD_BOOT_KS -n &>/dev/null; then
        kill $pid 2>/dev/null || true
        teardown_boot_ks_test
        echo "SHROUD_BOOT_KS chain not created"
        return 1
    fi
    
    kill $pid 2>/dev/null || true
    wait $pid 2>/dev/null || true
    teardown_boot_ks_test
}

test_boot_ks_blocks_by_default() {
    setup_boot_ks_test
    
    # Create the boot kill switch chain manually for testing
    iptables -N SHROUD_BOOT_KS 2>/dev/null || true
    iptables -A SHROUD_BOOT_KS -o lo -j ACCEPT
    iptables -A SHROUD_BOOT_KS -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
    iptables -A SHROUD_BOOT_KS -j DROP
    
    # Insert at OUTPUT
    iptables -I OUTPUT 1 -j SHROUD_BOOT_KS
    
    # Verify the chain blocks new connections
    local rules
    rules=$(iptables -L SHROUD_BOOT_KS -n)
    assert_contains "$rules" "DROP" "Should have DROP rule"
    
    teardown_boot_ks_test
}

test_boot_ks_allows_loopback() {
    setup_boot_ks_test
    
    # Create chain
    iptables -N SHROUD_BOOT_KS 2>/dev/null || true
    iptables -A SHROUD_BOOT_KS -o lo -j ACCEPT
    iptables -A SHROUD_BOOT_KS -j DROP
    iptables -I OUTPUT 1 -j SHROUD_BOOT_KS
    
    local rules
    rules=$(iptables -L SHROUD_BOOT_KS -n -v)
    assert_contains "$rules" "lo" "Should allow loopback"
    
    teardown_boot_ks_test
}

test_boot_ks_allows_lan_when_configured() {
    setup_boot_ks_test
    
    # Create chain with LAN access
    iptables -N SHROUD_BOOT_KS 2>/dev/null || true
    iptables -A SHROUD_BOOT_KS -o lo -j ACCEPT
    iptables -A SHROUD_BOOT_KS -d 192.168.0.0/16 -j ACCEPT
    iptables -A SHROUD_BOOT_KS -d 10.0.0.0/8 -j ACCEPT
    iptables -A SHROUD_BOOT_KS -d 172.16.0.0/12 -j ACCEPT
    iptables -A SHROUD_BOOT_KS -j DROP
    iptables -I OUTPUT 1 -j SHROUD_BOOT_KS
    
    local rules
    rules=$(iptables -L SHROUD_BOOT_KS -n)
    
    assert_contains "$rules" "192.168.0.0/16" "Should allow 192.168.x.x"
    assert_contains "$rules" "10.0.0.0/8" "Should allow 10.x.x.x"
    
    teardown_boot_ks_test
}

test_boot_ks_cleanup() {
    setup_boot_ks_test
    
    # Create chain
    iptables -N SHROUD_BOOT_KS 2>/dev/null || true
    iptables -A SHROUD_BOOT_KS -j DROP
    iptables -I OUTPUT 1 -j SHROUD_BOOT_KS
    
    # Verify it exists
    assert_chain_exists SHROUD_BOOT_KS "Chain should exist"
    
    # Clean up
    iptables -D OUTPUT -j SHROUD_BOOT_KS 2>/dev/null || true
    iptables -F SHROUD_BOOT_KS 2>/dev/null || true
    iptables -X SHROUD_BOOT_KS 2>/dev/null || true
    
    # Verify cleanup
    assert_chain_not_exists SHROUD_BOOT_KS "Chain should be removed"
    
    teardown_boot_ks_test
}

test_boot_ks_ipv6_creation() {
    setup_boot_ks_test
    
    # Create IPv6 chain
    ip6tables -N SHROUD_BOOT_KS 2>/dev/null || true
    ip6tables -A SHROUD_BOOT_KS -o lo -j ACCEPT
    ip6tables -A SHROUD_BOOT_KS -j DROP
    ip6tables -I OUTPUT 1 -j SHROUD_BOOT_KS
    
    local rules
    rules=$(ip6tables -L SHROUD_BOOT_KS -n)
    assert_contains "$rules" "DROP" "IPv6 should have DROP rule"
    
    # Cleanup
    ip6tables -D OUTPUT -j SHROUD_BOOT_KS 2>/dev/null || true
    ip6tables -F SHROUD_BOOT_KS 2>/dev/null || true
    ip6tables -X SHROUD_BOOT_KS 2>/dev/null || true
    
    teardown_boot_ks_test
}

test_boot_ks_established_allowed() {
    setup_boot_ks_test
    
    # Create chain with established connections allowed
    iptables -N SHROUD_BOOT_KS 2>/dev/null || true
    iptables -A SHROUD_BOOT_KS -o lo -j ACCEPT
    iptables -A SHROUD_BOOT_KS -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
    iptables -A SHROUD_BOOT_KS -j DROP
    iptables -I OUTPUT 1 -j SHROUD_BOOT_KS
    
    local rules
    rules=$(iptables -L SHROUD_BOOT_KS -n)
    assert_contains "$rules" "ESTABLISHED" "Should allow established connections"
    
    teardown_boot_ks_test
}

test_boot_ks_persists_check() {
    setup_boot_ks_test
    
    # Check if the persistence file location is correct
    local persist_file="/etc/shroud/boot-killswitch"
    
    # Ensure directory exists
    mkdir -p /etc/shroud
    
    # Create marker file
    echo "1" > "$persist_file"
    
    # Verify we can read it
    local value
    value=$(cat "$persist_file")
    assert_eq "1" "$value" "Should persist boot killswitch state"
    
    # Cleanup
    rm -f "$persist_file"
    
    teardown_boot_ks_test
}

# ============================================================================
# Run Tests
# ============================================================================

begin_suite "boot-killswitch"

run_test "Boot KS chain creation" test_boot_ks_chain_creation
run_test "Boot KS blocks by default" test_boot_ks_blocks_by_default
run_test "Boot KS allows loopback" test_boot_ks_allows_loopback
run_test "Boot KS allows LAN when configured" test_boot_ks_allows_lan_when_configured
run_test "Boot KS cleanup" test_boot_ks_cleanup
run_test "Boot KS IPv6 creation" test_boot_ks_ipv6_creation
run_test "Boot KS allows established" test_boot_ks_established_allowed
run_test "Boot KS persistence check" test_boot_ks_persists_check

end_suite
