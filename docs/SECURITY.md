# Security Policy

You found a vulnerability. Thank you. Here's what to do.

---

## Reporting

**Don't open a public issue.** Security vulnerabilities need private disclosure.

### Preferred: GitHub Security Advisories

1. Go to the Security tab on the repository
2. Click "Report a vulnerability"
3. Fill in the details

### Alternative: Direct Contact

If advisories aren't available, contact the maintainers directly through GitHub.

---

## What to Include

The more detail, the faster the fix:

- **Description** -- what's the vulnerability?
- **Impact** -- what can an attacker do?
- **Reproduction** -- step-by-step instructions
- **Affected versions** -- which versions are vulnerable?
- **Proof of concept** -- code or commands that demonstrate it (if safe to share)

---

## Response Timeline

| Step | Target |
|------|--------|
| Acknowledge receipt | 72 hours |
| Initial assessment | 1 week |
| Fix or mitigation plan | 2 weeks |
| Public disclosure | After fix is available |

Expect updates on progress.

---

## Supported Versions

Security fixes are provided for the latest released version.

Older versions don't receive updates. If you're on an old version and a security issue is found, update.

---

## Dependency Audits

We use `cargo audit` to check dependencies against the RustSec Advisory Database:

```bash
./scripts/audit.sh

# Or via CLI
shroud audit
```

This runs in CI. If a vulnerable dependency is found:

1. The actual risk is assessed (not all advisories apply to all use cases)
2. The risk is documented if an update isn't immediately possible
3. A fix is prioritized in the next release

---

## Dependency Justification

Every dependency is a liability. Every crate you pull in is code you didn't write, can't fully audit, and have to trust. Shroud ships 19 direct dependencies. Here's why each one exists, what it does, and what happens if it's compromised.

### Runtime

| Crate | What It Does in Shroud | Why It's Here | Compromise Impact |
|-------|------------------------|---------------|-------------------|
| `tokio` | Drives the supervisor event loop, async IPC server, signal handling, reconnect timers, and process spawning | Shroud is async. Writing a multi-threaded async runtime from scratch is not an option. | Full control of the event loop. An attacker could suppress reconnects, delay kill switch activation, or drop IPC commands silently. High risk. |
| `async-trait` | Enables async methods on the NetworkManager client trait so the mock can swap in during tests | Rust doesn't have native async trait support in stable yet. This is the standard workaround. | Code generation at compile time only. No runtime attack surface. |
| `scopeguard` | Ensures the reconnect guard flag is cleared even if the reconnect path panics | The alternative is manual cleanup in every error path. One missed path and the guard stays locked forever. | Could skip cleanup logic. Limited blast radius -- affects reconnect guard only. |

### System Integration

