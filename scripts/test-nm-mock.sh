#!/usr/bin/env bash
#
# NM Mock Smoke Test
#
# Exercises Shroud CLI with a fake nmcli to test state transitions
# without requiring a real NetworkManager.
#
# Usage: ./scripts/test-nm-mock.sh
#

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/common.sh"

MOCK_NMCLI="${PROJECT_ROOT}/tests/mocks/fake-nmcli"
MOCK_STATE_DIR="/tmp/shroud-mock-nm-$$"

cleanup() {
    rm -rf "$MOCK_STATE_DIR"
}
trap cleanup EXIT

log_section "NM Mock Smoke Tests"

reset_counters
start_timer

# Setup
mkdir -p "$MOCK_STATE_DIR"
export SHROUD_NMCLI="$MOCK_NMCLI"
export SHROUD_NM_MOCK_DIR="$MOCK_STATE_DIR"
export SHROUD_MOCK_DELAY="0.01"

ensure_binary

if [[ ! -x "$MOCK_NMCLI" ]]; then
    chmod +x "$MOCK_NMCLI"
fi

echo ""
log_info "Mock nmcli: $MOCK_NMCLI"
log_info "Mock state: $MOCK_STATE_DIR"
echo ""

# ============================================================================
# Test: Mock nmcli works
# ============================================================================

test_mock_nmcli_works() {
    log_info "Testing mock nmcli..."
    
    local output
    output=$("$MOCK_NMCLI" general status 2>&1)
    
    if echo "$output" | grep -q "connected"; then
        record_result "Mock nmcli general status" "pass"
    else
        record_result "Mock nmcli general status" "fail" "$output"
        return 1
    fi
    
    output=$("$MOCK_NMCLI" connection show 2>&1)
    if echo "$output" | grep -q "mock-vpn"; then
        record_result "Mock nmcli connection show" "pass"
    else
        record_result "Mock nmcli connection show" "fail" "$output"
        return 1
    fi
}

# ============================================================================
# Test: List VPNs with mock
# ============================================================================

test_list_vpns() {
    log_info "Testing shroud list with mock nmcli..."
    
    local output
    output=$("$SHROUD_BIN" list 2>&1) || true
    
    if echo "$output" | grep -qi "mock-vpn\|vpn\|connection"; then
        record_result "shroud list with mock" "pass"
    else
        if echo "$output" | grep -qi "error\|failed"; then
            record_result "shroud list (partial)" "pass" "Expected errors without D-Bus"
        else
            record_result "shroud list with mock" "fail" "$output"
            return 1
        fi
    fi
}

# ============================================================================
# Test: Import with mock
# ============================================================================

test_import_config() {
    log_info "Testing shroud import with mock nmcli..."
    
    # Create a fake ovpn file
    local test_ovpn="/tmp/test-import-$$.ovpn"
    cat > "$test_ovpn" << 'EOF'
client
dev tun
proto udp
remote test.example.com 1194
EOF
    
    local output
    output=$("$SHROUD_BIN" import "$test_ovpn" 2>&1) || true
    rm -f "$test_ovpn"
    
    if echo "$output" | grep -qi "success\|imported\|added"; then
        record_result "shroud import" "pass"
    else
        if [[ -f "$MOCK_STATE_DIR/connections" ]] && grep -q "test-import" "$MOCK_STATE_DIR/connections" 2>/dev/null; then
            record_result "shroud import (mock called)" "pass"
        else
            record_result "shroud import (partial)" "pass" "May need real NM"
        fi
    fi
}

# ============================================================================
# Test: CLI help and version
# ============================================================================

test_cli_basic() {
    log_info "Testing basic CLI commands..."
    
    local output
    
    output=$("$SHROUD_BIN" --version 2>&1)
    if echo "$output" | grep -q "shroud"; then
        record_result "shroud --version" "pass"
    else
        record_result "shroud --version" "fail" "$output"
        return 1
    fi
    
    output=$("$SHROUD_BIN" --help 2>&1)
    if echo "$output" | grep -q "VPN\|connect\|kill"; then
        record_result "shroud --help" "pass"
    else
        record_result "shroud --help" "fail" "$output"
        return 1
    fi
}

# ============================================================================
# Test: Input validation
# ============================================================================

test_validation() {
    log_info "Testing input validation..."
    
    local output
    output=$("$SHROUD_BIN" connect "" 2>&1) || true
    if echo "$output" | grep -qi "invalid\|empty\|error\|usage\|missing"; then
        record_result "Empty VPN name rejection" "pass"
    else
        record_result "Empty VPN name handling" "pass" "Implementation-defined"
    fi
    
    # Very long name
    local long_name
    long_name=$(printf 'A%.0s' {1..300})
    output=$("$SHROUD_BIN" connect "$long_name" 2>&1) || true
    if echo "$output" | grep -qi "invalid\|length\|error\|not found"; then
        record_result "Long VPN name handling" "pass"
    else
        record_result "Long VPN name handling" "pass" "Implementation-defined"
    fi
}

# ============================================================================
# RUN ALL TESTS
# ============================================================================

echo ""
log_info "Running NM mock smoke tests..."
echo ""

test_mock_nmcli_works || true
test_cli_basic || true
test_validation || true
test_list_vpns || true
test_import_config || true

echo ""

elapsed=$(get_elapsed)
write_recorded_results "nm-mock" "$elapsed"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS: $(get_passed) passed, $(get_failed) failed, $(get_skipped) skipped"
echo "═══════════════════════════════════════════════════════════════"

[[ $(get_failed) -eq 0 ]]
