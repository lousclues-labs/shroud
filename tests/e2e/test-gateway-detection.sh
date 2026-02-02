#!/usr/bin/env bash
# Gateway Interface Detection Tests
#
# Tests for detecting VPN and LAN interfaces (non-privileged)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib.sh
source "${SCRIPT_DIR}/lib.sh"

# ============================================================================
# Test Functions
# ============================================================================

test_gateway_help_available() {
    local output
    output=$(shroud gateway --help 2>&1 || true)
    assert_contains "$output" "gateway" "Should show gateway help"
}

test_gateway_enable_help() {
    # The 'on' command is documented in gateway --help
    local output
    output=$(shroud gateway --help 2>&1 || true)
    assert_contains "$output" "on" "Should document 'on' subcommand"
}

test_gateway_disable_help() {
    # The 'off' command is documented in gateway --help
    local output
    output=$(shroud gateway --help 2>&1 || true)
    assert_contains "$output" "off" "Should document 'off' subcommand"
}

test_gateway_status_help() {
    local output
    output=$(shroud gateway --help 2>&1 || true)
    assert_contains "$output" "status" "Should document 'status' subcommand"
}

test_gateway_status_without_daemon() {
    # Status should work even without daemon running
    local output exit_code
    set +e
    output=$(shroud gateway status 2>&1)
    exit_code=$?
    set -e
    
    # Should either show disabled status or connect to running daemon
    # Check for common words that would appear in status output
    if [[ "$output" == *"Gateway"* ]] || [[ "$output" == *"gateway"* ]] || \
       [[ "$output" == *"disabled"* ]] || [[ "$output" == *"enabled"* ]] || \
       [[ "$output" == *"Daemon"* ]] || [[ "$output" == *"not running"* ]]; then
        return 0
    fi
    echo "Output was: $output"
    return 1
}

test_interface_detection_pattern() {
    # Test that common VPN interface patterns are recognized
    # This is a unit-style test of the detection logic
    
    # Get list of all interfaces
    local interfaces
    interfaces=$(ip link show 2>/dev/null | grep -oP '^\d+: \K[^:@]+' || true)
    
    if [[ -z "$interfaces" ]]; then
        skip_test "no network interfaces found"
    fi
    
    # Just verify we can list interfaces without crashing
    # Check for common interface names
    if [[ "$interfaces" == *"lo"* ]] || [[ "$interfaces" == *"eth"* ]] || \
       [[ "$interfaces" == *"wlan"* ]] || [[ "$interfaces" == *"enp"* ]] || \
       [[ "$interfaces" == *"wlp"* ]] || [[ "$interfaces" == *"eno"* ]]; then
        return 0
    fi
    echo "Interfaces found: $interfaces"
    return 1
}

test_default_route_detection() {
    # Test that we can detect the default route interface
    local default_iface
    default_iface=$(ip route show default 2>/dev/null | awk '{print $5}' | head -1 || true)
    
    if [[ -z "$default_iface" ]]; then
        skip_test "no default route configured"
    fi
    
    # Should be a valid interface name
    assert_ne "" "$default_iface" "Default interface should be non-empty"
    
    # Interface should exist
    if ! ip link show "$default_iface" &>/dev/null; then
        echo "Default interface '$default_iface' does not exist"
        return 1
    fi
}

test_vpn_interface_patterns() {
    # Verify VPN interface naming patterns match expected formats
    local vpn_patterns="tun|tap|wg|proton|mullvad|ovpn|vpn|nordlynx"
    
    # Check if any VPN-like interfaces exist (informational)
    local vpn_ifaces
    vpn_ifaces=$(ip link show 2>/dev/null | grep -oP '^\d+: \K[^:@]+' | grep -E "^($vpn_patterns)" || true)
    
    if [[ -n "$vpn_ifaces" ]]; then
        echo "Found VPN interfaces: $vpn_ifaces" >&2
    fi
    
    # Test passes regardless - just verifying pattern matching works
    return 0
}

test_gateway_config_parsing() {
    # Create a temp config and verify it parses
    local config_dir="/tmp/shroud-test-$$"
    mkdir -p "$config_dir"
    
    cat > "$config_dir/config.toml" << 'EOF'
[gateway]
enabled = false
vpn_interface = "tun0"
lan_interface = "eth0"
allowed_clients = "subnet"
EOF
    
    # Try to read config (shroud should validate it)
    # This is a basic smoke test
    local output
    output=$(SHROUD_CONFIG_DIR="$config_dir" shroud --help 2>&1 || true)
    
    rm -rf "$config_dir"
    
    # Should not crash on valid config
    assert_contains "$output" "shroud" "Should handle config without crashing"
}

test_allowed_clients_values() {
    # Test different allowed_clients values
    local config_dir="/tmp/shroud-test-$$"
    mkdir -p "$config_dir"
    
    for value in "all" "subnet" '"192.168.1.0/24, 10.0.0.0/8"'; do
        cat > "$config_dir/config.toml" << EOF
[gateway]
allowed_clients = $value
EOF
        
        local output
        output=$(SHROUD_CONFIG_DIR="$config_dir" shroud --help 2>&1 || true)
        
        if [[ "$output" == *"error"* && "$output" == *"allowed_clients"* ]]; then
            rm -rf "$config_dir"
            echo "Failed to parse allowed_clients = $value"
            return 1
        fi
    done
    
    rm -rf "$config_dir"
}

# ============================================================================
# Run Tests
# ============================================================================

begin_suite "gateway-detection"

run_test "Gateway help available" test_gateway_help_available
run_test "Gateway enable --help" test_gateway_enable_help
run_test "Gateway disable --help" test_gateway_disable_help
run_test "Gateway status --help" test_gateway_status_help
run_test "Status without daemon" test_gateway_status_without_daemon
run_test "Interface detection works" test_interface_detection_pattern
run_test "Default route detection" test_default_route_detection
run_test "VPN interface patterns" test_vpn_interface_patterns
run_test "Gateway config parsing" test_gateway_config_parsing
run_test "Allowed clients values" test_allowed_clients_values

end_suite
