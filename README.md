# Shroud

**A provider-agnostic VPN connection manager for Linux.**

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)

Shroud wraps around NetworkManager and OpenVPN like a protective shroud around a lock mechanism — hardening security without replacing the tools you already have.

---

## Why "Shroud"?

```
┌─────────────────────────────────────────┐
│                                         │
│         ┌───────────┐                   │
│         │  SHROUD   │ ← Protective      │
│         │ ┌───────┐ │    outer casing   │
│         │ │ LOCK  │ │                   │
│         │ │MECHANISM│ ← The vulnerable  │
│         │ └───────┘ │    internals      │
│         └───────────┘                   │
│                                         │
└─────────────────────────────────────────┘
```

A **lock shroud** is the protective metal casing that surrounds a padlock's shackle, preventing tampering. That's exactly what this tool does:

| Lock Shroud | Shroud (This Tool) |
|-------------|-------------------|
| Wraps around the lock | Wraps around NetworkManager + OpenVPN |
| Protects the mechanism | Kill switch protects against leaks |
| Doesn't replace the lock | Doesn't replace NM, works alongside it |
| Hardens against attack | Hardens against connection failures, stale states |

The name has layers:
1. **Concealment** — A VPN shrouds your traffic
2. **Lock hardware** — Protective shell around the mechanism
3. **Architecture** — Surrounds and binds to existing tools

---

## Features

- **Provider-agnostic** — Works with any `.ovpn` config file (NordVPN, Mullvad, ProtonVPN, self-hosted, corporate VPNs)
- **Kill switch** — nftables-based traffic blocking with DNS and IPv6 leak protection
- **Auto-reconnect** — Health monitoring with exponential backoff retry
- **Formal state machine** — Disconnected → Connecting → Connected → Degraded → Reconnecting → Failed
- **Works alongside NetworkManager** — Wraps, doesn't replace (Principle I)
- **System tray integration** — KDE Plasma, GNOME with AppIndicator extension, etc.
- **Configurable via TOML** — All settings persisted across restarts
- **No telemetry** — No phoning home, no analytics (Principle IV)
- **Single binary** — One binary for both daemon and CLI (Principle VIII)
- **CLI control** — Full command-line interface for scripting and automation

---

## Quick Start

```bash
git clone https://github.com/loujr/shroud.git
cd shroud
./setup.sh
```

That's it! The setup script handles everything: dependencies, building, installation, systemd service, shell completions, and more.

---

## Installation

### Using setup.sh (Recommended)

The setup script provides a complete installation experience:

```bash
# Clone the repository
git clone https://github.com/loujr/shroud.git
cd shroud

# Full installation (builds, installs, configures everything)
./setup.sh

# Or with options
./setup.sh --help              # Show all options
./setup.sh --dry-run install   # Preview what would be done
./setup.sh --verbose install   # Detailed output
./setup.sh --force install     # Skip confirmations
```

#### What setup.sh Does

1. **Pre-flight checks** — Verifies distro, display server, NetworkManager
2. **Dependencies** — Installs required packages via your package manager
3. **Build** — Compiles release binary with cargo
4. **Install** — Places binary in `~/.local/bin/` with backup/rollback
5. **Configure** — Creates `~/.config/shroud/config.toml` with defaults
6. **Service** — Installs and enables systemd user service
7. **Desktop** — Creates application menu entry and autostart
8. **Completions** — Installs shell completions for bash, zsh, fish
9. **Polkit** — Optionally configures passwordless kill switch (with security warning)
10. **Verify** — Tests the installation and shows summary

#### Other Commands

```bash
./setup.sh status      # Check installation status
./setup.sh update      # Update to latest (preserves config)
./setup.sh repair      # Reinstall without rebuilding
./setup.sh check       # Check dependencies only
./setup.sh uninstall   # Complete removal (prompts for config/logs)
```

### Manual Installation

If you prefer manual control:

#### Dependencies

