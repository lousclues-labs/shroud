# Shroud VPN Gateway

Route your entire network's traffic through the VPN.

## Overview

Gateway mode transforms your Shroud machine into a VPN router:

```
┌──────────────────────────────────────────────────────────────┐
│                       YOUR NETWORK                           │
├──────────────────────────────────────────────────────────────┤
│                                                               │
│   Phone ────┐                                                │
│   Laptop ───┼──→ Shroud Gateway ──→ VPN ──→ Internet        │
│   Smart TV ─┘                                                │
│                                                               │
└──────────────────────────────────────────────────────────────┘
```

**Benefits:**
- Protect devices that can't run VPNs (Smart TVs, gaming consoles, IoT)
- One VPN connection protects entire network
- Kill switch protects all routed traffic
- No per-device VPN configuration

## Quick Start

### 1. Install Shroud in Headless Mode

```bash
sudo ./setup.sh --headless
```

### 2. Connect to VPN

```bash
shroud connect your-vpn-name
```

### 3. Enable Gateway

```bash
shroud gateway on
```

### 4. Configure Client Devices

On each device you want to route through the gateway, set the default gateway to your Shroud machine's IP:

```bash
# Linux client
sudo ip route replace default via 192.168.1.10  # Shroud machine's IP

# Or configure in your router's DHCP settings
```

## Configuration

### /etc/shroud/config.toml

```toml
[gateway]
# Enable gateway on startup
enabled = true

# LAN interface (auto-detected if not set)
lan_interface = "eth0"

# Which clients can use the gateway
# Options: "all", "192.168.1.0/24", or ["192.168.1.50", "192.168.1.51"]
allowed_clients = "all"

# Block forwarded traffic if VPN drops
kill_switch_forwarding = true

# Keep IP forwarding after Shroud exits
persist_ip_forward = false

# Enable IPv6 forwarding (disabled by default for leak prevention)
enable_ipv6 = false
```

### Configuration Options

| Option | Default | Description |
|--------|---------|-------------|
| `enabled` | `false` | Enable gateway on startup |
| `lan_interface` | auto | LAN interface (eth0, enp3s0, etc.) |
| `allowed_clients` | `"all"` | Who can use the gateway |
| `kill_switch_forwarding` | `true` | Block if VPN drops |
| `persist_ip_forward` | `false` | Keep forwarding after exit |
| `enable_ipv6` | `false` | Route IPv6 (leak risk) |

## CLI Commands

```bash
# Enable gateway mode
shroud gateway on

# Disable gateway mode  
shroud gateway off

# Show gateway status
shroud gateway status
```

### Status Output

```
Gateway Status
==============

Gateway:           ✓ enabled
IP Forwarding:     ✓ enabled
Forward Kill SW:   ✓ active

LAN Interface
-------------
  Interface:       eth0
  IP Address:      192.168.1.10
  Subnet:          192.168.1.0/24

VPN Interface
-------------
  Interface:       tun0
  IP Address:      10.8.0.2
```

## Client Configuration

### Option A: Per-Device Static Route

On each client:

```bash
# Linux
sudo ip route replace default via 192.168.1.10

# macOS
sudo route delete default
sudo route add default 192.168.1.10

# Windows (Admin PowerShell)
route delete 0.0.0.0
route add 0.0.0.0 mask 0.0.0.0 192.168.1.10
```

### Option B: Router DHCP Configuration

Configure your router to assign the Shroud gateway as the default gateway for specific devices via DHCP options.

### Option C: Full Network Gateway

Replace your router's upstream gateway with the Shroud machine. All network traffic will route through the VPN.

## How It Works

1. **IP Forwarding** — Linux kernel forwards packets between interfaces
2. **NAT (MASQUERADE)** — Rewrites source IP so VPN server can reply
3. **FORWARD Rules** — Control which traffic is allowed
4. **Kill Switch** — Blocks forwarded traffic if VPN drops

### Firewall Rules Applied

```
# NAT table
iptables -t nat -A POSTROUTING -o tun0 -j MASQUERADE

# FORWARD chain
iptables -A FORWARD -i eth0 -o tun0 -j ACCEPT
iptables -A FORWARD -i tun0 -o eth0 -m state --state RELATED,ESTABLISHED -j ACCEPT

# Kill switch (end of FORWARD)
iptables -A FORWARD -o eth0 -j DROP
```

## Troubleshooting

### Gateway won't enable

```bash
# Check if VPN is connected
shroud status

# VPN must be connected before gateway can enable
shroud connect your-vpn
shroud gateway on
```

### Clients can't reach internet

```bash
# Check gateway status
shroud gateway status

# Verify IP forwarding
cat /proc/sys/net/ipv4/ip_forward  # Should be 1

# Check NAT rules
sudo iptables -t nat -L POSTROUTING -n -v

# Check FORWARD rules
sudo iptables -L FORWARD -n -v
```

### Traffic leaking when VPN drops

```bash
# Verify kill switch is active
shroud gateway status

# Check for FORWARD kill switch chain
sudo iptables -L SHROUD_GATEWAY_KS -n -v
```

### IPv6 leaks

IPv6 is disabled by default in gateway mode. If you enable it, ensure your VPN supports IPv6 routing.

```toml
[gateway]
enable_ipv6 = false  # Keep false unless VPN supports IPv6
```

## Security Considerations

1. **Kill Switch** — Always keep `kill_switch_forwarding = true`
2. **IPv6** — Keep `enable_ipv6 = false` to prevent leaks
3. **Allowed Clients** — Restrict with `allowed_clients` if needed
4. **DNS** — Clients should use VPN's DNS or a trusted resolver
5. **Audit** — Run `shroud gateway status` to verify rules

## Use Cases

### Home Network VPN

Protect all household devices:
- Smart TVs
- Gaming consoles
- IoT devices
- Guest devices

### Small Office

One VPN connection for entire office:
- No per-device configuration
- Central management
- Cost savings (one VPN subscription)

### Travel Router

Raspberry Pi as portable VPN gateway:
- Protect all devices on hotel WiFi
- Consistent VPN across locations
- No device-specific apps needed

## Uninstall / Disable

```bash
# Disable gateway
shroud gateway off

# Or stop the service
sudo systemctl stop shroud

# Gateway rules are automatically cleaned up
```
