# Chaos Engineering Tests for Shroud

This directory contains chaos engineering tests that systematically test Shroud's
resilience against various failure modes.

## Philosophy

> Murphy's Law as a Service: Anything that CAN go wrong WILL go wrong.

These tests don't try to exploit security vulnerabilities. Instead, they simulate
hostile environments: network failures, resource exhaustion, concurrent access,
crash recovery, etc.

## Running the Tests

```bash
# Run all safe tests
./run-chaos.sh

# Run all tests including destructive ones (VM recommended)
./run-chaos.sh --all

# Run a specific test
./run-chaos.sh --test config_corrupted
```

## Test Categories

### 1. Configuration Chaos
- **config_corrupted**: Corrupt config file with garbage data
- **config_unwritable**: Make config directory unwritable

### 2. IPC Chaos
- **stale_socket**: Stale socket file from crashed instance
- **ipc_flood**: 100 concurrent IPC requests
- **ipc_malformed**: Garbage data sent to IPC socket
- **ipc_disconnect_mid_request**: Clients disconnecting mid-request
- **socket_deleted_while_running**: IPC socket deleted during operation

### 3. Signal Chaos
- **signal_storm**: Rapid SIGUSR1/SIGHUP signals
- **sigstop_sigcont**: Pause and resume with SIGSTOP/SIGCONT

### 4. Kill Switch Chaos
- **rapid_ks_toggle**: Rapid enable/disable cycling

### 5. State Machine Chaos
- **concurrent_commands**: Multiple simultaneous commands
- **rapid_state_transitions**: Rapid connect/disconnect cycling

### 6. Crash Recovery
- **kill9_recovery**: SIGKILL with kill switch enabled
- **multiple_instances**: Multiple daemon instances

### 7. Resource Exhaustion
- **low_fd_limit**: Very low file descriptor limit

## Results

Results are written to `results/chaos-results.json`:

```json
{
    "suite": "chaos",
    "passed": 15,
    "failed": 0,
    "skipped": 1,
    "timestamp": "2026-02-02T12:00:00+00:00"
}
```

## Adding New Tests

1. Create a function `test_your_test_name()`
2. Use `should_run_test "your_test_name" || return 0` at the start
3. Use `log_chaos`, `log_pass`, `log_fail`, `log_skip` for output
4. Add the test to an appropriate category section

## Severity Ratings

| Severity | Description | Action |
|----------|-------------|--------|
| Critical | User lockout, data loss | Must fix before release |
| High | Feature completely broken | Should fix before release |
| Medium | Degraded but functional | Fix in next release |
| Low | Cosmetic/minor | Backlog |

## Known Failure Modes

After running chaos tests, document findings in `docs/RESILIENCE.md`.
