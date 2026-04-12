# Resilience

How VPNShroud handles failure. Because things break.

---

## The Philosophy

> Murphy's Law as a Service: Anything that CAN go wrong WILL go wrong.

The kill switch is especially critical. If it fails in a way that locks users out of their network, we've made their life worse. That's not acceptable.

So we build with paranoia. Timeouts everywhere. Fallbacks for fallbacks. Recovery procedures that actually work.

---

## Hardening Patterns

### 1. Timeouts Everywhere

Nothing waits forever. Every external call has a timeout.

| Component | Timeout | Why |
|-----------|---------|-----|
| nmcli commands | 30s | NM can hang on auth issues |
| D-Bus connection | 10s | Fail fast if D-Bus is broken |
| sudo/iptables | 30s | Detect sudo prompts or kernel issues |
| IPC responses | 60s | Generous for user interaction |
| Health checks | 5s | Quick detection of tunnel problems |
| Kill switch cleanup | 5s | Don't block shutdown forever |

### 2. Graceful Degradation

When components fail, VPNShroud keeps running in a reduced capacity:

| Failure | What Happens |
|---------|--------------|
| D-Bus dies | Fall back to polling NetworkManager |
| iptables fails | Kill switch disabled, VPN still works |
| Config corrupt | Use defaults, log warning |
| Tray fails | Desktop mode continues without tray |

We'd rather work partially than crash completely.

### 3. State Reconciliation

VPNShroud periodically verifies its internal state matches reality:

- `sync_state_from_nm()` -- query NM for actual VPN state
- `sync_killswitch_state()` -- verify iptables rules exist
- `is_actually_enabled()` -- check rule presence, not just our flag

Trust, but verify. Every 30 seconds.

### 4. Crash Recovery

On startup, VPNShroud cleans up from potential previous crashes:

| Artifact | Recovery |
|----------|----------|
| Stale lock file | Check if PID is still running, override if dead |
| Stale iptables rules | `cleanup_stale_on_startup()` removes old chains |
| Stale socket file | Remove and recreate |
| Panic state | Panic hook attempts emergency cleanup |

### 5. Non-Blocking Cleanup

Shutdown can't hang. Cleanup uses timeouts:

```rust
match tokio::time::timeout(CLEANUP_TIMEOUT, cleanup()).await {
    Ok(_) => info!("Cleanup successful"),
    Err(_) => warn!("Cleanup timed out, proceeding with shutdown"),
}
```

If cleanup hangs, we log a warning and exit anyway. Better than freezing.

---

## Known Failure Modes

### Critical (User Lockout Risk)

These can lock users out of their network:

| Failure | Cause | Mitigation |
|---------|-------|------------|
| Kill switch stuck ON | SIGKILL during iptables modification | Stale rule detection on startup |
| iptables hangs | Kernel module issue | 30s timeout on sudo commands |
| D-Bus hangs | dbus-daemon frozen | 10s connection timeout |

### High (Feature Broken)

These break functionality but don't lock users out:

| Failure | Cause | Mitigation |
|---------|-------|------------|
| NM connection drops | NetworkManager restart | Auto-detect and reconnect |
| State divergence | External VPN changes (nm-applet) | Periodic state sync |
| IPC socket deleted | User error | Recreate on next CLI command |

### Medium (Degraded Operation)

These reduce capability but keep basic function:

| Failure | Cause | Mitigation |
|---------|-------|------------|
| D-Bus events stop | Connection lost | Fall back to polling |
| Config unwritable | Permissions | Continue with in-memory state |
| Health check fails | Network issue | Exponential backoff retry |

---

## Recovery Procedures

### Emergency: Locked Out by Kill Switch

If you can't reach the network because the kill switch is stuck:

