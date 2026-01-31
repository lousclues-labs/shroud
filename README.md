# Shroud

[![CI](https://github.com/loujr/shroud/actions/workflows/ci.yml/badge.svg)](https://github.com/loujr/shroud/actions/workflows/ci.yml)
[![Security Audit](https://github.com/loujr/shroud/actions/workflows/security.yml/badge.svg)](https://github.com/loujr/shroud/actions/workflows/security.yml)
[![codecov](https://codecov.io/gh/loujr/shroud/graph/badge.svg)](https://codecov.io/gh/loujr/shroud)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

**A provider-agnostic VPN connection manager for Linux.**

<img height="20" alt="Shroud logo" src="https://github.com/user-attachments/assets/85c62428-7608-4dcf-8e16-434f901d5021" />


Shroud wraps around NetworkManager (OpenVPN and WireGuard) like a protective shroud around a lock mechanism — hardening security without replacing the tools you already have.

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
| Wraps around the lock | Wraps around NetworkManager (OpenVPN + WireGuard) |
| Protects the mechanism | Kill switch protects against leaks |
| Doesn't replace the lock | Doesn't replace NM, works alongside it |
| Hardens against attack | Hardens against connection failures, stale states |

The name has layers:
1. **Concealment** — A VPN shrouds your traffic
2. **Lock hardware** — Protective shell around the mechanism
3. **Architecture** — Surrounds and binds to existing tools

---

## Features

- **Provider-agnostic** — Works with `.ovpn` (OpenVPN) and `.conf` (WireGuard) configs (NordVPN, Mullvad, ProtonVPN, self-hosted, corporate VPNs)
- **Kill switch** — iptables-based traffic blocking with DNS and IPv6 leak protection
- **Auto-reconnect** — Health monitoring with exponential backoff retry
- **Formal state machine** — Disconnected → Connecting → Connected → Degraded → Reconnecting → Failed
- **Works alongside NetworkManager** — Wraps, doesn't replace (Principle I), with OpenVPN and WireGuard via NM
- **System tray integration** — KDE Plasma, GNOME with AppIndicator extension, etc.
- **Configurable via TOML** — All settings persisted across restarts
- **No telemetry** — No phoning home, no analytics (Principle IV)
- **Single binary** — One binary for both daemon and CLI (Principle VIII)
- **CLI control** — Full command-line interface for scripting and automation

*System tray menu showing connection status and controls*

<img width="313" alt="Shroud system tray menu" src="https://github.com/user-attachments/assets/7b00be8f-d97f-4b27-aabb-d4e56bb42a81" />


---

## Quick Start

```bash
git clone https://github.com/loujr/shroud.git
cd shroud
./setup.sh
```

That's it! The setup script handles everything: dependencies, building, installation, desktop entries, shell completions, and more.

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
6. **Desktop** — Creates application menu entry
7. **Autostart** — Use `shroud autostart on` to enable start on login
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
sudo pacman -S rust networkmanager networkmanager-openvpn networkmanager-wireguard openvpn wireguard-tools iptables polkit libappindicator-gtk3

# Debian/Ubuntu
sudo apt install rustc cargo network-manager network-manager-openvpn network-manager-wireguard openvpn wireguard-tools iptables policykit-1 libayatana-appindicator3-1

# Fedora
sudo dnf install rust cargo NetworkManager NetworkManager-openvpn NetworkManager-wireguard openvpn wireguard-tools iptables polkit libappindicator-gtk3
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

---

## Importing VPN Configs

Shroud can import WireGuard and OpenVPN config files directly:

```bash
# Import a single config
shroud import ~/mullvad-us1.conf

# Import with custom name
shroud import ~/vpn.ovpn --name "Work VPN"

# Import all configs from a directory
shroud import ~/vpn-configs/

# Preview what would be imported
shroud import ~/configs/ --dry-run

# Import and connect immediately
shroud import ~/vpn.conf --connect
```

Supported formats:

- WireGuard: .conf files with [Interface] and [Peer] sections
- OpenVPN: .ovpn files

You can still import via nmcli if you prefer:

Before using Shroud, import your VPN configs into NetworkManager:

```bash
# Import a single OpenVPN config
nmcli connection import type openvpn file /path/to/config.ovpn

# Import a single WireGuard config
nmcli connection import type wireguard file /path/to/config.conf

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

# Start on login (recommended)
shroud autostart on
```

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

# Import configs
shroud import ~/mullvad-us1.conf           # Import WireGuard config
shroud import ~/corporate.ovpn --name "Work VPN"  # Import OpenVPN config
shroud import ~/vpn-configs/               # Import all configs in a directory
shroud import ~/configs/ --dry-run         # Preview what would be imported
shroud import ~/vpn.conf --connect         # Import and connect

# Kill switch control
shroud killswitch on            # Enable kill switch
shroud killswitch off           # Disable kill switch
shroud ks toggle                # Toggle kill switch
shroud ks status                # Show kill switch status

# Auto-reconnect control
shroud auto-reconnect on        # Enable auto-reconnect
shroud ar off                   # Disable auto-reconnect
shroud ar toggle                # Toggle auto-reconnect

# Autostart control
shroud autostart on             # Enable autostart on login
shroud autostart off            # Disable autostart
shroud autostart status         # Show autostart status
shroud autostart toggle         # Toggle autostart
shroud cleanup                  # Remove old systemd service and stale files

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
shroud reload                   # Reload configuration without restart
shroud update                   # Build, install, and restart (dev workflow)
shroud version --check          # Check if rebuild is needed

# Help
shroud --help                   # Show main help
shroud help connect             # Help for specific command
shroud connect --help           # Alternative help syntax

# Security
shroud audit                    # Check dependencies for vulnerabilities
```

---

## Development Workflow

### Quick rebuild and restart

```bash
shroud update
```

Other commands:

```bash
shroud restart         # Restart daemon
shroud reload          # Reload config without restart
shroud version --check # Check if rebuild needed
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

# DNS leak protection mode: "tunnel" | "strict" | "localhost" | "any"
dns_mode = "tunnel"

# Block DNS-over-HTTPS to known providers (tunnel/strict)
block_doh = true

# Additional DoH provider IPs to block
custom_doh_blocklist = []

# IPv6 leak protection: "block" | "tunnel" | "off"
ipv6_mode = "block"
```

---

## Security

See [SECURITY.md](SECURITY.md) for vulnerability reporting guidance.

### Kill Switch

When enabled, the kill switch creates iptables rules that:

1. **Allow** loopback traffic
2. **Allow** established/related connections
3. **Allow** traffic through VPN tunnel interfaces (tun*, wg*, tap*)
4. **Allow** traffic to VPN server IPs (for connection establishment)
5. **Allow** local network access (192.168.0.0/16, 10.0.0.0/8, 172.16.0.0/12)
6. **Allow** DHCP for network configuration
7. **Drop** everything else

## Kill Switch Privileges

The kill switch requires root privileges to manage firewall rules. Shroud supports a polkit policy that enables passwordless operation for active desktop sessions.

### Option 1: Polkit Policy (Recommended)

Install during setup or standalone:

```bash
./setup.sh                # Will prompt to install polkit policy
./setup.sh --install-polkit
```

What this grants:

- Run iptables/ip6tables without password
- Only for active local desktop sessions
- SSH/remote sessions still require authentication

Policy location:

```
/usr/share/polkit-1/actions/com.shroud.killswitch.policy
```

Remove policy:

```bash
./setup.sh --uninstall-polkit
# or
sudo rm /usr/share/polkit-1/actions/com.shroud.killswitch.policy
```

### Option 2: Password Prompts

If you skip the polkit policy, you will be prompted when enabling/disabling the kill switch and during shutdown cleanup.

### Stale Rule Cleanup

If Shroud crashes, it will detect and clean stale rules on startup. If automatic cleanup fails, run:

```bash
sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH
sudo iptables -F SHROUD_KILLSWITCH
sudo iptables -X SHROUD_KILLSWITCH
sudo ip6tables -D OUTPUT -j SHROUD_KILLSWITCH
sudo ip6tables -F SHROUD_KILLSWITCH
sudo ip6tables -X SHROUD_KILLSWITCH
```

### DNS Leak Protection

| Mode | Behavior | Use Case |
|------|----------|----------|
| `tunnel` (default) | DNS only through VPN interface | Maximum security |
| `strict` | Tunnel + DoH/DoT blocking | Privacy-critical environments |
| `localhost` | DNS to 127.0.0.0/8, ::1, 127.0.0.53 | systemd-resolved, local DNS cache |
| `any` | DNS to any destination | Legacy compatibility (not recommended) |

When `dns_mode` is `tunnel` or `strict`, Shroud explicitly drops DNS (53) on non-VPN interfaces and blocks DNS-over-TLS (853). If `block_doh = true`, known DoH provider IPs are blocked on port 443 unless routed through the VPN.

### IPv6 Leak Protection

| Mode | Behavior | Use Case |
|------|----------|----------|
| `block` (default) | Drop all IPv6 except loopback | Most VPNs don't tunnel IPv6 |
| `tunnel` | IPv6 only through VPN interface | VPN properly tunnels IPv6 |
| `off` | No IPv6 restrictions | Full IPv6 connectivity (may leak) |

### Auditing Rules

```bash
# View active kill switch rules
sudo iptables -S SHROUD_KILLSWITCH

# View OUTPUT rules (jump into kill switch chain)
sudo iptables -S OUTPUT

### Dependency Audits

Shroud uses cargo-audit to check dependencies against the RustSec Advisory Database.

```bash
./scripts/audit.sh

# Or via the CLI
shroud audit
```

## Kill Switch Privileges

The kill switch requires root privileges to manage iptables rules. Shroud uses `sudo`
with a NOPASSWD rule for reliable operation across all session types (desktop, SSH,
headless).

### Setup (Automatic)

```bash
./setup.sh  # Will prompt to install sudoers rule
```

Or install just the sudoers rule:

```bash
./setup.sh --install-sudoers
```

### Setup (Manual)

```bash
# Arch/Fedora/RHEL (wheel group)
echo '%wheel ALL=(ALL) NOPASSWD: /usr/sbin/iptables, /usr/sbin/ip6tables, /usr/sbin/nft' | sudo tee /etc/sudoers.d/shroud
sudo chmod 440 /etc/sudoers.d/shroud

# Debian/Ubuntu (sudo group)
echo '%sudo ALL=(ALL) NOPASSWD: /usr/sbin/iptables, /usr/sbin/ip6tables, /usr/sbin/nft' | sudo tee /etc/sudoers.d/shroud
sudo chmod 440 /etc/sudoers.d/shroud
```

### Security Notes

- Only `iptables`, `ip6tables`, and `nft` are granted passwordless access
- Only users in the wheel/sudo group can use this
- Remove anytime with: `sudo rm /etc/sudoers.d/shroud`
```

---

## Troubleshooting

### Tray Icon Not Appearing

1. Ensure your DE supports StatusNotifierItem (SNI)
2. For GNOME, install the AppIndicator extension
3. Check if `XDG_RUNTIME_DIR` and `DBUS_SESSION_BUS_ADDRESS` are set

### Kill Switch Not Working

1. Verify iptables is installed: `iptables --version`
2. Verify sudoers rule: `sudo -n iptables -L -n`
3. Install the sudoers rule: `./setup.sh --install-sudoers`

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
sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH
sudo iptables -F SHROUD_KILLSWITCH
sudo iptables -X SHROUD_KILLSWITCH
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
- Desktop entries and autostart
- Shell completions
- Sudoers rule at `/etc/sudoers.d/shroud` (if installed)
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

This software is dual-licensed under the **GNU General Public License v3.0 (GPLv3)** and a **Commercial License**.

### Open Source Use

For open source use, this program is free software: you can redistribute it and/or modify it under the terms of the **GNU General Public License** as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.

### Commercial Use

For commercial use, or to use this software in a proprietary product without the restrictions of the GPL, please contact the author for commercial licensing options.

See [LICENSE](LICENSE) for full details.

---

*Shroud: Wrap your VPN in armor, not bloatware.*
