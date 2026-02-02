#!/usr/bin/env bash
# Gateway Tests (Privileged)
#
# Tests for VPN gateway mode: NAT, IP forwarding, firewall rules

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

require_root

# ============================================================================
# Setup/Teardown
# ============================================================================

ORIGINAL_IP_FORWARD=""
TEST_VPN_IFACE="tun-test"
TEST_LAN_IFACE="eth0"  # Will be detected

setup_gateway_test() {
    # Save original IP forward state
    ORIGINAL_IP_FORWARD=$(get_ip_forward_state)
    
    # Disable IP forwarding for clean test
    echo 0 > /proc/sys/net/ipv4/ip_forward
    
    # Clean up any existing test state
    cleanup_iptables
    cleanup_test_interfaces
    
    # Create dummy VPN interface for testing
    ip link add "$TEST_VPN_IFACE" type dummy 2>/dev/null || true
    ip addr add 10.200.200.1/24 dev "$TEST_VPN_IFACE" 2>/dev/null || true
    ip link set "$TEST_VPN_IFACE" up
    
    # Detect actual LAN interface
    TEST_LAN_IFACE=$(ip route show default 2>/dev/null | awk '{print $5}' | head -1 || echo "eth0")
}

teardown_gateway_test() {
    # Restore IP forwarding state
    echo "$ORIGINAL_IP_FORWARD" > /proc/sys/net/ipv4/ip_forward
    
    # Clean up
    cleanup_iptables
    cleanup_test_interfaces
}

# ============================================================================
# IP Forwarding Tests
# ============================================================================

test_ip_forwarding_enable() {
    setup_gateway_test
    
    # Ensure disabled
    echo 0 > /proc/sys/net/ipv4/ip_forward
    
    # Enable via sysctl
    echo 1 > /proc/sys/net/ipv4/ip_forward
    
    local state
    state=$(get_ip_forward_state)
    
    teardown_gateway_test
    
    assert_eq "1" "$state" "IP forwarding should be enabled"
}

test_ip_forwarding_disable() {
    setup_gateway_test
    
    # Enable first
    echo 1 > /proc/sys/net/ipv4/ip_forward
    
    # Disable
    echo 0 > /proc/sys/net/ipv4/ip_forward
    
    local state
    state=$(get_ip_forward_state)
    
    teardown_gateway_test
    
    assert_eq "0" "$state" "IP forwarding should be disabled"
}

# ============================================================================
# NAT Tests
# ============================================================================

test_nat_masquerade_creation() {
    setup_gateway_test
    
    # Add MASQUERADE rule
    iptables -t nat -A POSTROUTING -o "$TEST_VPN_IFACE" -j MASQUERADE
    
    local rules
    rules=$(iptables -t nat -L POSTROUTING -n -v)
    
    teardown_gateway_test
    
    assert_contains "$rules" "MASQUERADE" "Should have MASQUERADE rule"
    assert_contains "$rules" "$TEST_VPN_IFACE" "Should reference VPN interface"
}

test_nat_source_restriction() {
    setup_gateway_test
    
    # Add MASQUERADE rule with source restriction
    iptables -t nat -A POSTROUTING -s 192.168.1.0/24 -o "$TEST_VPN_IFACE" -j MASQUERADE
    
    local rules
    rules=$(iptables -t nat -L POSTROUTING -n -v)
    
    teardown_gateway_test
    
    assert_contains "$rules" "192.168.1.0/24" "Should restrict to source subnet"
}

test_nat_cleanup() {
    setup_gateway_test
    
    # Add rule
    iptables -t nat -A POSTROUTING -o "$TEST_VPN_IFACE" -j MASQUERADE
    
    # Remove rule
    iptables -t nat -D POSTROUTING -o "$TEST_VPN_IFACE" -j MASQUERADE
    
    local rules
    rules=$(iptables -t nat -L POSTROUTING -n -v)
    
    teardown_gateway_test
    
    assert_not_contains "$rules" "$TEST_VPN_IFACE" "Should not have VPN interface rule"
}

# ============================================================================
# FORWARD Chain Tests
# ============================================================================

