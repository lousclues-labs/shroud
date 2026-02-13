# CLI Reference

Every command Shroud understands. No hidden features.

---

## How It Works

```bash
shroud [OPTIONS]              # Start the daemon (tray app)
shroud [OPTIONS] <COMMAND>    # Send command to running daemon
```

The first form starts Shroud. The second talks to an already-running Shroud.

---

## Global Options

These work with any command:

| Option | What It Does |
|--------|--------------|
| `-h, --help` | Show help |
| `-V, --version` | Show version |
| `-v, --verbose` | More logging (stack them: -v, -vv, -vvv) |
| `--log-level <LEVEL>` | Set level: error, warn, info, debug, trace |
| `--log-file <PATH>` | Log to file instead of stderr |
| `--json` | Output as JSON (for scripting) |
| `-q, --quiet` | No output, just exit code |
| `--timeout <SECS>` | Daemon communication timeout (default: 5) |
| `-H, --headless` | Run without tray icon (servers) |
| `--desktop` | Force tray mode |

---

## Connection Commands

### `shroud connect <NAME>`

Connect to a VPN.

```bash
shroud connect ireland-42
shroud connect "Work VPN"
```

### `shroud disconnect`

Disconnect from the current VPN.

```bash
shroud disconnect
```

### `shroud reconnect`

Disconnect and reconnect to the current VPN.

```bash
shroud reconnect
```

### `shroud switch <NAME>`

Atomic switch: disconnect from current, connect to new.

```bash
shroud switch us-west-2
```

### `shroud list` / `shroud ls`

List available VPN connections.

```bash
shroud list
shroud ls --json    # Machine-readable
```

### `shroud status`

What's happening right now.

```bash
shroud status
shroud status --json
```

---

## Kill Switch Commands

### `shroud killswitch on` / `shroud ks on`

Enable the kill switch. Traffic stops if VPN drops.

```bash
shroud ks on
```

### `shroud killswitch off` / `shroud ks off`

Disable the kill switch.

```bash
shroud ks off
```

### `shroud killswitch toggle` / `shroud ks toggle`

Flip the kill switch.

```bash
shroud ks toggle
```

### `shroud killswitch status` / `shroud ks status`

Check kill switch state.

```bash
shroud ks status
```

See [Kill Switch](KILLSWITCH.md) for the full story.

---

## Auto-Reconnect Commands

### `shroud auto-reconnect on` / `shroud ar on`

Enable auto-reconnect.

```bash
shroud ar on
```

### `shroud auto-reconnect off` / `shroud ar off`

Disable auto-reconnect.

```bash
shroud ar off
```

### `shroud auto-reconnect toggle` / `shroud ar toggle`

Toggle auto-reconnect.

```bash
shroud ar toggle
```

---

## Import Commands

### `shroud import <PATH>`

Import VPN configs into NetworkManager.

```bash
shroud import ~/vpn.ovpn                      # Single file
shroud import ~/vpn-configs/                  # Directory
shroud import ~/vpn.conf --name "My VPN"      # Custom name
shroud import ~/vpn.conf --connect            # Import and connect
shroud import ~/configs/ --dry-run            # Preview only
```

---

## Autostart Commands

### `shroud autostart on` / `shroud startup on`

Launch Shroud on login.

```bash
shroud autostart on
```

### `shroud autostart off`

Don't launch on login.

```bash
shroud autostart off
```

### `shroud autostart status`

Check autostart state.

```bash
shroud autostart status
```

### `shroud autostart toggle`

Toggle autostart.

```bash
shroud autostart toggle
```

---

## Daemon Control

### `shroud ping`

Check if daemon is running.

```bash
shroud ping
```

### `shroud quit` / `shroud stop` / `shroud exit`

Stop the daemon gracefully.

```bash
shroud quit
```

### `shroud restart`

Restart the daemon.

```bash
shroud restart
```

### `shroud reload`

Reload config without restart.

```bash
shroud reload
```

### `shroud refresh`

Refresh the VPN connection list.

```bash
shroud refresh
```

---

## Debug Commands

### `shroud debug on`

Enable debug logging to file.

```bash
shroud debug on
```

### `shroud debug off`

Disable debug logging.

```bash
shroud debug off
```

### `shroud debug log-path`

Show where logs are written.

```bash
shroud debug log-path
```

### `shroud debug tail`

Follow the log file.

```bash
shroud debug tail
```

### `shroud debug dump`

Dump internal state as JSON.

```bash
shroud debug dump
```

---

## Diagnostic Commands

### `shroud doctor`

Run diagnostics. Check everything.

```bash
shroud doctor
```

### `shroud audit`

Check dependencies for known vulnerabilities.

```bash
shroud audit
```

---

## Other Commands

### `shroud cleanup`

Remove old systemd services and stale files.

```bash
shroud cleanup
```

### `shroud update`

Build, install, and restart. Developer workflow.

```bash
shroud update
```

### `shroud version`

Show version info.

```bash
shroud version
shroud version --check    # Check if rebuild needed
```

### `shroud help <COMMAND>`

Help for a specific command.

```bash
shroud help connect
shroud connect --help     # Same thing
```

---

## Aliases

Short names for common commands:

| Alias | Full Command |
|-------|--------------|
| `ls` | `list` |
| `ks` | `killswitch` |
| `ar` | `auto-reconnect` |
| `startup` | `autostart` |
| `stop` | `quit` |
| `exit` | `quit` |

---

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Command failed |
| 2 | Daemon not running |
| 3 | Timeout waiting for daemon |

Useful for scripting:

```bash
if shroud ping; then
    echo "Daemon is running"
else
    echo "Daemon is not running (exit code: $?)"
fi
```

---

## Examples

```bash
# Start Shroud
shroud

# Connect and enable kill switch
shroud connect ireland-42
shroud ks on

# Check status
shroud status

# Switch VPNs
shroud switch us-west-2

# List VPNs as JSON
shroud list --json

# Run in headless mode
shroud --headless

# Debug a problem
shroud debug on
shroud debug tail
```

---

## The Philosophy

The CLI should be obvious. If you have to read the docs to figure out how to connect, we failed.

`shroud connect my-vpn` connects. `shroud disconnect` disconnects. `shroud ks on` enables the kill switch.

No surprises. No magic flags. Just do what it says.
