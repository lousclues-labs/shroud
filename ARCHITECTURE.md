# Shroud — System Architecture

## High-Level Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              SHROUD                                      │
│                                                                          │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐              │
│  │   System     │    │     VPN      │    │    Kill      │              │
│  │    Tray      │◄──►│  Supervisor  │◄──►│   Switch     │              │
│  │   (ksni)     │    │              │    │ (iptables)   │              │
│  └──────────────┘    └──────────────┘    └──────────────┘              │
│         │                   │                   │                        │
│         │                   │                   │                        │
│         ▼                   ▼                   ▼                        │
│  ┌──────────────────────────────────────────────────────┐              │
│  │                    Tokio Runtime                      │              │
│  └──────────────────────────────────────────────────────┘              │
│                             │                                            │
└─────────────────────────────│────────────────────────────────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          ▼                   ▼                   ▼
┌──────────────┐    ┌──────────────┐    ┌──────────────┐
│ NetworkManager│    │    D-Bus     │    │   iptables   │
│ (nmcli: OpenVPN/WireGuard) │    │   (zbus)     │    │   (pkexec)   │
└──────────────┘    └──────────────┘    └──────────────┘
```

---

## CLI Architecture

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

---

## Module Structure

```
src/
├── main.rs           # Entry point, VpnSupervisor, main event loop
├── cli/
│   ├── mod.rs        # Module exports
│   ├── args.rs       # Command-line argument parsing (clap)
│   ├── handlers.rs   # CLI command handlers
│   └── help.rs       # Custom help generation
├── logging.rs        # Structured logging setup
├── config/
│   ├── mod.rs        # Module exports
│   └── settings.rs   # Config struct, ConfigManager, TOML persistence
├── dbus/
│   ├── mod.rs        # Module exports
│   └── monitor.rs    # NmMonitor, D-Bus signal subscription
├── health/
│   ├── mod.rs        # Module exports
│   └── checker.rs    # HealthChecker, HTTP/ping connectivity tests
├── import/
│   ├── mod.rs         # Import module exports
│   ├── detector.rs    # Config type detection (WireGuard/OpenVPN)
│   ├── validator.rs   # Config validation
│   ├── importer.rs    # nmcli import wrapper + bulk import
│   └── types.rs       # Import options and JSON types
├── ipc/
│   ├── mod.rs        # Module exports
│   ├── protocol.rs   # IPC types (IpcCommand, IpcResponse)
│   ├── server.rs     # Unix Domain Socket Server
│   └── client.rs     # Unix Domain Socket Client
├── killswitch/
│   ├── mod.rs        # Module exports
│   └── firewall.rs   # KillSwitch, iptables rule generation
├── nm/
│   ├── mod.rs        # Module exports
│   ├── client.rs     # nmcli wrappers (connect, disconnect, list)
│   └── connections.rs # VPN type detection (wireguard/openvpn)
├── state/
│   ├── mod.rs        # Module exports
│   ├── machine.rs    # StateMachine, event handling, transitions
│   └── types.rs      # VpnState, Event, TransitionReason enums
├── supervisor/       
│   ├── mod.rs        # Module exports
│   ├── event_loop.rs # Main event loop logic
│   ├── handlers.rs   # Supervisor command handlers
│   └── reconnect.rs  # Reconnection strategy
└── tray/
    ├── mod.rs        # Module exports
    ├── service.rs    # VpnTray, SharedState, menu construction
    └── icons.rs      # Icon generation (colored status indicators)
