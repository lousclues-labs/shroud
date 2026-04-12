# Testing Strategy

How we test VPNShroud, and how you can run them locally.

## Test Categories

### Unit Tests (~130 tests)
Location: `src/**/tests.rs` modules

Unit tests verify individual functions and modules in isolation. They use mock
infrastructure to avoid external dependencies.

```bash
./scripts/test.sh unit
# or directly:
cargo test --bins --all-features
```

### Integration Tests (~20 tests)
Location: `tests/integration/`

Integration tests verify component interactions using mock infrastructure:
- `mock_nm.rs` - Mock NetworkManager client
- `mock_executor.rs` - Mock command executor (no subprocess spawning)
- `mock_dbus.rs` - Mock D-Bus client

```bash
./scripts/test.sh integration
# or directly:
cargo test --test integration --all-features
```

### Security Tests
Location: `tests/security/`

Security tests verify permission checks, path validation, and security boundaries.
Some tests require root privileges.

```bash
# Non-privileged tests
./scripts/test.sh security

# Privileged tests (requires sudo)
sudo -E cargo test --test security -- --ignored
```

### Regression Tests
Location: `tests/regression.rs`

Regression tests prevent reintroduction of fixed bugs.

```bash
./scripts/test.sh regression
```

## Running All Tests

```bash
# Run all tests
./scripts/test.sh all

# Or directly
cargo test --all-features -- --test-threads=4
```

## Coverage

Generate coverage reports using tarpaulin:

```bash
./scripts/test.sh coverage
# Report: coverage/tarpaulin-report.html
```

## CI Pipeline

The CI pipeline runs:
1. **check** - Format, clippy, docs
2. **test** - All tests with `--all-features`
3. **coverage** - Upload to Codecov
4. **msrv** - Verify minimum supported Rust version

## Mock Infrastructure

All tests use mock infrastructure instead of spawning real processes:

| Mock | Purpose |
|------|---------|
| `MockNetworkManager` | Simulates NM D-Bus responses |
| `MockCommandExecutor` | Captures commands without execution |
| `MockDbusClient` | Simulates session bus monitoring |

This approach ensures:
- Tests run fast (<15s total)
- No external dependencies
- Deterministic behavior
- Full coverage instrumentation

## Manual Testing

For features that require real system interaction:

### VPN Connection
```bash
# Import a config
shroud import /path/to/config.ovpn

# Connect
shroud connect my-vpn

# Check status
shroud status
```

### Kill Switch
```bash
# Enable (requires sudo via PolicyKit)
shroud killswitch enable

# Check status
shroud killswitch status

# Disable
shroud killswitch disable
```

## Why No E2E Tests?

We used to have end-to-end tests that spawned the actual binary. We removed them because:

1. **CI Reliability** - Process lifecycle is fragile across CI environments
2. **No Coverage Value** - Subprocess instrumentation doesn't work with tarpaulin
3. **Redundancy** - Integration tests with mocks cover the same code paths
4. **Maintenance Burden** - E2E infrastructure required constant debugging

The integration test suite with comprehensive mocks provides equivalent coverage
with better reliability and performance.
