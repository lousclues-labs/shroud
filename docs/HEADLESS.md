# Headless Mode

No GUI? No problem.

Headless mode runs VPNShroud as a system service on servers, containers, or any system without a desktop. Same protection, no tray icon required.

---

## What You Get

```
┌────────────────────────────────────────────────────────────────┐
│                      HEADLESS MODE                             │
├────────────────────────────────────────────────────────────────┤
│                                                                │
│   ✓ Systemd integration                                       │
│     └─ Runs as a proper service. Starts on boot.              │
│                                                                │
│   ✓ Auto-connect on startup                                   │
│     └─ VPN connects before anything else can leak.            │
│                                                                │
│   ✓ Boot kill switch                                          │
│     └─ Traffic blocked until VPN is up. No window of exposure.│
│                                                                │
│   ✓ Infinite reconnect                                        │
│     └─ Never gives up. Servers don't get to quit.             │
│                                                                │
│   ✓ Full CLI access                                           │
│     └─ Everything works over SSH. No desktop needed.          │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

---

## Quick Start

### 1. Install

```bash
git clone https://github.com/loujr/shroud
cd shroud
cargo build --release

# Install for headless operation
sudo ./setup.sh --headless
```

### 2. Import a VPN Connection

```bash
# OpenVPN
sudo nmcli connection import type openvpn file /path/to/your-vpn.ovpn

# WireGuard
sudo nmcli connection import type wireguard file /path/to/wg0.conf

# Check the connection name
nmcli connection show
```

### 3. Configure

Edit `/etc/shroud/config.toml`:

```toml
[headless]
auto_connect = true
startup_server = "your-vpn-connection-name"  # ← Change this!
kill_switch_on_boot = true

[killswitch]
allow_lan = true
```

That `startup_server` value must match exactly what `nmcli connection show` displays.

### 4. Start

```bash
# Enable and start the service
sudo systemctl enable shroud
sudo systemctl start shroud

# Check it's working
sudo systemctl status shroud
shroud status
```

You're done. The server is protected.

---

## How It Works

When VPNShroud starts in headless mode, here's what happens:

1. **Boot kill switch activates** -- all traffic blocked except loopback and LAN
2. **VPN connects** -- using the connection named in `startup_server`
3. **Kill switch transfers** -- boot rules replaced with normal kill switch rules
4. **Monitoring begins** -- health checks, auto-reconnect, the usual

If the VPN drops, VPNShroud reconnects. If VPNShroud crashes, systemd restarts it. If the server reboots, everything comes back up automatically.

The goal: you configure it once and forget about it.

---

## Configuration

All headless options live in the `[headless]` section of your config:

```toml
[headless]
# Connect to VPN when VPNShroud starts
auto_connect = true

# Which VPN to connect to
startup_server = "mullvad-us1"

# Block all traffic until VPN connects
kill_switch_on_boot = true

# Fail to start if kill switch can't be enabled
require_kill_switch = true

# Keep kill switch active even after VPNShroud exits
persist_kill_switch = false

# Never stop trying to reconnect (0 = infinite)
max_reconnect_attempts = 0

# Initial delay between reconnection attempts
reconnect_delay_secs = 5
```

### Config Location

| Mode | Path |
|------|------|
| Desktop | `~/.config/shroud/config.toml` |
| Headless (root) | `/etc/shroud/config.toml` |

---

## Systemd Commands

```bash
# Start the service
sudo systemctl start shroud

# Stop the service
sudo systemctl stop shroud

# Restart the service
sudo systemctl restart shroud

# Enable auto-start on boot
sudo systemctl enable shroud

# Disable auto-start
sudo systemctl disable shroud

# Check status
sudo systemctl status shroud
```

### Viewing Logs

```bash
# Follow logs live
journalctl -u shroud -f

# Today's logs
journalctl -u shroud --since today

# Errors only
journalctl -u shroud -p err

# Last 50 lines
journalctl -u shroud -n 50
```

---

## CLI Commands

Everything works over SSH. No GUI required.

```bash
shroud status              # Connection status
shroud connect <name>      # Connect to VPN
shroud disconnect          # Disconnect
shroud switch <name>       # Switch VPNs
shroud list                # Available VPNs
shroud ks status           # Kill switch status
shroud ks on               # Enable kill switch
shroud ks off              # Disable kill switch
shroud doctor              # Run diagnostics
shroud debug on            # Enable debug logging
shroud debug tail          # Follow logs
```

---

## Troubleshooting

### Service won't start

```bash
# Check the logs
journalctl -u shroud -n 50

# Common issues:
# - startup_server not set or misspelled
# - VPN connection doesn't exist in NM
# - NetworkManager not running
```

### VPN won't connect

```bash
# Is NetworkManager running?
sudo systemctl status NetworkManager

# Does the connection exist?
nmcli connection show | grep vpn

# Test the connection directly
nmcli connection up "your-vpn-name"

# Check VPNShroud logs
journalctl -u shroud -f
```

### Locked out by kill switch

If the boot kill switch blocks SSH:

**If you have console access:**
```bash
# Disable the kill switch
shroud ks off

# Or manually
sudo iptables -D OUTPUT -j SHROUD_BOOT_KS
sudo iptables -F SHROUD_BOOT_KS
sudo iptables -X SHROUD_BOOT_KS
```

**Prevention:** Make sure `allow_lan = true` in your config so SSH over LAN keeps working.

---

## Security Considerations

- **Runs as root** -- required for iptables. This is intentional.
- **Boot kill switch** -- blocks ALL traffic until VPN connects. This includes SSH unless you're on the LAN.
- **persist_kill_switch** -- if set to `true`, traffic stays blocked even after VPNShroud exits. Use carefully.
- **Config permissions** -- should be 600 (owner read/write only).

---

## Uninstalling

```bash
# Stop and disable the service
sudo systemctl stop shroud
sudo systemctl disable shroud

# Run uninstall
sudo ./setup.sh --uninstall

# Remove config and state (optional)
sudo rm -rf /etc/shroud /var/lib/shroud
```

---

## The Philosophy

A server VPN should be invisible. Configure it once, forget it exists, trust that it's working.

If you're SSHing into your server to babysit the VPN, we've failed. Headless mode is designed so you never have to think about it.

It connects. It stays connected. It reconnects if it falls. That's the whole job.