```

---

## Error Handling Pattern

The application uses specific error types for each domain, leveraging the `thiserror` crate for structured error handling.

| Module | Error Type | Description |
|--------|------------|-------------|
| `config` | `ConfigError` | Configuration loading, parsing, and saving errors |
| `ipc` | `ClientError` | IPC client connection and communication errors |
| `ipc` | `ServerError` | IPC server binding and acceptance errors |
| `killswitch` | `KillSwitchError` | Firewall/iptables operation errors |
| `nm` | `NmError` | NetworkManager interaction errors |

Errors are propagated up to the `VpnSupervisor` or CLI handlers, where they are logged or displayed to the user.

---

## Data Flow

### Command Flow (User → VPN)

```
┌────────────┐     VpnCommand      ┌──────────────┐
│  Tray Menu │ ─────────────────► │  Supervisor  │
└────────────┘    (mpsc channel)   └──────────────┘
                                          │
                                          ▼
                                   ┌──────────────┐
                                   │ StateMachine │
                                   └──────────────┘
                                          │
                                          ▼
                                   ┌──────────────┐
                                   │    nmcli     │
                                   └──────────────┘

### Import Flow (Config → NetworkManager)

```
┌────────────┐     Config Path      ┌──────────────┐     nmcli import     ┌──────────────┐
│    CLI     │ ───────────────────► │ Import Module│ ───────────────────► │ NetworkManager│
│ shroud import│   (file/dir)       │ detector/    │   (wireguard/openvpn)│  connections  │
└────────────┘                       │ validator    │                      └──────────────┘
```

- Auto-detects WireGuard vs OpenVPN by extension + content
- Validates required fields before import
- Supports bulk directory imports (optional recursion)
```

### Event Flow (NetworkManager → Supervisor)

```
┌────────────────┐     NmEvent       ┌──────────────┐
│  D-Bus Monitor │ ───────────────► │  Supervisor  │
└────────────────┘   (mpsc channel)  └──────────────┘
                                           │
                                           ▼
                                    ┌──────────────┐
                                    │ StateMachine │
                                    └──────────────┘
                                           │
                                           ▼
                                    ┌──────────────┐
                                    │ SharedState  │◄──── Tray reads this
                                    └──────────────┘
```

---

## State Machine

### States

```
┌──────────────┐
│ Disconnected │◄─────────────────────────────────────┐
└──────────────┘                                      │
       │                                              │
       │ UserEnable                                   │
       ▼                                              │
┌──────────────┐                                      │
│  Connecting  │────────────────┐                     │
└──────────────┘                │                     │
       │                        │                     │
       │ NmVpnUp                │ Timeout             │
       ▼                        ▼                     │
┌──────────────┐         ┌──────────────┐             │
│  Connected   │────────►│ Reconnecting │─────────────┤
└──────────────┘         └──────────────┘             │
       │                        │                     │
       │ HealthDegraded         │ Retries exhausted   │
       ▼                        ▼                     │
┌──────────────┐         ┌──────────────┐             │
│   Degraded   │────────►│    Failed    │─────────────┘
└──────────────┘         └──────────────┘
       │                        │
       │ HealthOk               │ UserEnable
       ▼                        │
