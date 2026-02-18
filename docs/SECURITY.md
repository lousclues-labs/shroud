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

- **Description** — What's the vulnerability?
- **Impact** — What can an attacker do?
- **Reproduction** — Step-by-step instructions
- **Affected versions** — Which versions are vulnerable?
- **Proof of concept** — Code or commands that demonstrate it (if safe to share)

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
- Modify the config file (though security-critical downgrades via config reload are refused — explicit IPC commands are required)

### Why this is acceptable

Shroud runs as the user it protects. The alternative — running as a system service with a separate UID — would require polkit integration, a client-server architecture, and significant complexity. This contradicts Principles V (Complexity Is Debt) and VIII (One Binary, One Purpose).

If an attacker has a shell as your user, they can already:
- Read your SSH keys, browser cookies, and GPG keys
- Install keyloggers via `.bashrc`
- Modify any file in your home directory

Shroud's job is to be the armor around your VPN. Protecting against local malware is the job of your OS, your login security, and your endpoint protection.

### Mitigations in place

- IPC commands are logged with the peer PID and source identification
- Security-critical config changes via file reload are refused (kill switch, auto-reconnect, DNS/IPv6 mode, DoH blocking)
- Disconnect does not persist kill switch disable to config — protection restores on next VPN connect
- Config values are validated: health intervals bounded, endpoints HTTPS-only, reconnect attempts capped
- The socket is created with 0600 permissions and symlink protection (no TOCTOU)
- Health checks disable redirect following and enforce connect timeouts
- Health check suspension returns `Suspended` (not `Healthy`) — no false assurance during wake
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

1. **Unix domain sockets are local-only.** The kernel enforces this — they cannot be connected to from another machine. There is no network path to intercept.

2. **The socket has `0600` permissions.** Only the owning user can read or write to it. The socket is created with a restrictive umask before `bind()`, so there is no window where other users can access it.

3. **`XDG_RUNTIME_DIR` is per-user and mode `0700`.** On modern Linux systems, systemd creates this directory (typically `/run/user/<uid>/`) and restricts it to the owning user. The socket lives at `$XDG_RUNTIME_DIR/shroud.sock`.

4. **Any process that can access the socket already has full user privileges.** It can read your SSH keys, modify your shell config, and do anything you can do. Encrypting IPC against an attacker who is already you is security theater.

5. **Complexity is debt (Principle V).** TLS would require certificate management, key storage, and a trust model — all for protecting a socket that only you can access.

### Socket Path Selection

The socket is placed at `$XDG_RUNTIME_DIR/shroud.sock` when available. If `XDG_RUNTIME_DIR` is not set (rare on modern systems), Shroud falls back to `~/.local/share/shroud/shroud.sock` — a user-owned directory.

Shroud **does not** use `/tmp` for the socket. The `/tmp` directory has the sticky bit set, which means other users can create files there that the owning user cannot remove. A local attacker could pre-create a socket at the expected path to prevent the daemon from starting (denial of service). Using `XDG_RUNTIME_DIR` or a user-owned directory eliminates this attack vector.

### Symlink Protection

Before removing a stale socket file on startup, Shroud checks whether the path is a symlink using `symlink_metadata()`. If the socket path is a symlink, the server refuses to proceed and logs a warning. This prevents a class of TOCTOU (time-of-check-time-of-use) attacks where an attacker replaces the socket file with a symlink to another file.

> **Note:** A small TOCTOU window exists between the symlink check and the `remove_file()` call. This is acceptable because `XDG_RUNTIME_DIR` is mode `0700` — only the owning user can create files there, so there is no attacker who could exploit the window.

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

Shroud's IPC trust boundary is the Unix user. Any process running as the same user is considered trusted. This matches the POSIX security model — if an attacker has code execution as your user, IPC encryption would not save you. They could attach a debugger to the daemon, read its memory, or replace the binary entirely.

What Shroud *does* protect against:

- **Other users on the same system** — socket permissions prevent cross-user access
- **Remote attackers** — Unix domain sockets have no network exposure
- **Accidental interference** — protocol validation rejects malformed input
- **Resource exhaustion** — connection and message limits prevent DoS

What Shroud *does not* attempt to protect against:

- **Same-user malware** — this is the job of endpoint protection, not a VPN manager
- **Root-level attackers** — root can do anything, including reading the socket
