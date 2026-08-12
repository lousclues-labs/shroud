# Kill Switch

The kill switch does one thing: when your VPN goes down, so does your traffic.

No leaks. No exceptions. No "oops, my real IP slipped out while the tunnel was reconnecting."

---

## How It Works

```
┌─────────────────────────────────────────────────────────────────┐
│                     KILL SWITCH ENABLED                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│   ALLOWED                         BLOCKED                       │
│   ───────                         ───────                       │
│                                                                 │
│   ✓ Loopback (127.0.0.1)          ✗ Everything else             │
│   ✓ VPN tunnel (tun0, wg0)                                      │
│   ✓ VPN server IP (to connect)                                  │
│   ✓ Local network (192.168.x.x)                                 │
│   ✓ DHCP (so you can get an IP)                                 │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

When you enable the kill switch, VPN Shroud creates an iptables chain called `SHROUD_KILLSWITCH`. This chain gets inserted into your OUTPUT chain, and it drops anything that doesn't match our allowlist.

Simple. Auditable. Effective.

---

## Quick Start

```bash
shroud ks on        # Enable
shroud ks off       # Disable
shroud ks status    # Check the current state
shroud ks toggle    # Flip it
```

That's the whole interface.

---

## What Gets Allowed

When the kill switch is active, these paths stay open:

| Traffic Type | Why It's Allowed |
|--------------|------------------|
| Loopback (127.0.0.1, ::1) | Local services need to talk to each other |
| VPN tunnel interfaces (tun*, wg*, tap*) | This is the whole point |
| VPN server IPs | You need to reach the server to connect |
| Established connections | Don't break existing tunnel traffic |
| LAN (192.168.0.0/16, 10.0.0.0/8, 172.16.0.0/12) | You probably want to print and access local shares |
| DHCP (port 67/68) | You need an IP address |

Everything else? Dropped. Logged if you enable debug mode.

---

## DNS Leak Protection

A kill switch that lets DNS leak is barely a kill switch at all.

VPN Shroud has four DNS modes, controlled by `dns_mode` in your config:

| Mode | What Happens | When To Use It |
|------|--------------|----------------|
| `tunnel` | DNS only through VPN interface | Default. Recommended. |
| `strict` | Tunnel + block DoH/DoT | When you're paranoid (in a good way) |
| `localhost` | Allow 127.0.0.1, ::1, 127.0.0.53 | Running a local DNS resolver |
| `any` | No DNS restrictions | Legacy compatibility. Not recommended. |

### Blocking DNS-over-HTTPS

Modern browsers love to bypass your DNS settings. Firefox, Chrome, Edge -- they all have DNS-over-HTTPS baked in, which can leak queries even with a kill switch.

When `block_doh = true` (the default), VPN Shroud blocks connections to known DoH provider IPs on port 443. This includes:

- Cloudflare (1.1.1.1, 1.0.0.1)
- Google (8.8.8.8, 8.8.4.4)
- Quad9 (9.9.9.9)
- And others

You can add your own IPs to block:

```toml
custom_doh_blocklist = ["208.67.222.222", "208.67.220.220"]
```

---

## IPv6 Leak Protection

Most VPNs don't tunnel IPv6. If your system has IPv6 enabled, traffic can leak around the tunnel on the v6 path.

VPN Shroud handles this with the `ipv6_mode` setting:

| Mode | What Happens | When To Use It |
|------|--------------|----------------|
| `block` | Drop all IPv6 except loopback | Default. Safe choice. |
| `tunnel` | IPv6 only through VPN interface | Your VPN actually tunnels IPv6 |
| `off` | No IPv6 restrictions | You know what you're doing |

---

## The Rules Under The Hood

If you want to see exactly what VPN Shroud is doing:

```bash
# View the kill switch chain
sudo iptables -L SHROUD_KILLSWITCH -n -v

# View the jump rule in OUTPUT
sudo iptables -L OUTPUT -n -v | grep SHROUD

