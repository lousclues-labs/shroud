#!/usr/bin/env bash
# Cleanup Tests (Privileged)
#
# Tests for proper cleanup of firewall rules and system state

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

require_root

# ============================================================================
# Setup
# ============================================================================

ORIGINAL_IP_FORWARD=""

setup_cleanup_test() {
    ORIGINAL_IP_FORWARD=$(get_ip_forward_state)
    cleanup_iptables
    cleanup_test_interfaces
}

teardown_cleanup_test() {
    echo "$ORIGINAL_IP_FORWARD" > /proc/sys/net/ipv4/ip_forward
    cleanup_iptables
    cleanup_test_interfaces
}

# ============================================================================
# Test Functions
# ============================================================================

test_cleanup_killswitch_chain() {
    setup_cleanup_test
    
    # Create killswitch chain
    iptables -N SHROUD_KILLSWITCH 2>/dev/null || true
    iptables -A SHROUD_KILLSWITCH -o lo -j ACCEPT
    iptables -A SHROUD_KILLSWITCH -j DROP
    iptables -I OUTPUT 1 -j SHROUD_KILLSWITCH
    
    # Verify exists
    assert_chain_exists SHROUD_KILLSWITCH "Chain should exist before cleanup"
    
    # Run cleanup
    iptables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
    iptables -F SHROUD_KILLSWITCH 2>/dev/null || true
    iptables -X SHROUD_KILLSWITCH 2>/dev/null || true
    
    # Verify removed
    assert_chain_not_exists SHROUD_KILLSWITCH "Chain should be removed after cleanup"
    
    teardown_cleanup_test
}

test_cleanup_boot_ks_chain() {
    setup_cleanup_test
    
    # Create boot killswitch chain
    iptables -N SHROUD_BOOT_KS 2>/dev/null || true
    iptables -A SHROUD_BOOT_KS -j DROP
    iptables -I OUTPUT 1 -j SHROUD_BOOT_KS
    
    assert_chain_exists SHROUD_BOOT_KS "Chain should exist before cleanup"
    
    # Cleanup
    iptables -D OUTPUT -j SHROUD_BOOT_KS 2>/dev/null || true
    iptables -F SHROUD_BOOT_KS 2>/dev/null || true
    iptables -X SHROUD_BOOT_KS 2>/dev/null || true
    
    assert_chain_not_exists SHROUD_BOOT_KS "Chain should be removed after cleanup"
    
    teardown_cleanup_test
}

test_cleanup_gateway_chain() {
    setup_cleanup_test
    
    # Create gateway chain
    iptables -N SHROUD_GATEWAY 2>/dev/null || true
    iptables -A SHROUD_GATEWAY -j ACCEPT
    iptables -I FORWARD 1 -j SHROUD_GATEWAY
    
    assert_chain_exists SHROUD_GATEWAY "Chain should exist before cleanup"
    
    # Cleanup
    iptables -D FORWARD -j SHROUD_GATEWAY 2>/dev/null || true
    iptables -F SHROUD_GATEWAY 2>/dev/null || true
    iptables -X SHROUD_GATEWAY 2>/dev/null || true
    
    assert_chain_not_exists SHROUD_GATEWAY "Chain should be removed after cleanup"
    
    teardown_cleanup_test
}

test_cleanup_gateway_ks_chain() {
    setup_cleanup_test
    
    # Create gateway killswitch chain
    iptables -N SHROUD_GATEWAY_KS 2>/dev/null || true
    iptables -A SHROUD_GATEWAY_KS -j DROP
    iptables -I FORWARD 1 -j SHROUD_GATEWAY_KS
    
    assert_chain_exists SHROUD_GATEWAY_KS "Chain should exist before cleanup"
    
    # Cleanup
    iptables -D FORWARD -j SHROUD_GATEWAY_KS 2>/dev/null || true
    iptables -F SHROUD_GATEWAY_KS 2>/dev/null || true
    iptables -X SHROUD_GATEWAY_KS 2>/dev/null || true
    
    assert_chain_not_exists SHROUD_GATEWAY_KS "Chain should be removed after cleanup"
    
    teardown_cleanup_test
}

test_cleanup_nat_rules() {
    setup_cleanup_test
    
    # Create dummy interface
    ip link add tun-test type dummy 2>/dev/null || true
    ip link set tun-test up
    
    # Add NAT rule
    iptables -t nat -A POSTROUTING -o tun-test -j MASQUERADE
    
    # Verify exists
    local rules
    rules=$(iptables -t nat -L POSTROUTING -n -v)
    assert_contains "$rules" "MASQUERADE" "NAT rule should exist"
    
    # Cleanup
    iptables -t nat -D POSTROUTING -o tun-test -j MASQUERADE 2>/dev/null || true
    ip link del tun-test 2>/dev/null || true
    
    # Verify removed
    rules=$(iptables -t nat -L POSTROUTING -n -v)
    assert_not_contains "$rules" "tun-test" "NAT rule should be removed"
    
    teardown_cleanup_test
}

test_cleanup_ip_forwarding() {
    setup_cleanup_test
    
    # Enable forwarding
    echo 1 > /proc/sys/net/ipv4/ip_forward
    assert_eq "1" "$(get_ip_forward_state)" "Should be enabled"
    
    # Disable (cleanup)
    echo 0 > /proc/sys/net/ipv4/ip_forward
    assert_eq "0" "$(get_ip_forward_state)" "Should be disabled after cleanup"
    
    teardown_cleanup_test
}