```bash
# Option 1: Use VPNShroud's cleanup
shroud ks off

# Option 2: Manual cleanup
sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH
sudo iptables -F SHROUD_KILLSWITCH
sudo iptables -X SHROUD_KILLSWITCH

# IPv6 too
sudo ip6tables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
sudo ip6tables -F SHROUD_KILLSWITCH 2>/dev/null || true
sudo ip6tables -X SHROUD_KILLSWITCH 2>/dev/null || true

# Verify
sudo iptables -L OUTPUT -n
```

### Daemon Won't Start

```bash
# Check for stale lock
ls -la ~/.local/state/shroud/shroud.lock

# Check for stale socket
ls -la ${XDG_RUNTIME_DIR}/shroud.sock

# Force cleanup
pkill -f shroud
rm -f ${XDG_RUNTIME_DIR}/shroud.sock
rm -f ~/.local/state/shroud/shroud.lock

# Try again
shroud
```

### Config Corrupted

```bash
# Backup and reset
mv ~/.config/shroud/config.toml ~/.config/shroud/config.toml.bak
shroud  # Creates fresh default config
```

### D-Bus Not Working

```bash
# Is dbus-daemon running?
systemctl status dbus

# Check shroud logs
shroud debug on
shroud debug tail
```

---

## Chaos Testing

We break things on purpose to make sure recovery works.

The chaos test suite lives in `tests/chaos/`:

```bash
# Run all safe tests
./tests/chaos/run-chaos.sh

# Run specific test
./tests/chaos/run-chaos.sh --test kill9_recovery
```

### Test Categories

| Category | What It Tests |
|----------|---------------|
| Configuration | Corrupt config, unwritable directories |
| IPC | Flood, malformed messages, disconnect mid-request |
| Signals | Signal storms, SIGSTOP/SIGCONT |
| Kill Switch | Rapid toggle, external rule deletion |
| State Machine | Concurrent commands, rapid transitions |
| Crash Recovery | SIGKILL with kill switch on |
| Resources | Low FD limit, disk full |

If the tests pass, we're confident the recovery code works.

---

## Production Monitoring

For headless servers, consider:

### Systemd Watchdog

```ini
[Service]
Type=notify
WatchdogSec=30
NotifyAccess=main
```

VPNShroud sends watchdog notifications in headless mode. If it stops responding, systemd restarts it.

### Health Checks

```bash
# Returns 0 if healthy
shroud ping

# Machine-readable status
shroud status --json
```

### Log Monitoring

Watch for these in logs:
- `PANIC` -- something went very wrong
- `ERROR` -- something failed
- `kill switch` -- kill switch state changes

---

## Design Decisions

### No Auto-Cleanup in Drop

The `KillSwitch` `Drop` implementation only warns:

```rust
impl Drop for KillSwitch {
    fn drop(&mut self) {
        if self.enabled {
            warn!("KillSwitch dropped while enabled!");
        }
    }
}
```

Why not cleanup?
1. Drop runs during panic. Cleanup could panic again.
2. User may WANT rules to persist (headless server crash = keep blocking)
3. Cleanup requires sudo which may prompt

Cleanup is explicit via `cleanup_with_fallback()` and the panic hook.

### sudo -n in Commands

We use `sudo -n` (non-interactive) for iptables:

```rust
Command::new("sudo").arg("-n").arg("iptables")...
```

This prevents hangs waiting for password prompts. If sudoers isn't configured, we fail immediately with a clear error instead of hanging.

### D-Bus Timeout

The D-Bus connection can hang indefinitely if:
- D-Bus daemon not running
- Socket permissions wrong
- System in a weird state

The 10s timeout ensures we fail fast with a clear error.

---

## The Philosophy

Resilience isn't about preventing failure. It's about recovering from failure.

Things will break. Networks drop. Processes crash. Kernels panic. The question is: when it breaks, does the user end up worse off than if they'd never installed VPNShroud?

The answer must always be no.

If the kill switch crashes, it shouldn't leave the user locked out. If the daemon dies, it shouldn't leave orphaned firewall rules. If the config corrupts, it should fall back to defaults.

We're guests in this system. Guests clean up after themselves.