┌──────────────┐                │
│  Connected   │◄───────────────┘
└──────────────┘
```

### Transition Table

| Current State | Event | Next State | Reason |
|---------------|-------|------------|--------|
| Disconnected | UserEnable | Connecting | user_requested |
| Disconnected | NmVpnUp | Connected | external_change |
| Connecting | NmVpnUp | Connected | vpn_established |
| Connecting | Timeout | Reconnecting | retrying |
| Connecting | NmVpnDown | Reconnecting | retrying |
| Connected | HealthDegraded | Degraded | health_check_failed |
| Connected | NmVpnDown | Reconnecting | vpn_lost |
| Degraded | HealthDead | Reconnecting | health_check_dead |
| Degraded | HealthOk | Connected | vpn_reestablished |
| Degraded | NmVpnDown | Reconnecting | vpn_lost |
| Reconnecting | NmVpnUp | Connected | vpn_reestablished |
| Reconnecting | Timeout | Failed | retries_exhausted |
| Failed | UserEnable | Connecting | user_requested |
| * | UserDisable | Disconnected | user_requested |

---

## Kill Switch Architecture

### iptables Chain Structure

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
        
        # 5. DNS (based on dns_mode)
        # tunnel: (no rules - only via VPN interface)
        # localhost: ip daddr 127.0.0.0/8 udp dport 53 accept
        # any: udp dport 53 accept
        
        # 6. Local network
        ip daddr 192.168.0.0/16 accept
        ip daddr 10.0.0.0/8 accept
        ip daddr 172.16.0.0/12 accept
        
        # 7. VPN tunnel interfaces
        oifname "tun*" accept
        oifname "tap*" accept
        oifname "wg*" accept
        
        # 8. VPN server IPs (auto-detected from NM)
        ip daddr <VPN_SERVER_IP> accept
        
        # 9. Default drop with logging
        limit rate 1/second log prefix "[SHROUD-KS DROP] " drop
    }
    
    chain input {
        type filter hook input priority 0; policy accept;
        # Input is permissive - kill switch focuses on output leaks
    }
}
```

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
│  │  tokio::select! {   │  │  while let Some(    │              │
│  │    rx.recv() =>     │  │    msg) = stream    │              │
│  │    dbus_rx.recv() =>│◄─│    .next() {        │              │
│  │    poll_tick =>     │  │      tx.send(event) │              │
│  │    health_tick =>   │  │  }                  │              │
│  │  }                  │  │                     │              │
│  └─────────────────────┘  └─────────────────────┘              │
│                                                                  │
│  ┌─────────────────────┐                                        │
│  │   Tray Thread       │  (separate OS thread for ksni)        │
│  │   (std::thread)     │                                        │
│  │                     │                                        │
│  │  Reads SharedState  │                                        │
│  │  via Arc<RwLock<>>  │                                        │
│  └─────────────────────┘                                        │
└────────────────────────────────────────────────────────────────┘
```

### Synchronization Primitives

| Primitive | Purpose |
|-----------|---------|
| `mpsc::channel<VpnCommand>` | Tray → Supervisor commands |
| `mpsc::channel<NmEvent>` | D-Bus monitor → Supervisor events |
| `Arc<RwLock<SharedState>>` | Supervisor → Tray state sync |
| `Arc<Mutex<TrayHandle>>` | Tray handle for updates |

---

## File Locations

| File | Path | Purpose |
|------|------|---------|
| Config | `~/.config/shroud/config.toml` | User preferences |
| Lock | `$XDG_RUNTIME_DIR/shroud.lock` | Single instance lock |
| Logs | `~/.local/share/shroud/` | Log files (when enabled) |
| Autostart | `~/.config/autostart/shroud.desktop` | XDG autostart |

---

## Key Design Decisions

### 1. nmcli over D-Bus for Commands

**Decision**: Use nmcli subprocess calls for VPN connect/disconnect, D-Bus only for events.

**Rationale**: nmcli handles all the edge cases (secrets, prompts, plugins). D-Bus VPN control is complex and varies by plugin. Principle V: Complexity Is Debt.

### 2. Polling as Fallback

**Decision**: Poll NetworkManager state every 2 seconds even with D-Bus events.

**Rationale**: D-Bus signals can be missed (race conditions, connection drops). Polling catches desync. Belt and suspenders.

### 3. Grace Period for Disconnects

**Decision**: 5-second grace period after intentional disconnect before processing D-Bus events.

**Rationale**: Prevents false "connection dropped" detection when user deliberately disconnects.

### 4. Kill Switch Stays Enabled on VPN Drop

**Decision**: Kill switch remains active when VPN unexpectedly disconnects.

**Rationale**: This IS the kill switch behavior. Traffic should be blocked until VPN reconnects. User can manually disable if needed.

### 5. Single Binary Architecture

**Decision**: No daemon, no IPC, single process.

**Rationale**: Principle VIII. Simpler deployment, simpler debugging, fewer failure modes. The tray IS the application.

### 6. Atomic Config Writes

**Decision**: Write config to temp file, then rename.

**Rationale**: Prevents corruption if crash occurs during write. Config is precious user data.

### 7. VPN Server IP Auto-Detection

**Decision**: Parse all VPN connections from NetworkManager to whitelist server IPs in kill switch.

**Rationale**: Allows VPN to establish connection even when kill switch is enabled. User doesn't need to manually configure server IPs.
