# Shroud

[![CI](https://github.com/loujr/shroud/actions/workflows/ci.yml/badge.svg)](https://github.com/loujr/shroud/actions/workflows/ci.yml)
[![Security Audit](https://github.com/loujr/shroud/actions/workflows/scheduled.yml/badge.svg)](https://github.com/loujr/shroud/actions/workflows/scheduled.yml)
[![Latest Release](https://img.shields.io/github/v/release/loujr/shroud?include_prereleases&sort=semver&label=release)](https://github.com/loujr/shroud/releases/latest)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

**A provider-agnostic VPN connection manager for Linux.**

A **lock shroud** is the protective metal casing around a padlock's shackle. It doesn't replace the lock. It protects the lock from attack.

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

That's what Shroud does:

| Lock Shroud | Shroud (This Tool) |
|-------------|-------------------|
| Wraps the lock | Wraps NetworkManager |
| Protects the mechanism | Kill switch protects against leaks |
| Doesn't replace anything | Works alongside your existing tools |
| Hardens against attack | Hardens against failures and stale state |

The name works on three levels:
1. **Concealment** — A VPN shrouds your traffic
2. **Hardware** — Protective armor around the lock
3. **Architecture** — We wrap existing tools, we don't replace them

---

## The Philosophy

Most VPN tools want to own your system. They install kernel modules, replace your DNS, spawn seventeen daemons, and phone home to tell someone you're using them.

Shroud doesn't do any of that.

**We wrap, we don't replace.** NetworkManager already knows how to connect to VPNs. OpenVPN and WireGuard already work. We're not here to reinvent the wheel — we're here to put armor around it.

**We fail loud, recover quiet.** When something breaks, you'll know. When it heals, you won't need to lift a finger.

**We leave no trace.** When Shroud stops, your system is exactly as it was. No orphaned firewall rules. No zombie processes. No "please run this script to unfuck your networking."

**We respect your privacy.** No telemetry. No analytics. No phoning home. If you want to run Shroud in a bunker with nothing but a VPN tunnel to the outside world, that's your right.

Read the full [Principles](docs/PRINCIPLES.md) if you want to understand what we're about.

---

## What You Get

```
┌──────────────────────────────────────────────────────────────────┐
│                                                                  │
│   ✓ Kill switch that actually works                              │
│     └─ Traffic blocked when VPN drops. No leaks.                 │
│                                                                  │
│   ✓ Auto-reconnect that doesn't nag                              │
│     └─ Falls, gets back up, doesn't complain about it.           │
│                                                                  │
│   ✓ LAN access while connected                                   │
│     └─ Print, share files, access local devices. VPN stays up.   │
│                                                                  │
│   ✓ System tray that stays out of your way                       │
│     └─ Click to connect. Click to disconnect. That's it.         │
│                                                                  │
│   ✓ Works with any VPN provider                                  │
│     └─ Mullvad, Nord, Proton, self-hosted, corporate. All good.  │
│                                                                  │
│   ✓ Headless mode for servers                                    │
│     └─ No GUI? No problem. Systemd integration included.         │
│                                                                  │
│   ✓ Single binary, single purpose                                │
│     └─ One executable. CLI and daemon in one.                    │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

---

## Why Shroud is Fast
- One lean Rust binary — no Electron, no heavyweight GUI stack.
- No provider handshake — we talk straight to NetworkManager with your OpenVPN/WireGuard profiles.
- Minimal background daemons — a single supervisor, no telemetry or auto-updaters.
- Tight event loop — async Tokio + formal state machine keep connect/disconnect on the hot path.
- In-process kill switch — iptables/nft rules applied/cleaned without extra helpers.

Boot-to-VPN in ~2–4s after network is ready (with `auto_connect = true` + headless/systemd autostart).

---

## The Interface

A system tray icon that stays out of your way. Left-click for the menu. That's it.

<img width="313" alt="Shroud system tray menu" src="https://github.com/user-attachments/assets/7b00be8f-d97f-4b27-aabb-d4e56bb42a81" />

---

## Quick Start

```bash
git clone https://github.com/loujr/shroud.git
cd shroud
./setup.sh
```

That's it. The script handles dependencies, builds the binary, installs it, sets up your desktop entry, and configures shell completions.

Then import a VPN and go:

```bash
shroud import ~/my-vpn.ovpn
shroud connect my-vpn
shroud ks on
```

You're protected.

---

## The Basics

### Starting Shroud

```bash
shroud                    # Start with system tray
shroud --headless         # Start without GUI (for servers)
shroud autostart on       # Launch on login
```

### Connecting

```bash
shroud list               # See your VPNs
shroud connect ireland-42 # Connect
shroud disconnect         # Disconnect
shroud switch us-west-2   # Atomic switch to different VPN
shroud status             # What's happening?
```

### The Kill Switch

The kill switch blocks all traffic when your VPN drops. No exceptions. No leaks.

```bash
shroud ks on              # Enable
shroud ks off             # Disable
shroud ks status          # Check
```

When enabled, only these paths are allowed:
- Loopback (localhost)
- Your VPN tunnel
- Your local network (so you can still print)
- DHCP (so you can still get an IP)

Everything else gets dropped. DNS goes through the tunnel or nowhere.

### Importing Configs

Bring your own configs. We don't care who your provider is.

```bash
shroud import ~/mullvad-us1.conf              # WireGuard
shroud import ~/corporate.ovpn --name "Work"  # OpenVPN with custom name
shroud import ~/vpn-configs/                   # Whole directory
shroud import ~/vpn.conf --connect             # Import and connect immediately
```

---

## Documentation

| Document | What's Inside |
|----------|---------------|
| [Installation](docs/INSTALL.md) | Dependencies, building, setup |
| [CLI Reference](docs/CLI.md) | Every command, every flag |
| [Configuration](docs/CONFIGURATION.md) | The config file explained |
| [Kill Switch](docs/KILLSWITCH.md) | How the firewall rules work |
| [Headless Mode](docs/HEADLESS.md) | Running on servers |
| [Troubleshooting](docs/TROUBLESHOOTING.md) | When things go wrong |
| [Architecture](docs/ARCHITECTURE.md) | How it's built |
| [Principles](docs/PRINCIPLES.md) | Why it's built this way |
| [Contributing](CONTRIBUTING.md) | How to help |

---

## Configuration

Shroud keeps its config in `~/.config/shroud/config.toml`. Here's what matters:

```toml
auto_reconnect = true              # Get back up when you fall
kill_switch_enabled = false        # Flip to true for always-on protection
dns_mode = "tunnel"                # DNS through VPN only
ipv6_mode = "block"                # Block IPv6 leaks
```

See [Configuration](docs/CONFIGURATION.md) for the full reference.

---

## The State Machine

Shroud knows exactly what state it's in at all times. No guessing. No "it says connected but nothing works."

```
    Disconnected ──────► Connecting ──────► Connected
         ▲                    │                 │
         │                    │                 │
         │                    ▼                 ▼
         │                 Failed           Degraded
         │                    │                 │
         │                    │                 │
         └────────────────────┴────► Reconnecting
```

Every transition is logged. Every state is real. If Shroud says you're connected, you're connected.

---

## Troubleshooting

### Tray icon missing?

Your desktop needs StatusNotifierItem support. GNOME users need the [AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/).

### Kill switch won't enable?

```bash
shroud doctor              # Run diagnostics
./setup.sh --install-sudoers  # Install the sudoers rule
```

### Stuck with no internet?

If Shroud crashes with the kill switch on:

```bash
shroud ks off              # Try this first

# If Shroud isn't responding:
sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH
sudo iptables -F SHROUD_KILLSWITCH
sudo iptables -X SHROUD_KILLSWITCH
```

### Debug mode

```bash
shroud debug on            # Start logging everything
shroud debug tail          # Watch the logs
```

See [Troubleshooting](docs/TROUBLESHOOTING.md) for more.

---

## Contributing

We'd love your help. But first, read the [Principles](docs/PRINCIPLES.md). Every contribution should align with them.

The short version:
- Wrap, don't replace
- Fail loud, recover quiet  
- Leave no trace
- Keep it simple

See [Contributing](CONTRIBUTING.md) for the full guide.

---

## Requirements

- Linux (Arch, Debian, Ubuntu, Fedora, etc.)
- NetworkManager with OpenVPN and/or WireGuard plugins
- iptables or nftables
- A VPN config file

That's really it.

---

## License

Shroud is a lousclues project, dual-licensed:

- **Source Code** — [GPL-3.0-or-later](LICENSE) for open source use, modification, and distribution under GPL terms.
- **Commercial** — [Commercial license](LICENSE-COMMERCIAL.md) available for proprietary use cases.
- **Documentation** — [CC BY 4.0](LICENSE-DOCS.md) for all project documentation.
- **Third-Party** — [Dependency licenses](THIRD-PARTY-LICENSES) for all included libraries.

"Shroud" and "lousclues" are trademarks of loujr. See [TRADEMARKS.md](TRADEMARKS.md) for usage guidelines.

By contributing, you agree to the [Contributor License](CONTRIBUTOR-LICENSE.md).

See [LICENSE](LICENSE) for the full GPL text.

---

*Shroud: Wrap your VPN in armor, not bloatware.*

*We protect. We recover. We disappear.*

*Your traffic is your business.*