```bash
# Arch Linux
sudo pacman -S rust networkmanager networkmanager-openvpn openvpn nftables polkit libappindicator-gtk3

# Debian/Ubuntu
sudo apt install rustc cargo network-manager network-manager-openvpn openvpn nftables policykit-1 libayatana-appindicator3-1

# Fedora
sudo dnf install rust cargo NetworkManager NetworkManager-openvpn openvpn nftables polkit libappindicator-gtk3
```

#### Build and Install

```bash
# Build
cargo build --release

# Install binary
mkdir -p ~/.local/bin
cp target/release/shroud ~/.local/bin/
chmod +x ~/.local/bin/shroud

# Verify
shroud --version
```

#### Systemd Service (Optional)

```bash
mkdir -p ~/.config/systemd/user
cp systemd/shroud.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now shroud.service
```

---

## Importing VPN Configs

Before using Shroud, import your `.ovpn` files into NetworkManager:

```bash
# Import a single config
nmcli connection import type openvpn file /path/to/config.ovpn

# The connection will be named after the file (e.g., "us-east-1")
# You can rename it:
nmcli connection modify "us-east-1" connection.id "USA East"

# List imported VPN connections
nmcli -t -f NAME,TYPE connection show | grep vpn
```

---

## Usage

### Starting Shroud

```bash
# Start the daemon (tray application)
shroud

# With verbose logging
shroud -v          # Info level
shroud -vv         # Debug level
shroud -vvv        # Trace level

# With specific log level
shroud --log-level debug

# With file logging
shroud --log-file /tmp/shroud.log

# Via systemd (recommended)
systemctl --user start shroud
```

### CLI Architecture

Shroud operates in two modes from a single binary:

```
┌─────────────────────────────────────────────────────────────┐
│                                                             │
│   $ shroud                    $ shroud connect ireland-42   │
│   (daemon mode)               (client mode)                 │
│                                                             │
│   ┌─────────────┐             ┌─────────────┐               │
│   │   Shroud    │◄────────────│   Shroud    │               │
│   │   Daemon    │   command   │   Client    │               │
│   │             │─────────────►             │               │
│   │  (tray app) │   response  │  (one-shot) │               │
│   └─────────────┘             └─────────────┘               │
│         ▲                                                   │
│         │ Unix socket: $XDG_RUNTIME_DIR/shroud.sock         │
│         │                                                   │
└─────────────────────────────────────────────────────────────┘
```

- **Daemon mode** (`shroud`): Starts the tray application, listens for CLI commands
- **Client mode** (`shroud <command>`): Sends command to running daemon and exits

### CLI Commands

```bash
# Connection management
shroud connect ireland-42       # Connect to a VPN
shroud disconnect               # Disconnect from VPN
shroud reconnect                # Reconnect to current VPN
shroud switch us-west-2         # Switch to different VPN

# Status and information
shroud status                   # Show current status
shroud status --json            # JSON output for scripting
shroud list                     # List available VPN connections
shroud ls --json                # List as JSON

# Kill switch control
shroud killswitch on            # Enable kill switch
shroud killswitch off           # Disable kill switch
shroud ks toggle                # Toggle kill switch
shroud ks status                # Show kill switch status

# Auto-reconnect control
shroud auto-reconnect on        # Enable auto-reconnect
shroud ar off                   # Disable auto-reconnect
shroud ar toggle                # Toggle auto-reconnect

# Debug and diagnostics
shroud debug on                 # Enable debug logging to file
shroud debug off                # Disable debug logging
shroud debug log-path           # Show log file path
shroud debug tail               # Follow log file (like tail -f)
shroud debug dump               # Dump internal state as JSON

# Daemon control
shroud ping                     # Check if daemon is running
shroud refresh                  # Refresh VPN connection list
shroud quit                     # Stop the daemon gracefully
shroud restart                  # Restart the daemon

# Help
shroud --help                   # Show main help
shroud help connect             # Help for specific command
shroud connect --help           # Alternative help syntax
```

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Command failed |
| 2 | Daemon not running |
| 3 | Timeout waiting for daemon |

---

## Configuration

Shroud stores configuration in `~/.config/shroud/config.toml`:

