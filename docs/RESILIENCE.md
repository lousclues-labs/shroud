# Shroud Resilience Engineering

This document describes Shroud's approach to resilience engineering, including
known failure modes, hardening patterns, and recovery procedures.

## Philosophy

> Murphy's Law as a Service: Anything that CAN go wrong WILL go wrong.

Shroud is designed to fail gracefully under adverse conditions. The kill switch
especially must never leave users locked out of their network.

## Hardening Patterns

### 1. Timeouts Everywhere

All external commands and connections have timeouts:

| Component | Timeout | Purpose |
|-----------|---------|---------|
| nmcli commands | 30s | Prevent hang on NM issues |
| D-Bus connection | 10s | Fail fast if D-Bus unavailable |
| sudo/iptables | 30s | Detect sudo prompt or kernel freeze |
| IPC responses | 60s | Generous for user interaction |
| Health checks | 5s | Quick detection of tunnel issues |
| Kill switch cleanup | 5s | Non-blocking shutdown |

### 2. Graceful Degradation

When components fail, Shroud continues operating in degraded mode:

- **D-Bus fails**: Falls back to polling NetworkManager
- **iptables fails**: Kill switch disabled, VPN still works
- **Config corrupt**: Uses defaults, logs warning
- **Tray fails**: Desktop mode degrades but daemon continues

### 3. State Reconciliation

Shroud periodically syncs internal state with external reality:

- `sync_state_from_nm()`: Queries NM for actual VPN state
- `sync_killswitch_state()`: Verifies iptables rules exist
- `is_actually_enabled()`: Checks rule presence, not just flag

### 4. Crash Recovery

On startup, Shroud cleans up from potential previous crashes:

- **Stale lock file**: Detected via flock, can be overridden
- **Stale iptables rules**: `cleanup_stale_on_startup()`
- **Stale socket file**: Removed and recreated
- **Panic hook**: Emergency cleanup before process exit

### 5. Non-Blocking Operations

Critical operations use non-blocking with timeout:

```rust
// Kill switch cleanup won't block shutdown
match tokio::time::timeout(CLEANUP_TIMEOUT, cleanup()).await {
    Ok(_) => info!("Cleanup successful"),
    Err(_) => warn!("Cleanup timed out, proceeding with shutdown"),
}
```

## Known Failure Modes

### Critical (User Lockout Risk)

| Failure | Cause | Mitigation |
|---------|-------|------------|
| Kill switch stuck ON | SIGKILL during iptables modification | Stale rule detection on startup |
| iptables hangs | Kernel module issue | 30s timeout on sudo commands |
| D-Bus hangs | dbus-daemon frozen | 10s connection timeout |

### High (Feature Broken)

| Failure | Cause | Mitigation |
|---------|-------|------------|
| NM connection drops | NetworkManager restart | Auto-detect and reconnect |
| State divergence | External VPN changes | Periodic state sync |
| IPC socket deleted | User error | Recreate on next CLI command |

### Medium (Degraded Operation)

| Failure | Cause | Mitigation |
|---------|-------|------------|
| D-Bus events stop | Connection lost | Fall back to polling |
| Config unwritable | Permissions | Continue with in-memory state |
| Health check fails | Network issue | Exponential backoff retry |

## Recovery Procedures

### Emergency: Locked Out by Kill Switch

If you cannot access the network due to stuck kill switch rules:

```bash
# Option 1: Use Shroud's cleanup
shroud ks off

# Option 2: Manual cleanup (if shroud not working)
sudo iptables -F SHROUD_KILLSWITCH
sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH
sudo iptables -X SHROUD_KILLSWITCH

# Also clean IPv6 if blocked
sudo ip6tables -D OUTPUT -j DROP 2>/dev/null || true

# Verify
sudo iptables -L OUTPUT -n
```

### Daemon Won't Start

```bash
# Check for stale lock
ls -la ~/.local/state/shroud/shroud.lock

# Check for stale socket
ls -la ${XDG_RUNTIME_DIR}/shroud.sock

# Force cleanup and restart
pkill -f shroud
rm -f ${XDG_RUNTIME_DIR}/shroud.sock
shroud
```

### Config Corrupted

```bash
# Backup and reset
mv ~/.config/shroud/config.toml ~/.config/shroud/config.toml.bak
shroud  # Will create fresh config with defaults
```

### D-Bus Monitor Not Working

D-Bus monitor failures are logged. Check:

```bash
# Is dbus-daemon running?
systemctl status dbus

# Check shroud logs
shroud debug tail
```

## Chaos Testing

Shroud includes a chaos test suite at `tests/chaos/`:

```bash
# Run all safe tests
./tests/chaos/run-chaos.sh

# Run specific test
./tests/chaos/run-chaos.sh --test kill9_recovery
```

### Test Categories

1. **Configuration Chaos**: Corrupt config, unwritable directories
2. **IPC Chaos**: Flood, malformed messages, disconnect mid-request
3. **Signal Chaos**: Signal storms, SIGSTOP/SIGCONT
4. **Kill Switch Chaos**: Rapid toggle, external rule deletion
5. **State Machine Chaos**: Concurrent commands, rapid transitions
6. **Crash Recovery**: SIGKILL with kill switch on, multiple instances
7. **Resource Exhaustion**: Low FD limit, disk full

## Monitoring Recommendations

For production headless deployments:

1. **Process Monitoring**: Use systemd with `WatchdogSec=`
2. **Log Monitoring**: Watch for `PANIC`, `ERROR`, `kill switch`
3. **Health Endpoint**: `shroud ping` returns 0 if healthy
4. **State Verification**: `shroud status --json` for automation

Example systemd watchdog integration:

```ini
[Service]
Type=notify
WatchdogSec=30
NotifyAccess=main
```

Shroud automatically sends watchdog notifications in headless mode.

## Design Decisions

### Why No Auto-Cleanup in Drop?

The `KillSwitch` `Drop` implementation only warns if dropped while enabled:

```rust
impl Drop for KillSwitch {
    fn drop(&mut self) {
        if self.enabled {
            warn!("KillSwitch dropped while enabled!");
        }
    }
}
```

This is intentional:
1. Drop runs during panic - cleanup could panic again
2. User may WANT rules to persist (headless server crash)
3. Cleanup requires sudo which may prompt

Instead, cleanup is explicit via `cleanup_with_fallback()` and panic hook.

### Why sudo -n in Commands?

We use `sudo -n` (non-interactive) for iptables commands:

```rust
Command::new("sudo").arg("-n").arg("iptables")...
```

This prevents hangs waiting for password prompts. If sudoers isn't
configured, commands fail immediately with clear error.

### Why Timeout on D-Bus Connection?

The zbus `Connection::system().await` can hang indefinitely if:
- D-Bus daemon not running
- Socket permissions wrong
- System in unusual state

The 10s timeout ensures fast failure with clear error message.

## Version History

| Version | Hardening Added |
|---------|-----------------|
| 1.8.4 | Race condition fixes, state sync |
| 1.8.5 | D-Bus timeout, sudo timeout, panic hook |
