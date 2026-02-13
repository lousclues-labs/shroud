# Configuration

Shroud stores its config in `~/.config/shroud/config.toml`. It's just TOML. You can edit it with any text editor.

---

## The Defaults

When you first run Shroud, it creates a config with sensible defaults:

```toml
version = 1
auto_reconnect = true
kill_switch_enabled = false
dns_mode = "tunnel"
block_doh = true
ipv6_mode = "block"
health_check_interval_secs = 30
health_degraded_threshold_ms = 2000
max_reconnect_attempts = 10

[killswitch]
allow_lan = true

[headless]
auto_connect = false
kill_switch_on_boot = true
require_kill_switch = true
persist_kill_switch = false
max_reconnect_attempts = 0
reconnect_delay_secs = 5
```

Most people won't need to change anything. But if you do, here's what everything means.

---

## Core Options

### `version`

**Don't touch this.** It's for config migration when we add new options.

### `auto_reconnect`

| Type | Default |
|------|---------|
| boolean | `true` |

When the VPN drops, try to reconnect automatically.

### `kill_switch_enabled`

| Type | Default |
|------|---------|
| boolean | `false` |

Block all traffic when VPN is disconnected. See [Kill Switch](KILLSWITCH.md).

### `last_server`

| Type | Default |
|------|---------|
| string | none |

The last VPN you connected to. Used for quick reconnect.

---

## Health Check Options

Shroud monitors your VPN connection and detects problems before you notice them.

### `health_check_interval_secs`

| Type | Default |
|------|---------|
| integer | `30` |

How often to check if the VPN is healthy (seconds). Set to `0` to disable.

### `health_degraded_threshold_ms`

| Type | Default |
|------|---------|
| integer | `2000` |

If latency exceeds this (milliseconds), connection is marked "degraded."

### `health_check_endpoints`

| Type | Default |
|------|---------|
| list of strings | `[]` (uses built-in defaults) |

Custom URLs to check for VPN health. If empty, Shroud uses its built-in endpoints (Cloudflare, ifconfig.me, ipify). Each endpoint must return HTTP 2xx/3xx to be considered healthy.

```toml
health_check_endpoints = [
    "https://your-company.com/health",
    "https://1.1.1.1/cdn-cgi/trace",
]
```

### `max_reconnect_attempts`

| Type | Default |
|------|---------|
| integer | `10` |

How many times to try reconnecting before giving up. Set to `0` for infinite retries.

---

## DNS Leak Protection

DNS leaks are real. We take them seriously.

### `dns_mode`

| Type | Default |
|------|---------|
| string | `"tunnel"` |

How DNS requests are handled:

| Mode | What Happens | When To Use |
|------|--------------|-------------|
| `tunnel` | DNS only through VPN interface | Default. Recommended. |
| `strict` | Tunnel + block DoH/DoT | Maximum paranoia |
| `localhost` | Allow 127.0.0.1, ::1, 127.0.0.53 | Local DNS resolver |
| `any` | No restrictions | Compatibility. Not recommended. |

### `block_doh`

| Type | Default |
|------|---------|
| boolean | `true` |

Block DNS-over-HTTPS to known providers (Cloudflare, Google, etc). Browsers love to bypass your DNS settings — this stops them.

### `custom_doh_blocklist`

| Type | Default |
|------|---------|
| array | `[]` |

Additional IPs to block on port 443:

```toml
custom_doh_blocklist = ["208.67.222.222", "208.67.220.220"]
```

---

## IPv6 Leak Protection

Most VPNs don't tunnel IPv6. That means your real IPv6 address can leak.

### `ipv6_mode`

| Type | Default |
|------|---------|
| string | `"block"` |

How IPv6 is handled:

| Mode | What Happens | When To Use |
|------|--------------|-------------|
| `block` | Drop all IPv6 except loopback | Default. Safe. |
| `tunnel` | IPv6 only through VPN | Your VPN actually tunnels IPv6 |
| `off` | No restrictions | You know what you're doing |

---

## Kill Switch Options

### `[killswitch]` Section

```toml
[killswitch]
allow_lan = true
```

### `allow_lan`

| Type | Default |
|------|---------|
| boolean | `true` |

Allow traffic to local network (192.168.x.x, 10.x.x.x, 172.16.x.x). You probably want this so you can still print and access local shares.

---

## Headless Options

For servers and headless systems. See [Headless Mode](HEADLESS.md).

### `[headless]` Section

```toml
[headless]
auto_connect = true
startup_server = "mullvad-us1"
kill_switch_on_boot = true
require_kill_switch = true
persist_kill_switch = false
max_reconnect_attempts = 0
reconnect_delay_secs = 5
```

| Option | Type | Default | What It Does |
|--------|------|---------|--------------|
| `auto_connect` | bool | `false` | Connect VPN on startup |
| `startup_server` | string | none | Which VPN to connect to |
| `kill_switch_on_boot` | bool | `true` | Block traffic until VPN connects |
| `require_kill_switch` | bool | `true` | Fail if kill switch can't enable |
| `persist_kill_switch` | bool | `false` | Keep kill switch after Shroud exits |
| `max_reconnect_attempts` | int | `0` | Retry limit (0 = infinite) |
| `reconnect_delay_secs` | int | `5` | Initial retry delay |

---

## Config Location

| Mode | Path |
|------|------|
| Desktop | `~/.config/shroud/config.toml` |
| Headless (root) | `/etc/shroud/config.toml` |

---

## Reloading

After editing the config:

```bash
shroud reload
```

Or restart:

```bash
shroud restart
```

---

## Reset to Defaults

Remove the config and let Shroud recreate it:

```bash
rm ~/.config/shroud/config.toml
shroud
```

---

## Full Example

Here's a fully annotated config:

```toml
# Config version (don't touch)
version = 1

# Reconnect automatically when VPN drops
auto_reconnect = true

# Last connected VPN (managed by Shroud)
last_server = "mullvad-us1"

# Check VPN health every 30 seconds
health_check_interval_secs = 30

# Warn if latency exceeds 2 seconds
health_degraded_threshold_ms = 2000

# Try reconnecting 10 times before giving up
max_reconnect_attempts = 10

# Block traffic when VPN is down
kill_switch_enabled = true

# DNS only through VPN
dns_mode = "tunnel"

# Block DNS-over-HTTPS
block_doh = true

# No extra DoH servers to block
custom_doh_blocklist = []

# Block IPv6 to prevent leaks
ipv6_mode = "block"

[killswitch]
# Keep local network accessible
allow_lan = true

[headless]
# Don't auto-connect in headless mode (I'll connect manually)
auto_connect = false
kill_switch_on_boot = true
require_kill_switch = true
persist_kill_switch = false
max_reconnect_attempts = 0
reconnect_delay_secs = 5
```

---

## The Philosophy

Configuration should be simple enough to understand at a glance.

If you need to read the docs to figure out what `dns_mode = "tunnel"` means, something's wrong. The names should be obvious.

We use TOML because it's human-readable. No YAML indentation nightmares. No JSON comma hunting.

Edit with confidence.
