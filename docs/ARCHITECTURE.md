# Architecture

How Shroud is built. Not the marketing version — the actual structure.

---

## The Big Picture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              SHROUD                                      │
│                                                                          │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐              │
│  │   System     │    │     VPN      │    │    Kill      │              │
│  │    Tray      │◄──►│  Supervisor  │◄──►│   Switch     │              │
│  │   (ksni)     │    │              │    │  (iptables)  │              │
│  └──────────────┘    └──────────────┘    └──────────────┘              │
│         │                   │                   │                        │
│         └───────────────────┼───────────────────┘                        │
│                             │                                            │
│                    ┌────────────────────┐                                │
│                    │   Tokio Runtime    │                                │
│                    └────────────────────┘                                │
│                             │                                            │
└─────────────────────────────│────────────────────────────────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          ▼                   ▼                   ▼
┌──────────────┐    ┌──────────────┐    ┌──────────────┐
│ NetworkManager│    │    D-Bus     │    │   iptables   │
│   (nmcli)     │    │   (zbus)     │    │    (sudo)    │
└──────────────┘    └──────────────┘    └──────────────┘
```

Shroud doesn't replace anything. It sits between you and the tools that actually do the work:
- **NetworkManager** handles VPN connections
- **iptables/nftables** handles firewall rules
- **D-Bus** handles system events

We're the glue. And the armor.

---

## One Binary, Two Modes

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
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

Same binary. If you run `shroud` with no arguments, you get the daemon with the tray icon. If you run `shroud <command>`, you get a one-shot client that talks to the running daemon.

This is Principle VIII: One Binary, One Purpose.

---

## Module Structure

```
src/
├── main.rs              # Entry point, mode detection
├── logging.rs           # Structured logging
├── mode.rs              # Desktop vs headless detection
├── autostart.rs         # XDG autostart handling
│
├── cli/                 # Command-line interface
│   ├── args.rs          # Argument parsing
│   ├── handlers.rs      # Command execution
│   ├── help.rs          # Help text
│   ├── import.rs        # Config import
│   ├── install.rs       # Installation helpers
│   └── validation.rs    # Input validation
│
├── config/              # Configuration
│   └── settings.rs      # Config struct, TOML persistence
│
├── daemon/              # Single instance
│   └── lock.rs          # Lock file handling
│
├── dbus/                # System events
│   └── monitor.rs       # NmMonitor, D-Bus signals
│
├── headless/            # Server mode
│   ├── runtime.rs       # Headless runtime
│   └── systemd.rs       # Systemd integration
│
├── health/              # Connection monitoring
│   └── checker.rs       # Health checks
│
├── import/              # VPN config import
│   ├── detector.rs      # WireGuard/OpenVPN detection
│   ├── importer.rs      # nmcli import wrapper
│   ├── types.rs         # Import types
│   └── validator.rs     # Config validation
│
├── ipc/                 # Inter-process communication
│   ├── client.rs        # Unix socket client
│   ├── protocol.rs      # Command/response types
│   └── server.rs        # Unix socket server
│
├── killswitch/          # Firewall rules
│   ├── boot.rs          # Boot-time kill switch
│   ├── cleanup.rs       # Rule cleanup
│   ├── firewall.rs      # iptables/nftables rules
│   ├── paths.rs         # Binary detection
│   └── sudo_check.rs    # Privilege verification
│
├── nm/                  # NetworkManager
│   ├── client.rs        # nmcli wrappers
│   └── connections.rs   # Connection handling
│
├── state/               # State machine
│   ├── machine.rs       # Transitions
│   └── types.rs         # VpnState, Event enums
│
├── supervisor/          # Core event loop
│   ├── event_loop.rs    # Main loop
│   ├── handlers.rs      # Command handlers
│   ├── reconnect.rs     # Reconnection logic
│   └── state_sync.rs    # State synchronization
│
└── tray/                # System tray
    ├── icons.rs         # Icon generation
    └── service.rs       # Tray menu, ksni integration
```

---

## The State Machine

This is the source of truth. If the state machine says you're connected, you're connected. If it says you're disconnected, you're disconnected. No guessing.

```
    Disconnected ──────► Connecting ──────► Connected
         ▲                    │                 │
         │                    │                 │
         │                    ▼                 ▼
         │                 Failed           Degraded
         │                    │                 │
         │                    │                 │
         └────────────────────┴────► Reconnecting
                                          │
                                          ▼
                                      Connected
```

### Transitions

| From | Event | To | Why |
|------|-------|-----|-----|
| Disconnected | UserEnable | Connecting | User clicked connect |
| Disconnected | NmVpnUp | Connected | External change (nm-applet, etc) |
| Connecting | NmVpnUp | Connected | Connection established |
| Connecting | Timeout | Reconnecting | Took too long |
| Connecting | NmVpnDown | Reconnecting | Failed, try again |
| Connected | HealthDegraded | Degraded | High latency or packet loss |
| Connected | NmVpnDown | Reconnecting | Connection dropped |
| Degraded | HealthDead | Reconnecting | Connection gone |
| Degraded | HealthOk | Connected | Recovered |
| Reconnecting | NmVpnUp | Connected | Back online |
| Reconnecting | Timeout | Failed | Gave up |
| Failed | UserEnable | Connecting | User retry |
| Any | UserDisable | Disconnected | User clicked disconnect |

Every transition is logged. Ambiguity is a bug.

---

## Kill Switch Rules

The kill switch creates an iptables chain that drops everything except what we explicitly allow:

```
Chain SHROUD_KILLSWITCH (policy DROP)

    # 1. Loopback - always allowed
    -o lo -j ACCEPT

    # 2. Established connections
    -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT

    # 3. VPN tunnel interfaces
    -o tun+ -j ACCEPT
    -o tap+ -j ACCEPT
    -o wg+ -j ACCEPT

    # 4. DHCP
    -p udp --dport 67 -j ACCEPT
    -p udp --sport 68 -j ACCEPT

    # 5. Local network
    -d 192.168.0.0/16 -j ACCEPT
    -d 10.0.0.0/8 -j ACCEPT
    -d 172.16.0.0/12 -j ACCEPT

    # 6. VPN server IPs (auto-detected)
    -d <VPN_SERVER_IP> -j ACCEPT

    # 7. DNS rules (based on dns_mode)
    # ...

    # 8. Drop everything else
    -j DROP
