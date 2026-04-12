# VPNShroud Test Suite

Every test, organized by type and purpose.

## Test Categories

| Category | Location | Command | CI Stage | Description |
|----------|----------|---------|----------|-------------|
| Unit | `src/**/*.rs` | `cargo test --bins` | Stage 2 | Inline tests for individual functions |
| Integration | `tests/integration/` | `cargo test --test integration` | Stage 3 | Module interaction tests |
| Security | `tests/security/` | `cargo test --test security` | Stage 3 | Security verification tests |
| E2E Desktop | `tests/e2e/desktop/` | `./tests/e2e/desktop/run-desktop-tests.sh` | Stage 4 | Full desktop mode tests |
| E2E Headless | `tests/e2e/headless/` | `./tests/e2e/headless/run-tests.sh` | Stage 4 | Full headless mode tests |

## Directory Structure

```
tests/
├── README.md                  # This file
│
├── integration/               # Integration tests (Rust)
│   ├── mod.rs                 # Test module root
│   ├── common/                # Shared test utilities
│   │   ├── mod.rs
│   │   └── fixtures.rs        # Test fixtures and builders
│   ├── config_tests.rs        # Configuration parsing tests
│   ├── state_machine_tests.rs # State machine transition tests
│   ├── tray_channel_tests.rs  # Tray ↔ supervisor channel tests
│   ├── cli_tests.rs           # CLI integration tests
│   ├── daemon_tests.rs        # Daemon lifecycle tests
│   ├── import_tests.rs        # VPN import tests
│   └── validation_tests.rs    # Input validation tests
│
├── security/                  # Security tests (Rust)
│   ├── mod.rs                 # Test module root
│   ├── common/                # Shared security test utilities
│   │   └── mod.rs
│   ├── signal_tests.rs        # Signal handling tests
│   ├── privilege_tests.rs     # Privilege escalation checks
│   ├── resource_tests.rs      # Resource exhaustion tests
│   ├── crash_tests.rs         # Crash recovery tests
│   ├── race_tests.rs          # Race condition tests
│   ├── ipc_tests.rs           # IPC socket security tests
│   ├── dbus_tests.rs          # D-Bus security tests
│   ├── config_tests.rs        # Config file security tests
│   └── leak_tests.rs          # VPN leak prevention tests
│
└── e2e/                       # End-to-end tests (Shell scripts)
    ├── README.md              # E2E test documentation
    ├── run-all.sh             # Run all E2E tests
    ├── run-privileged.sh      # Run privileged tests
    ├── lib/
    │   └── test-helpers.sh    # Shared shell utilities
    ├── desktop/               # Desktop mode E2E tests
    │   ├── run-desktop-tests.sh
    │   ├── test-*.sh
    │   └── results/
    └── headless/              # Headless mode E2E tests
        ├── run-tests.sh
        ├── test-*.sh
        └── results/
```

## Running Tests Locally

### Quick Commands

```bash
# Run all tests (recommended before committing)
./scripts/test-all.sh

# Quick iteration (unit tests only)
./scripts/test-quick.sh

# Run specific test category
./scripts/test-unit.sh
./scripts/test-integration.sh
./scripts/test-e2e.sh
```

### Detailed Commands

```bash
# Unit tests
cargo test --bins                    # All unit tests
cargo test --bins mode               # Tests matching "mode"
cargo test --bins -- --nocapture     # Show println! output

# Integration tests
cargo test --test integration        # All integration tests
cargo test --test integration config # Config-related tests

# Security tests (non-privileged)
cargo test --test security           # Run non-privileged only

# Security tests (privileged - requires sudo)
sudo -E cargo test --test security -- --ignored

# E2E tests
./tests/e2e/run-all.sh               # Non-privileged only
sudo -E ./tests/e2e/run-all.sh --privileged  # All tests
```

## Test Execution Order (CI Pipeline)

```
┌──────────────────────────────────────────────────────────────────┐
│ STAGE 1: Quick Checks (parallel, ~2 min)                         │
├──────────────────────────────────────────────────────────────────┤
│  Format → Clippy → Compile Check → Documentation                 │
└────────────────────────────┬─────────────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────────────┐
│ STAGE 2: Unit Tests (~3 min)                                     │
├──────────────────────────────────────────────────────────────────┤
│  cargo test --bins + cargo test --doc                            │
└────────────────────────────┬─────────────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────────────┐
│ STAGE 3: Integration + Security Tests (~5 min)                   │
├──────────────────────────────────────────────────────────────────┤
│  Integration Tests ──┬── Security Tests (non-privileged)         │
└──────────────────────┴───────────────┬───────────────────────────┘
                                       │
                                       ▼
┌──────────────────────────────────────────────────────────────────┐
│ STAGE 4: E2E Tests (~10 min)                                     │
├──────────────────────────────────────────────────────────────────┤
│  Desktop E2E ──────────┬────────── Headless E2E                  │
└────────────────────────┴─────────────────────────────────────────┘
```

## Adding New Tests

### Unit Test

Add `#[test]` in the same file as the code being tested:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_function() {
        assert_eq!(my_function(), expected);
    }
}
```

### Integration Test

Add to `tests/integration/<module>_tests.rs`:

```rust
use crate::common::fixtures::*;

#[test]
fn test_module_interaction() {
    // Test multiple modules working together
}
```

### Security Test

Add to `tests/security/<area>_tests.rs`:

```rust
#[test]
#[ignore = "requires privileged environment"]
fn test_security_property() {
    // Security-sensitive test
}
```

### E2E Test

Create `tests/e2e/<mode>/test-<name>.sh`:

```bash
#!/usr/bin/env bash
source "$(dirname "$0")/../lib/test-helpers.sh"

test_my_feature() {
    # Test full binary behavior
}

run_tests
```

## Decision Tree: Where Should This Test Go?

```
Does it test a single function/struct in isolation?
├── YES → Unit test (inline in src/)
└── NO
    ↓
Does it test multiple modules working together?
├── YES → Integration test (tests/integration/)
└── NO
    ↓
Does it test security-sensitive behavior?
├── YES → Security test (tests/security/)
└── NO
    ↓
Does it need the full binary running?
├── YES → E2E test (tests/e2e/)
└── NO
    ↓
Does it need system resources (iptables, network)?
├── YES → E2E test with --privileged
└── NO → Probably integration test
```

## Test Requirements

### Integration Tests

- No daemon required (tests use internal APIs)
- No root privileges required
- No network access required

### Security Tests

Non-privileged tests:
- Socket permission verification
- Input validation
- Log rotation checks

Privileged tests (marked with `#[ignore]`):
- Root/sudo access for iptables
- NetworkManager running
- D-Bus session available

### E2E Tests

Non-privileged:
- Mode detection
- Gateway detection (parsing only)
- CLI help/version

Privileged:
- Kill switch enable/disable
- Gateway NAT rules
- Boot kill switch
- Cleanup verification

## Coverage

Generate coverage report:

```bash
./scripts/coverage.sh --html --open
```

Coverage is also generated weekly by the scheduled CI workflow.

## Troubleshooting

### Tests fail with "Failed to run shroud"

Build the project first:
```bash
cargo build --release
```

### Privileged tests fail with permission denied

Run with sudo:
```bash
sudo -E cargo test --test security -- --ignored
```

### E2E tests can't find lib.sh

Source paths are relative. Run from project root:
```bash
cd /path/to/shroud
./tests/e2e/run-all.sh
```