| Crate | What It Does in Shroud | Why It's Here | Compromise Impact |
|-------|------------------------|---------------|-------------------|
| `ksni` | Renders the system tray icon and menu via the StatusNotifierItem D-Bus protocol | The SNI protocol is the standard on KDE/XFCE/modern GNOME. Reimplementing it means reimplementing a D-Bus service. | Could render fake tray state (showing "connected" when you're not). Cannot affect actual VPN state or firewall rules. |
| `zbus` | Subscribes to NetworkManager D-Bus signals for VPN state changes | NetworkManager exposes its API over D-Bus. This is the only way to get real-time VPN events without polling. | Could inject fake NetworkManager signals. Would cause Shroud to react to VPN state changes that didn't happen. |
| `futures-lite` | Provides the async stream adapter for iterating over D-Bus signals from `zbus` | `zbus` returns async streams. You need stream combinators to process them. This is the lightest option. | Could tamper with the D-Bus message stream. Same impact as `zbus` compromise. |
| `notify-rust` | Sends desktop notifications ("Connected to us-east-1") via D-Bus | Desktop notifications require the `org.freedesktop.Notifications` D-Bus interface. Not something you hand-roll. | Could send fake notifications or suppress real ones. Cannot affect VPN state. Annoyance-level impact. |
| `ctrlc` | Registers the Ctrl-C handler that triggers graceful shutdown and kill switch cleanup | Signal handling is platform-specific and easy to get wrong. This crate handles the edge cases. | Could block or intercept shutdown signals. Kill switch cleanup might not run. |
| `libc` | Provides `flock()` for the instance lock and low-level system call bindings | These are raw POSIX syscalls. There is no pure-Rust alternative. | **Full system access. This is the highest-risk dependency.** A compromised `libc` crate has access to every syscall Shroud makes. Game over. |

### Serialization

| Crate | What It Does in Shroud | Why It's Here | Compromise Impact |
|-------|------------------------|---------------|-------------------|
| `serde` | Derives `Serialize`/`Deserialize` for config structs and IPC messages | The config file and IPC protocol both need structured serialization. Writing a parser and serializer for each format by hand is fragile and slow. | Could corrupt config parsing or IPC deserialization. An attacker could inject arbitrary config values or forge IPC commands. |
| `toml` | Parses and serializes `~/.config/shroud/config.toml` | The config file is TOML. You need a TOML parser. | Could inject malicious config values during parsing. Bounded by Shroud's config validation layer. |
| `serde_json` | Serializes and deserializes JSON messages on the IPC socket | The IPC protocol uses JSON. You need a JSON parser. | Could forge or corrupt IPC messages. Bounded by IPC command validation and the Unix socket trust boundary. |

### Utilities

| Crate | What It Does in Shroud | Why It's Here | Compromise Impact |
|-------|------------------------|---------------|-------------------|
| `tracing` | Structured logging throughout the codebase -- state transitions, IPC commands, errors | `println!` doesn't cut it for a daemon. You need log levels, structured fields, and subscriber filtering. | Could suppress or falsify log output. An attacker could hide their tracks. |
| `tracing-subscriber` | Configures log output format and `RUST_LOG` filtering for the tracing framework | `tracing` needs a subscriber to actually output anything. This is the official one. | Same as `tracing`. Could suppress or redirect log output. |
| `dirs` | Resolves `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, and `XDG_RUNTIME_DIR` for config, logs, and socket paths | XDG directory resolution has platform-specific fallbacks. Getting it wrong means writing files to the wrong place. | Could redirect file paths. An attacker could point config or socket resolution to attacker-controlled locations. |
| `walkdir` | Recursively traverses directories during bulk VPN config import (`shroud import ~/configs/`) | `std::fs::read_dir` is non-recursive. Reimplementing recursive traversal with proper error handling is a solved problem. | Could inject fake directory entries during import. Bounded by the config validator that runs after import. |
| `thiserror` | Derives `Error` implementations for Shroud's error types | Writing `impl std::fmt::Display` and `impl std::error::Error` by hand for dozens of error variants is boilerplate. | Compile-time code generation only. No runtime attack surface. |
| `ureq` | Sends HTTP GET requests for health check pings to verify the VPN tunnel is working | Health checks need to make HTTP requests through the tunnel. `std::net::TcpStream` doesn't speak HTTP. | Could redirect health check pings or return fake responses. Cannot affect VPN traffic or firewall rules. Health checks have redirect following disabled and enforce connect timeouts. |
| `rand` | Generates random jitter for the linear backoff delay between reconnect attempts | Deterministic backoff causes thundering herd problems. You need randomness. `std::collections::hash_map::RandomState` is not a proper RNG. | Could make backoff timing predictable. Minimal security impact -- affects reconnect scheduling only. |

### Transitive Dependencies

The table above covers direct dependencies only. Each crate pulls in its own dependency tree. To inspect the full tree:

```bash
# Full dependency tree
cargo tree

# With duplicates shown
cargo tree --duplicates

# License audit for all transitive dependencies
cargo license --all-deps
```

For license compatibility of every dependency (direct and transitive) against Shroud's dual-license model, see [DEPENDENCY-AUDIT.md](../licenses/DEPENDENCY-AUDIT.md). For the full generated license list, see [THIRD-PARTY-LICENSES](../licenses/THIRD-PARTY-LICENSES).

---

## Security Model

Shroud's security assumptions:

| Trust | Don't Trust |
|-------|-------------|
| The local user | Remote networks |
| NetworkManager | VPN server contents |
| The kernel | D-Bus messages (validated) |
| iptables/nftables | User-provided config (validated) |

### Kill Switch Privileges

The kill switch requires root for iptables. Shroud uses sudoers rules that only allow specific commands:

```
%wheel ALL=(ALL) NOPASSWD: /usr/bin/iptables, /usr/bin/ip6tables, ...
```

This limits the attack surface. If someone compromises Shroud, they can manipulate firewall rules but not run arbitrary commands as root.

### No Network Communication

Shroud doesn't phone home. No telemetry. No update checks. No analytics.

The only network communication is:
1. VPN connection (through NetworkManager)
2. Health check pings (to verify tunnel works)

Both are initiated by the user.

---

## Threat Model

### In Scope

- VPN traffic leaks (IP, DNS, IPv6)
- Kill switch bypass
- Privilege escalation via sudoers rule
- State machine manipulation
- Config injection

### Out of Scope

- Attacks requiring root access (you already lost)
- Attacks on NetworkManager or iptables themselves
- Physical access attacks
- VPN protocol vulnerabilities (we just wrap, we don't implement)

---

## The Philosophy

Security through clarity.

Every firewall rule should be auditable. Every design decision should be explainable. If users can't understand what Shroud is doing, they won't trust it.

Shroud prioritizes fewer features done right over more features with hidden risks.

---

## Threat Model: Local Attacker Limitations

Shroud protects against **network-level threats**: ISP surveillance, open WiFi sniffing, accidental VPN disconnection, DNS leaks, and IPv6 leaks.

Shroud **does not protect** against a local attacker running as the same user. This is an inherent architectural constraint of user-level security tools, not a bug.

### What a same-user attacker can do

The IPC socket (`$XDG_RUNTIME_DIR/shroud.sock`) accepts commands from any process running as the same UID. A local attacker can:

- Send IPC commands to disconnect the VPN or disable the kill switch
- Read the debug log for VPN connection history
- Modify the config file (though security-critical downgrades via config reload are refused. Explicit IPC commands are required)

### Why this is acceptable

Shroud runs as the user it protects. The alternative, running as a system service with a separate UID, would require polkit integration, a client-server architecture, and significant complexity. This contradicts Principles V (Complexity Is Debt) and VIII (One Binary, One Purpose).

If an attacker has a shell as your user, they can already:
- Read your SSH keys, browser cookies, and GPG keys
- Install keyloggers via `.bashrc`
- Modify any file in your home directory

Shroud's job is to be the armor around your VPN. Protecting against local malware is the job of your OS, your login security, and your endpoint protection.

### Mitigations in place

- IPC commands are logged with the peer PID and source identification
- Security-critical config changes via file reload are refused (kill switch, auto-reconnect, DNS/IPv6 mode, DoH blocking)
- Disconnect does not persist kill switch disable to config. Protection restores on next VPN connect
- Config values are validated: health intervals bounded, endpoints HTTPS-only, reconnect attempts capped
- The socket is created with 0600 permissions and symlink protection (no TOCTOU)
- Health checks disable redirect following and enforce connect timeouts
- Health check suspension returns `Suspended` (not `Healthy`). No false assurance during wake
- All firewall commands use `Command::new().args()` (no shell expansion)
- VPN names are validated against shell metacharacters and ANSI escapes
- Sudoers rules are scoped to SHROUD_* chains (no bare `iptables -F` or `nft -f /path`)
- Restart path resolution removes PATH fallback and verifies ELF headers
- LAN firewall rules use auto-detected subnets (not full RFC1918)
- Kill switch Drop implementation attempts emergency rule cleanup

### Debug Logs

Shroud's debug log (`~/.local/share/shroud/debug.log`) contains VPN connection details including server names, IPs, connection timestamps, and state transitions. This file has `0600` permissions and is only readable by the running user.

Any process running as the same user can read this file. If you are concerned about local malware, consider disabling file logging (`--log-level error`) or configuring log rotation via the config file.

---

## IPC Security Model

Shroud's CLI communicates with the daemon over a Unix domain socket using JSON messages delimited by newlines. The connection is **not encrypted**. This section explains why, and what protections are in place instead.

### Why No Encryption

Adding TLS or any encryption layer to the IPC would add complexity with zero security benefit. Here's why:

1. **Unix domain sockets are local-only.** The kernel enforces this. They cannot be connected to from another machine. There is no network path to intercept.

2. **The socket has `0600` permissions.** Only the owning user can read or write to it. The socket is created with a restrictive umask before `bind()`, so there is no window where other users can access it.

3. **`XDG_RUNTIME_DIR` is per-user and mode `0700`.** On modern Linux systems, systemd creates this directory (typically `/run/user/<uid>/`) and restricts it to the owning user. The socket lives at `$XDG_RUNTIME_DIR/shroud.sock`.

4. **Any process that can access the socket already has full user privileges.** It can read your SSH keys, modify your shell config, and do anything you can do. Encrypting IPC against an attacker who is already you is security theater.

5. **Complexity is debt (Principle V).** TLS would require certificate management, key storage, and a trust model. All for protecting a socket that only you can access.

### Socket Path Selection

The socket is placed at `$XDG_RUNTIME_DIR/shroud.sock` when available. If `XDG_RUNTIME_DIR` is not set (rare on modern systems), Shroud falls back to `~/.local/share/shroud/shroud.sock`, a user-owned directory.

Shroud **does not** use `/tmp` for the socket. The `/tmp` directory has the sticky bit set, which means other users can create files there that the owning user cannot remove. A local attacker could pre-create a socket at the expected path to prevent the daemon from starting (denial of service). Using `XDG_RUNTIME_DIR` or a user-owned directory eliminates this attack vector.

### Symlink Protection

Before removing a stale socket file on startup, Shroud checks whether the path is a symlink using `symlink_metadata()`. If the socket path is a symlink, the server refuses to proceed and logs a warning. This prevents a class of TOCTOU (time-of-check-time-of-use) attacks where an attacker replaces the socket file with a symlink to another file.

> **Note:** A small TOCTOU window exists between the symlink check and the `remove_file()` call. This is acceptable because `XDG_RUNTIME_DIR` is mode `0700`. Only the owning user can create files there, so there is no attacker who could exploit the window.

### Peer Identification

On Linux, Shroud uses `SO_PEERCRED` (via `getsockopt`) to retrieve the PID of the connecting process. Every non-trivial IPC command is logged with the peer PID:

```
INFO Received command: Connect { name: "us-east-1" } from PID 12345
```

This provides an audit trail showing which process sent which command. If something unexpected happens, the logs will tell you what initiated it.

### Connection Limits

Two limits prevent resource exhaustion:

| Protection | Mechanism | Limit |
|-----------|-----------|-------|
| **Concurrent connections** | `tokio::sync::Semaphore` | 10 simultaneous connections |
| **Message size** | `BufReader::take()` | 64 KB per message (SHROUD-VULN-026) |
| **Commands per connection** | Counter with reject | 100 commands per session |

If a connection exceeds the message size limit without a newline, the server rejects it and closes the connection. This prevents a malicious or buggy client from sending an unbounded stream of data and exhausting memory.

### Protocol Versioning

The IPC protocol includes a version handshake. The first message from a client must be a `Hello` with the protocol version number. If the versions don't match, the server responds with `VersionMismatch` and the client reports a clear error asking the user to restart the daemon. This prevents silent failures when the CLI and daemon are from different versions.

### Trust Boundary

Shroud's IPC trust boundary is the Unix user. Any process running as the same user is considered trusted. This matches the POSIX security model. If an attacker has code execution as your user, IPC encryption would not save you. They could attach a debugger to the daemon, read its memory, or replace the binary entirely.

What Shroud *does* protect against:

- **Other users on the same system** -- socket permissions prevent cross-user access
- **Remote attackers** -- Unix domain sockets have no network exposure
- **Accidental interference** -- protocol validation rejects malformed input
- **Resource exhaustion** -- connection and message limits prevent DoS

What Shroud *does not* attempt to protect against:

- **Same-user malware** -- this is the job of endpoint protection, not a VPN manager
- **Root-level attackers** -- root can do anything, including reading the socket