# IPv6 version
sudo ip6tables -L SHROUD_KILLSWITCH -n -v
```

Everything is auditable. No magic. No obscurity.

---

## Privileges

The kill switch needs root to modify iptables. VPN Shroud uses a sudoers rule for passwordless operation:

```bash
# Install automatically
./setup.sh --install-sudoers

# Verify it works
sudo -n iptables -L -n
```

The rule only grants access to `iptables`, `ip6tables`, and `nft`. Nothing else.

Remove anytime:
```bash
sudo rm /etc/sudoers.d/shroud
```

### Scoped wrapper (recommended; required on sudo-rs)

For tighter privilege separation — and for **sudo-rs** systems (Ubuntu
25.10+/26.04) where wildcard sudoers rules are rejected — install the validating
wrapper instead:

```bash
sudo ./assets/fw-wrapper/install.sh
```

The passwordless grant then points only at
`/usr/local/lib/shroud/shroud-{iptables,ip6tables,nft}`, each of which validates
its arguments and permits only operations on the `SHROUD_KILLSWITCH` /
`SHROUD_BOOT_KS` chains and the `shroud_killswitch` nftables table. Even
`nft -f -` payloads are parsed to reject any foreign table or `flush ruleset`,
so the grant cannot be repurposed to rewrite the wider firewall.

## Connect-first ordering (`pre_connect`)

By default the kill switch is enabled **before** connecting (fail-closed), which
requires a whitelistable server IP. Hostname-only endpoints (e.g. NordVPN
`*.nordvpn.com`) can't be pre-whitelisted, so pre-enabling a closed kill switch
would block the DNS needed to (re)connect.

Shroud handles this automatically: when the VPN you are connecting to has a
hostname-only endpoint, it falls back to **connect-first** ordering for that
connection even while `pre_connect = true` — the kill switch is suspended during
the handshake and re-enabled immediately after a successful connect (the live
tunnel is kept up by the connection-tracking `ESTABLISHED` rule). IP endpoints
keep strict fail-closed ordering, so there is no leak window when the server IP
*can* be whitelisted.

Set `pre_connect = false` under `[killswitch]` to force connect-first ordering
for **every** connection, regardless of endpoint type:

```toml
[killswitch]
pre_connect = false
```

Trade-off: connect-first has a brief unprotected window during (re)connect, and
is fail-open (the kill switch stays suspended, with a logged warning) if a
connect attempt fails.

Remove anytime:
```bash
sudo rm /etc/sudoers.d/shroud
```

---

## Recovery

### If VPN Shroud Crashes

VPN Shroud cleans up stale rules on startup. Just start it again:

```bash
shroud
```

### Manual Cleanup

If you need to clean up manually:

```bash
# Remove the jump rule
sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH

# Flush and delete the chain
sudo iptables -F SHROUD_KILLSWITCH
sudo iptables -X SHROUD_KILLSWITCH

# Same for IPv6
sudo ip6tables -D OUTPUT -j SHROUD_KILLSWITCH
sudo ip6tables -F SHROUD_KILLSWITCH
sudo ip6tables -X SHROUD_KILLSWITCH
```

If you were using the nftables backend:

```bash
sudo nft delete table inet shroud_killswitch
```

---

## Configuration Reference

All kill switch options in `~/.config/shroud/config.toml`:

```toml
# Enable kill switch (blocks non-VPN traffic)
kill_switch_enabled = false

# DNS leak protection mode
dns_mode = "tunnel"

# Block DNS-over-HTTPS
block_doh = true

# Additional DoH IPs to block
custom_doh_blocklist = []

# IPv6 leak protection
ipv6_mode = "block"

[killswitch]
# Allow local network traffic
allow_lan = true
```

---

## The Philosophy

A kill switch should be simple enough to trust.

If you can't understand what it's doing, you'll eventually disable it. We'd rather you keep it on.

That's why every rule is:
- **Auditable** -- `iptables -L` shows you everything
- **Explainable** -- this doc tells you why each rule exists
- **Removable** -- manual cleanup commands if something goes wrong

Your security shouldn't depend on faith. It should depend on clarity.