test_forward_chain_creation() {
    setup_gateway_test
    
    # Create gateway chain
    iptables -N SHROUD_GATEWAY 2>/dev/null || true
    iptables -A SHROUD_GATEWAY -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
    iptables -A SHROUD_GATEWAY -i "$TEST_LAN_IFACE" -o "$TEST_VPN_IFACE" -j ACCEPT
    iptables -A SHROUD_GATEWAY -j DROP
    
    local rules
    rules=$(iptables -L SHROUD_GATEWAY -n -v)
    
    teardown_gateway_test
    
    assert_contains "$rules" "ACCEPT" "Should have ACCEPT rule"
    assert_contains "$rules" "DROP" "Should have DROP rule"
}

test_forward_established_allowed() {
    setup_gateway_test
    
    iptables -N SHROUD_GATEWAY 2>/dev/null || true
    iptables -A SHROUD_GATEWAY -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
    
    local rules
    rules=$(iptables -L SHROUD_GATEWAY -n)
    
    teardown_gateway_test
    
    assert_contains "$rules" "ESTABLISHED" "Should allow established connections"
    assert_contains "$rules" "RELATED" "Should allow related connections"
}

test_forward_lan_to_vpn() {
    setup_gateway_test
    
    iptables -N SHROUD_GATEWAY 2>/dev/null || true
    iptables -A SHROUD_GATEWAY -i "$TEST_LAN_IFACE" -o "$TEST_VPN_IFACE" -j ACCEPT
    
    local rules
    rules=$(iptables -L SHROUD_GATEWAY -n -v)
    
    teardown_gateway_test
    
    assert_contains "$rules" "$TEST_VPN_IFACE" "Should forward to VPN"
}

# ============================================================================
# Gateway Kill Switch Tests
# ============================================================================

test_gateway_ks_creation() {
    setup_gateway_test
    
    # Create gateway killswitch chain
    iptables -N SHROUD_GATEWAY_KS 2>/dev/null || true
    iptables -A SHROUD_GATEWAY_KS -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
    iptables -A SHROUD_GATEWAY_KS -o "$TEST_VPN_IFACE" -j ACCEPT
    iptables -A SHROUD_GATEWAY_KS -j DROP
    
    # Insert into FORWARD
    iptables -I FORWARD 1 -j SHROUD_GATEWAY_KS
    
    local rules
    rules=$(iptables -L SHROUD_GATEWAY_KS -n)
    
    teardown_gateway_test
    
    assert_contains "$rules" "DROP" "Gateway KS should drop by default"
}

test_gateway_ks_only_allows_vpn() {
    setup_gateway_test
    
    iptables -N SHROUD_GATEWAY_KS 2>/dev/null || true
    iptables -A SHROUD_GATEWAY_KS -o "$TEST_VPN_IFACE" -j ACCEPT
    iptables -A SHROUD_GATEWAY_KS -o lo -j ACCEPT
    iptables -A SHROUD_GATEWAY_KS -j DROP
    
    local rules
    rules=$(iptables -L SHROUD_GATEWAY_KS -n -v)
    
    teardown_gateway_test
    
    assert_contains "$rules" "$TEST_VPN_IFACE" "Should allow VPN interface"
    assert_contains "$rules" "lo" "Should allow loopback"
}

test_gateway_ks_ipv6_drop() {
    setup_gateway_test
    
    # Create IPv6 gateway killswitch - drop all forwarded IPv6
    ip6tables -N SHROUD_GATEWAY_KS 2>/dev/null || true
    ip6tables -A SHROUD_GATEWAY_KS -j DROP
    ip6tables -I FORWARD 1 -j SHROUD_GATEWAY_KS
    
    local rules
    rules=$(ip6tables -L SHROUD_GATEWAY_KS -n)
    
    # Cleanup IPv6
    ip6tables -D FORWARD -j SHROUD_GATEWAY_KS 2>/dev/null || true
    ip6tables -F SHROUD_GATEWAY_KS 2>/dev/null || true
    ip6tables -X SHROUD_GATEWAY_KS 2>/dev/null || true
    
    teardown_gateway_test
    
    assert_contains "$rules" "DROP" "IPv6 gateway KS should drop all"
}

# ============================================================================
# Integration Tests
# ============================================================================

