# Shroud Headless Mode

Run Shroud as a system service on servers without a GUI.

## Overview

Headless mode provides:
- **No GUI dependencies** — No tray icon, no desktop notifications
- **Systemd integration** — Runs as a system service with watchdog support
- **Auto-connect** — Connects to VPN automatically on boot
- **Boot kill switch** — Blocks all traffic until VPN connects
- **Infinite reconnect** — Never gives up trying to reconnect

## Quick Start

### 1. Install

```bash
# Clone and build
git clone https://github.com/loujr/shroud
cd shroud
cargo build --release

# Install for headless operation
sudo ./setup.sh --headless
```

### 2. Import VPN Connection

```bash
# OpenVPN
sudo nmcli connection import type openvpn file /path/to/your-vpn.ovpn

# WireGuard
sudo nmcli connection import type wireguard file /path/to/wg0.conf

# List connections to find the name
nmcli connection show
```

### 3. Configure

Edit `/etc/shroud/config.toml`:

```toml
[headless]
auto_connect = true
startup_server = "your-vpn-connection-name"  # <-- Change this!
kill_switch_on_boot = true

[killswitch]
allow_lan = true
```

### 4. Start

```bash
# Enable and start
sudo systemctl enable shroud
sudo systemctl start shroud

# Check status
sudo systemctl status shroud
shroud status
```

## Configuration Reference

### [headless] Section

| Option | Default | Description |
|--------|---------|-------------|
| `auto_connect` | `false` | Connect to VPN on startup |
| `startup_server` | `null` | VPN connection name to connect to |
| `max_reconnect_attempts` | `0` | Max retries (0 = infinite) |
| `reconnect_delay_secs` | `5` | Initial delay between retries |
| `kill_switch_on_boot` | `true` | Enable kill switch before VPN connects |
| `require_kill_switch` | `true` | Fail startup if kill switch fails |
| `persist_kill_switch` | `false` | Keep kill switch after Shroud exits |

### [killswitch] Section

| Option | Default | Description |
|--------|---------|-------------|
| `allow_lan` | `true` | Allow local network traffic |

## Systemd Commands

```bash
# Start/stop/restart
sudo systemctl start shroud
sudo systemctl stop shroud
sudo systemctl restart shroud

# Enable/disable auto-start
sudo systemctl enable shroud
sudo systemctl disable shroud

# View status
sudo systemctl status shroud

# View logs
journalctl -u shroud -f           # Follow logs
journalctl -u shroud --since today # Today's logs
journalctl -u shroud -p err        # Errors only
```

## CLI Commands

All standard Shroud commands work in headless mode:

```bash
shroud status          # Connection status
shroud connect <name>  # Connect to VPN
shroud disconnect      # Disconnect
shroud ks status       # Kill switch status
shroud gateway on      # Enable gateway mode
shroud doctor          # Diagnose issues
```

## Troubleshooting

### Service won't start

```bash
# Check for errors
journalctl -u shroud -n 50

# Common issues:
# - startup_server not set in config
# - VPN connection doesn't exist (check: nmcli connection show)
# - NetworkManager not running
```

### VPN won't connect

```bash
# Check NetworkManager
sudo systemctl status NetworkManager

# Test connection manually
nmcli connection up "your-vpn-name"

# Check shroud logs
journalctl -u shroud -f
```

### Kill switch blocking everything

```bash
# Check kill switch status
shroud ks status

# Disable temporarily
shroud ks off

# Check if boot kill switch is stuck
sudo iptables -L SHROUD_BOOT_KS

# Manual cleanup if needed
sudo iptables -D OUTPUT -j SHROUD_BOOT_KS
sudo iptables -F SHROUD_BOOT_KS
sudo iptables -X SHROUD_BOOT_KS
```

## Security Notes

- Headless mode runs as root (required for iptables)
- Boot kill switch blocks ALL traffic until VPN connects
- If `persist_kill_switch = true`, traffic stays blocked after Shroud exits
- Config file permissions should be 600 (owner read/write only)

## Uninstall

```bash
sudo ./setup.sh --uninstall

# Remove config and state (optional)
sudo rm -rf /etc/shroud /var/lib/shroud
```
