# Shroud E2E Tests

End-to-end tests for Shroud's headless mode and gateway functionality.

## Overview

These tests verify the complete functionality of:
- **Mode Detection**: `--headless` and `--desktop` flag handling
- **Headless Runtime**: Daemon startup, IPC, boot killswitch
- **Gateway Mode**: IP forwarding, NAT, FORWARD chain rules
- **Cleanup**: Proper removal of firewall rules and state

## Test Suites

| Suite | Description | Requires Root |
|-------|-------------|---------------|
| `test-mode-detection.sh` | CLI flag recognition and mode detection | No |
| `test-gateway-detection.sh` | Interface detection and config parsing | No |
| `test-boot-killswitch.sh` | Boot-time killswitch chain creation | Yes |
| `test-headless-runtime.sh` | Headless daemon operation | Yes |
| `test-gateway.sh` | Gateway NAT and firewall rules | Yes |
| `test-cleanup.sh` | Firewall rule cleanup | Yes |

## Running Tests

### All Tests (Non-Privileged Only)

```bash
./tests/e2e/run-all.sh
```

### All Tests (Including Privileged)

```bash
sudo ./tests/e2e/run-all.sh --privileged
```

### Privileged Tests Only

```bash
sudo ./tests/e2e/run-privileged.sh
```

### Specific Suite

```bash
./tests/e2e/run-all.sh --suite mode-detection
sudo ./tests/e2e/run-all.sh --privileged --suite gateway
```

### Individual Test File

```bash
./tests/e2e/test-mode-detection.sh
sudo ./tests/e2e/test-gateway.sh
```

## Options

| Option | Description |
|--------|-------------|
| `--privileged` | Run tests that require root (iptables, etc.) |
| `--quick` | Skip slow tests |
| `--verbose` | Show detailed output |
| `--suite NAME` | Run only the specified test suite |

## Test Output

Results are written to `tests/e2e/results/` as JSON files:

```json
{
  "suite": "gateway",
  "passed": 15,
  "failed": 0,
  "skipped": 1,
  "tests": [...],
  "timestamp": "2025-01-15T12:34:56Z"
}
```

## Writing New Tests

### Test Function Pattern

```bash
test_my_feature() {
    # Setup
    local result
    
    # Execute
    result=$(shroud some-command 2>&1)
    
    # Assert
    assert_contains "$result" "expected" "Should contain expected output"
}
```

### Available Assertions

| Assertion | Description |
|-----------|-------------|
| `assert_eq EXPECTED ACTUAL MSG` | Values are equal |
| `assert_ne NOT_EXPECTED ACTUAL MSG` | Values are not equal |
| `assert_contains HAYSTACK NEEDLE MSG` | String contains substring |
| `assert_not_contains HAYSTACK NEEDLE MSG` | String doesn't contain |
| `assert_success EXIT_CODE MSG` | Exit code is 0 |
| `assert_failure EXIT_CODE MSG` | Exit code is non-zero |
| `assert_file_exists PATH MSG` | File exists |
| `assert_chain_exists CHAIN MSG` | iptables chain exists |
| `assert_chain_not_exists CHAIN MSG` | iptables chain doesn't exist |

### Skip Tests

```bash
test_requires_feature() {
    require_root  # Skips if not root
    require_command "some-tool"  # Skips if command not found
    
    # Or manually:
    skip_test "reason for skipping"
}
```

### Register Test

```bash
begin_suite "my-suite"

run_test "Test description" test_my_feature
run_test "Another test" test_another_feature

end_suite
```

## CI Integration

These tests run in GitHub Actions via `.github/workflows/e2e-headless.yml`.

The workflow:
1. Runs non-privileged tests without sudo
2. Runs privileged tests with sudo and mock VPN interfaces
3. Uploads test results as artifacts

## Troubleshooting

### "Permission denied" errors

Run with `sudo` for privileged tests.

### Tests leave firewall rules

Run cleanup manually:
```bash
sudo ./tests/e2e/test-cleanup.sh
# Or manually:
sudo iptables -F SHROUD_KILLSWITCH && sudo iptables -X SHROUD_KILLSWITCH
```

### Daemon won't start

Check for existing socket:
```bash
rm -f "${XDG_RUNTIME_DIR}/shroud.sock"
```

### Tests hang

The tests use timeouts. If hanging, check for zombie shroud processes:
```bash
pkill -9 -f "shroud.*--headless"
```