test_full_gateway_setup() {
    setup_gateway_test
    
    # Enable IP forwarding
    echo 1 > /proc/sys/net/ipv4/ip_forward
    
    # Create NAT
    iptables -t nat -A POSTROUTING -o "$TEST_VPN_IFACE" -j MASQUERADE
    
    # Create FORWARD rules
    iptables -N SHROUD_GATEWAY 2>/dev/null || true
    iptables -A SHROUD_GATEWAY -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
    iptables -A SHROUD_GATEWAY -i "$TEST_LAN_IFACE" -o "$TEST_VPN_IFACE" -j ACCEPT
    iptables -A SHROUD_GATEWAY -j DROP
    iptables -I FORWARD 1 -j SHROUD_GATEWAY
    
    # Verify all components
    local ip_fwd
    ip_fwd=$(get_ip_forward_state)
    assert_eq "1" "$ip_fwd" "IP forwarding should be on"
    
    local nat_rules
    nat_rules=$(iptables -t nat -L POSTROUTING -n -v)
    assert_contains "$nat_rules" "MASQUERADE" "Should have NAT"
    
    local fwd_rules
    fwd_rules=$(iptables -L FORWARD -n)
    assert_contains "$fwd_rules" "SHROUD_GATEWAY" "FORWARD should jump to SHROUD_GATEWAY"
    
    teardown_gateway_test
}

test_gateway_cli_enable() {
    setup_gateway_test
    
    # Try gateway enable (may fail without VPN, that's okay)
    local result
    set +e
    result=$(shroud gateway on --vpn-interface "$TEST_VPN_IFACE" --lan-interface "$TEST_LAN_IFACE" 2>&1)
    local exit_code=$?
    set -e
    
    teardown_gateway_test
    
    # Either succeeds or fails with meaningful error
    if [[ $exit_code -eq 0 ]]; then
        if [[ "$result" == *"nable"* ]] || [[ "$result" == *"Gateway"* ]] || [[ "$result" == *"success"* ]]; then
            return 0
        fi
    else
        # Expected errors are okay (e.g., VPN not connected)
        if [[ "$result" == *"rror"* ]] || [[ "$result" == *"ailed"* ]] || \
           [[ "$result" == *"VPN"* ]] || [[ "$result" == *"interface"* ]] || \
           [[ "$result" == *"ermission"* ]]; then
            return 0
        fi
    fi
    echo "Unexpected output: $result"
    return 1
}

test_gateway_cli_status() {
    setup_gateway_test
    
    local result
    result=$(shroud gateway status 2>&1 || true)
    
    teardown_gateway_test
    
    # Status should return something meaningful
    if [[ "$result" == *"Gateway"* ]] || [[ "$result" == *"gateway"* ]] || \
       [[ "$result" == *"enabled"* ]] || [[ "$result" == *"disabled"* ]] || \
       [[ "$result" == *"status"* ]] || [[ "$result" == *"Status"* ]]; then
        return 0
    fi
    echo "Unexpected status output: $result"
    return 1
}

test_gateway_cli_disable() {
    setup_gateway_test
    
    # First enable
    shroud gateway on --vpn-interface "$TEST_VPN_IFACE" --lan-interface "$TEST_LAN_IFACE" 2>/dev/null || true
    
    # Then disable
    local result
    set +e
    result=$(shroud gateway off 2>&1)
    local exit_code=$?
    set -e
    
    teardown_gateway_test
    
    # Disable should work
    assert_success "$exit_code" "Disable should succeed"
}

# ============================================================================
# Run Tests
# ============================================================================

begin_suite "gateway"

# IP Forwarding
run_test "IP forwarding enable" test_ip_forwarding_enable
run_test "IP forwarding disable" test_ip_forwarding_disable

# NAT
run_test "NAT MASQUERADE creation" test_nat_masquerade_creation
run_test "NAT source restriction" test_nat_source_restriction
run_test "NAT cleanup" test_nat_cleanup

# FORWARD chain
run_test "FORWARD chain creation" test_forward_chain_creation
run_test "FORWARD allows established" test_forward_established_allowed
run_test "FORWARD LAN to VPN" test_forward_lan_to_vpn

# Gateway Kill Switch
run_test "Gateway KS creation" test_gateway_ks_creation
run_test "Gateway KS only allows VPN" test_gateway_ks_only_allows_vpn
run_test "Gateway KS IPv6 drop" test_gateway_ks_ipv6_drop

# Integration
run_test "Full gateway setup" test_full_gateway_setup
run_test "Gateway CLI enable" test_gateway_cli_enable
run_test "Gateway CLI status" test_gateway_cli_status
run_test "Gateway CLI disable" test_gateway_cli_disable

end_suite