test_cleanup_ipv6_chains() {
    setup_cleanup_test
    
    # Create IPv6 chains
    ip6tables -N SHROUD_KILLSWITCH 2>/dev/null || true
    ip6tables -A SHROUD_KILLSWITCH -j DROP
    ip6tables -I OUTPUT 1 -j SHROUD_KILLSWITCH
    
    # Verify exists
    if ! ip6tables -L SHROUD_KILLSWITCH -n &>/dev/null; then
        teardown_cleanup_test
        echo "IPv6 chain not created"
        return 1
    fi
    
    # Cleanup
    ip6tables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
    ip6tables -F SHROUD_KILLSWITCH 2>/dev/null || true
    ip6tables -X SHROUD_KILLSWITCH 2>/dev/null || true
    
    # Verify removed
    if ip6tables -L SHROUD_KILLSWITCH -n &>/dev/null; then
        teardown_cleanup_test
        echo "IPv6 chain not removed"
        return 1
    fi
    
    teardown_cleanup_test
}

test_cleanup_all_chains() {
    setup_cleanup_test
    
    # Create all chains
    for chain in SHROUD_KILLSWITCH SHROUD_BOOT_KS SHROUD_GATEWAY SHROUD_GATEWAY_KS; do
        iptables -N "$chain" 2>/dev/null || true
        iptables -A "$chain" -j ACCEPT
    done
    
    # Add to appropriate hooks
    iptables -I OUTPUT 1 -j SHROUD_KILLSWITCH 2>/dev/null || true
    iptables -I OUTPUT 1 -j SHROUD_BOOT_KS 2>/dev/null || true
    iptables -I FORWARD 1 -j SHROUD_GATEWAY 2>/dev/null || true
    iptables -I FORWARD 1 -j SHROUD_GATEWAY_KS 2>/dev/null || true
    
    # Run full cleanup
    cleanup_iptables
    
    # Verify all removed
    for chain in SHROUD_KILLSWITCH SHROUD_BOOT_KS SHROUD_GATEWAY SHROUD_GATEWAY_KS; do
        assert_chain_not_exists "$chain" "Chain $chain should be removed"
    done
    
    teardown_cleanup_test
}

test_cleanup_socket_file() {
    setup_cleanup_test
    
    local socket_path="${XDG_RUNTIME_DIR:-/tmp}/shroud-cleanup-test.sock"
    
    # Create fake socket
    touch "$socket_path"
    
    # Verify exists
    assert_file_exists "$socket_path" "Socket file should exist"
    
    # Cleanup
    rm -f "$socket_path"
    
    # Verify removed
    if [[ -e "$socket_path" ]]; then
        teardown_cleanup_test
        echo "Socket file not removed"
        return 1
    fi
    
    teardown_cleanup_test
}

test_cleanup_test_interfaces() {
    setup_cleanup_test
    
    # Create test interfaces
    ip link add tun-test type dummy 2>/dev/null || true
    ip link add wg-test type dummy 2>/dev/null || true
    
    # Verify exist
    if ! ip link show tun-test &>/dev/null; then
        teardown_cleanup_test
        echo "tun-test not created"
        return 1
    fi
    
    # Cleanup
    cleanup_test_interfaces
    
    # Verify removed
    if ip link show tun-test &>/dev/null; then
        teardown_cleanup_test
        echo "tun-test not removed"
        return 1
    fi
    
    teardown_cleanup_test
}

test_cleanup_idempotent() {
    setup_cleanup_test
    
    # Run cleanup multiple times - should not error
    cleanup_iptables
    cleanup_iptables
    cleanup_iptables
    
    # No assertion needed - test passes if no error
    
    teardown_cleanup_test
}

test_cleanup_partial_state() {
    setup_cleanup_test
    
    # Create chain but don't add to OUTPUT/FORWARD
    iptables -N SHROUD_KILLSWITCH 2>/dev/null || true
    iptables -A SHROUD_KILLSWITCH -j DROP
    # Intentionally NOT adding: iptables -I OUTPUT -j SHROUD_KILLSWITCH
    
    # Cleanup should handle this gracefully
    cleanup_iptables
    
    assert_chain_not_exists SHROUD_KILLSWITCH "Orphan chain should be removed"
    
    teardown_cleanup_test
}

# ============================================================================
# Run Tests
# ============================================================================

begin_suite "cleanup"

run_test "Cleanup killswitch chain" test_cleanup_killswitch_chain
run_test "Cleanup boot KS chain" test_cleanup_boot_ks_chain
run_test "Cleanup gateway chain" test_cleanup_gateway_chain
run_test "Cleanup gateway KS chain" test_cleanup_gateway_ks_chain
run_test "Cleanup NAT rules" test_cleanup_nat_rules
run_test "Cleanup IP forwarding" test_cleanup_ip_forwarding
run_test "Cleanup IPv6 chains" test_cleanup_ipv6_chains
run_test "Cleanup all chains" test_cleanup_all_chains
run_test "Cleanup socket file" test_cleanup_socket_file
run_test "Cleanup test interfaces" test_cleanup_test_interfaces
run_test "Cleanup is idempotent" test_cleanup_idempotent
run_test "Cleanup partial state" test_cleanup_partial_state

end_suite