```

Shroud prefers iptables, falls back to nftables if iptables has issues, and falls back to iptables-legacy if nftables fails. Binary paths are detected at runtime.

---

## Concurrency Model

```
┌────────────────────────────────────────────────────────────────┐
│                        Tokio Runtime                            │
│                                                                  │
│  ┌─────────────────────┐  ┌─────────────────────┐              │
│  │   Main Task         │  │   D-Bus Monitor     │              │
│  │   (VpnSupervisor)   │  │   (NmMonitor)       │              │
│  │                     │  │                     │              │
│  │  tokio::select! {   │  │  Watches NM signals │              │
│  │    command_rx =>    │◄─│  Sends events       │              │
│  │    dbus_rx =>       │  │                     │              │
│  │    poll_tick =>     │  └─────────────────────┘              │
│  │    health_tick =>   │                                        │
│  │  }                  │                                        │
│  └─────────────────────┘                                        │
│                                                                  │
│  ┌─────────────────────┐                                        │
│  │   Tray Thread       │  (std::thread, not tokio)             │
│  │                     │                                        │
│  │  Reads SharedState  │                                        │
│  │  via Arc<RwLock<>>  │                                        │
│  └─────────────────────┘                                        │
└────────────────────────────────────────────────────────────────┘
```

The tray runs in a separate OS thread because ksni requires it. Communication happens through channels and shared state protected by locks.

---

## Data Flow

### Commands: User → VPN

```
User clicks "Connect"
        │
        ▼
┌───────────────┐     VpnCommand::Connect     ┌──────────────┐
│   Tray Menu   │ ────────────────────────────►│  Supervisor  │
└───────────────┘        (mpsc channel)        └──────────────┘
                                                      │
                                                      ▼
                                               ┌──────────────┐
                                               │ StateMachine │
                                               └──────────────┘
                                                      │
                                                      ▼
                                               ┌──────────────┐
                                               │  nm::connect │
                                               └──────────────┘
                                                      │
                                                      ▼
                                               ┌──────────────┐
                                               │   nmcli      │
                                               └──────────────┘
```

### Events: NetworkManager → Supervisor

```
NM changes VPN state
        │
        ▼
┌───────────────┐        NmEvent::VpnUp        ┌──────────────┐
│   D-Bus       │ ────────────────────────────►│  Supervisor  │
│   Monitor     │        (mpsc channel)        └──────────────┘
└───────────────┘                                     │
                                                      ▼
                                               ┌──────────────┐
                                               │ StateMachine │
                                               └──────────────┘
                                                      │
                                                      ▼
                                               ┌──────────────┐
                                               │ SharedState  │◄── Tray reads this
                                               └──────────────┘
```

---

## File Locations

| Purpose | Path |
|---------|------|
| Config | `~/.config/shroud/config.toml` |
| Lock file | `$XDG_RUNTIME_DIR/shroud.lock` |
| Socket | `$XDG_RUNTIME_DIR/shroud.sock` |
| Logs | `~/.local/share/shroud/debug.log` |
| Autostart | `~/.config/autostart/shroud.desktop` |

---

## Design Decisions

These aren't accidents. They're choices.

### nmcli over D-Bus for commands

We use nmcli subprocess calls for VPN connect/disconnect, D-Bus only for events.

**Why:** nmcli handles all the edge cases — secrets, prompts, plugins. D-Bus VPN control is complex and varies by plugin. Principle V: Complexity Is Debt.

### Polling as fallback

We poll NetworkManager every 2 seconds even with D-Bus events.

**Why:** D-Bus signals can be missed (race conditions, connection drops). Polling catches desync. Belt and suspenders.

### Kill switch stays on when VPN drops

The kill switch remains active when VPN unexpectedly disconnects.

**Why:** That's the whole point. Traffic should be blocked until VPN reconnects. User can manually disable if needed.

### Single binary

No daemon-client split. One process.

**Why:** Principle VIII. Simpler deployment, simpler debugging, fewer failure modes. The tray IS the application.

### Atomic config writes

Write config to temp file, then rename.

**Why:** Prevents corruption if crash occurs during write. Config is user data.

### VPN server IP auto-detection

Parse VPN connections from NetworkManager to whitelist server IPs in kill switch.

**Why:** VPN needs to reach its server even when kill switch is on. User shouldn't have to manually configure this.

---

## The Philosophy

Architecture should be boring. Predictable. Understandable.

If you can't trace a bug from symptom to cause by reading the code, the architecture has failed. Every component should do one thing. Every data flow should be obvious.

We're not building a cathedral. We're building a lock shroud. It should be simple enough to trust.