```toml
# Config version for migration support
version = 1

# Automatically reconnect when VPN drops
auto_reconnect = true

# Last successfully connected server (for quick reconnect)
last_server = "us-east-1"

# Health check interval in seconds (0 to disable)
health_check_interval_secs = 30

# Latency threshold for degraded state (ms)
health_degraded_threshold_ms = 2000

# Maximum reconnection attempts before giving up
max_reconnect_attempts = 10

# Enable kill switch (blocks non-VPN traffic)
kill_switch_enabled = false

# DNS leak protection mode: "tunnel" | "localhost" | "any"
dns_mode = "tunnel"

# IPv6 leak protection: "block" | "tunnel" | "off"
ipv6_mode = "block"
```

---

## Security

### Kill Switch

When enabled, the kill switch creates nftables rules that:

1. **Allow** loopback traffic
2. **Allow** established/related connections
3. **Allow** traffic through VPN tunnel interfaces (tun*, wg*, tap*)
4. **Allow** traffic to VPN server IPs (for connection establishment)
5. **Allow** local network access (192.168.0.0/16, 10.0.0.0/8, 172.16.0.0/12)
6. **Allow** DHCP for network configuration
7. **Drop** everything else

### DNS Leak Protection

| Mode | Behavior | Use Case |
|------|----------|----------|
| `tunnel` (default) | DNS only through VPN interface | Maximum security |
| `localhost` | DNS to 127.0.0.0/8, ::1, 127.0.0.53 | systemd-resolved, local DNS cache |
| `any` | DNS to any destination | Legacy compatibility (not recommended) |

### IPv6 Leak Protection

| Mode | Behavior | Use Case |
|------|----------|----------|
| `block` (default) | Drop all IPv6 except loopback | Most VPNs don't tunnel IPv6 |
| `tunnel` | IPv6 only through VPN interface | VPN properly tunnels IPv6 |
| `off` | No IPv6 restrictions | Full IPv6 connectivity (may leak) |

### Auditing Rules

```bash
# View active kill switch rules
sudo nft list table inet shroud_killswitch

# View all tables
sudo nft list tables
```

---

## Troubleshooting

### Tray Icon Not Appearing

1. Ensure your DE supports StatusNotifierItem (SNI)
2. For GNOME, install the AppIndicator extension
3. Check if `XDG_RUNTIME_DIR` and `DBUS_SESSION_BUS_ADDRESS` are set

### Kill Switch Not Working

1. Verify nftables is installed: `nft --version`
2. Check polkit is running: `systemctl status polkit`
3. Try enabling manually and check for pkexec prompt

### VPN Connection Fails

1. Test with nmcli directly: `nmcli con up "connection-name"`
2. Check NetworkManager logs: `journalctl -u NetworkManager -f`
3. Verify OpenVPN plugin is installed

### Network Locked After Crash

If Shroud crashes with kill switch enabled:

```bash
# Shroud automatically cleans stale rules on next start
shroud

# Or manually clean up
sudo nft delete table inet shroud_killswitch
```

### Debug Logging

```bash
# Enable debug output
RUST_LOG=debug shroud

# Or use CLI
shroud debug on
shroud debug tail
```

---

## Uninstalling

```bash
# Complete uninstall with prompts
./setup.sh uninstall

# Force uninstall without prompts
./setup.sh --force uninstall
```

This removes:
- Binary from `~/.local/bin/`
- Systemd service
- Desktop entries and autostart
- Shell completions
- Polkit policy (if installed)
- Optionally: config and logs (prompts)

---

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed system design.

## Principles

See [PRINCIPLES.md](PRINCIPLES.md) for the core values that guide development.

---

## Contributing

Contributions are welcome! Please read [PRINCIPLES.md](PRINCIPLES.md) first.

Before submitting a PR:

1. `cargo fmt` — Format code
2. `cargo clippy -D warnings` — No warnings
3. `cargo test` — All tests pass

---

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

---

*Shroud: Wrap your VPN in armor, not bloatware.*
