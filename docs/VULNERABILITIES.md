# Remediated Vulnerabilities

This document tracks vulnerabilities remediated in VPN Shroud and recorded in the changelog.

Scope:
- Internal vulnerabilities tracked with SHROUD-VULN IDs
- External dependency advisories tracked with RustSec IDs (RUSTSEC-*)

Source of truth: CHANGELOG.md
Last updated: 2026-08-12

Current totals from the changelog:
- Internal vulnerabilities: 43
- External dependency advisories: 5

---

## Severity Definitions

| Severity | Meaning |
|----------|---------|
| Critical | Can bypass core protection guarantees, disable the kill switch, or create a major trust boundary failure. |
| High | Significant security weakness with practical exploitation impact, but narrower than Critical. |
| Medium | Security gap or hardening failure that weakens protection or correctness. |
| Low | Minor hardening or operational security improvement with limited direct impact. |
| Unspecified | Severity was not explicitly labeled in the changelog entry. |
| Advisory | External RustSec advisory severity not normalized in this document. |

---

## Internal Vulnerability Index (SHROUD-VULN)

| ID | Severity | Fixed In | Summary |
|----|----------|----------|---------|
| SHROUD-VULN-001 | Unspecified | 1.15.0 | IPC peer PID logging - every non-trivial command is logged with the peer process ID and (self)/(external) source tag via SO_PEERCRED |
| SHROUD-VULN-002 | Unspecified | 1.15.0 | config reload refuses security downgrades - kill_switch_enabled, auto_reconnect, dns_mode, ipv6_mode, and block_doh cannot be weakened via config file reload. Explicit IPC commands still work |
| SHROUD-VULN-003 | Unspecified | 1.15.0 | sudoers rules (v3) scoped to SHROUD_* chain operations - bare iptables -F, bare nft -f /path no longer permitted. Only nft -f - (stdin) allowed |
| SHROUD-VULN-004 | Unspecified | 1.15.0 | IPC socket created with restrictive umask (0o077) before bind() - eliminates TOCTOU permission window. Symlink check before stale socket removal prevents symlink attacks |
| SHROUD-VULN-005 | Unspecified | 1.15.0 | SHROUD_NMCLI environment override gated behind #[cfg(test)] - production builds always use nmcli from PATH |
| SHROUD-VULN-006 | Unspecified | 1.15.0 | iptables jump rule (-I OUTPUT -j SHROUD_KILLSWITCH) now inserted LAST in script - chain is fully populated before traffic is directed to it, eliminating partial-chain window |
| SHROUD-VULN-007 | Unspecified | 1.15.0 | lan_restrict_ports config option - when true, only allows common LAN service ports (printing, file sharing, mDNS, SSDP, DNS, ICMP) instead of blanket LAN access Also: auto-detect actual LAN subnets from system interfaces instead of hardcoding full RFC1918 ranges. Falls back to RFC1918 if detection fails |
| SHROUD-VULN-008 | Unspecified | 1.15.1 | resolve_restart_path() removes $PATH fallback, verifies ELF headers, and warns on inode mismatch with running binary |
| SHROUD-VULN-009 | Unspecified | 1.15.0 | localhost DNS mode restricted to 127.0.0.1 and 127.0.0.53 only (was 127.0.0.0/8), preventing rogue resolver attacks on other loopback addresses |
| SHROUD-VULN-010 | Unspecified | 1.15.0 | setup script logs moved from world-readable /tmp to $XDG_DATA_HOME/shroud/ with 0600 permissions and cleanup-on-success trap |
| SHROUD-VULN-012 | Unspecified | 1.15.0 | VPN names now reject shell metacharacters (;\|&$\<>!) and ANSI escape sequences. Real-world names with @, (), Unicode still accepted |
| SHROUD-VULN-013 | Unspecified | 1.15.1 | ureq agent disables redirect following (max_redirects(0)) and adds 5s connect timeout |
| SHROUD-VULN-015 | Unspecified | 1.15.1 | handle_disconnect() no longer persists kill_switch_enabled = false to config - kill switch is suspended for the session only and restores on next VPN connect |
| SHROUD-VULN-016 | Unspecified | 1.15.1 | nftables backend now uses detect_local_subnets() instead of hardcoded RFC1918 ranges, matching the iptables backend |
| SHROUD-VULN-017 | Unspecified | 1.15.1 | check() returns HealthResult::Suspended instead of Healthy during suspension - callers leave state unchanged instead of falsely affirming health |
| SHROUD-VULN-021 | Critical | 1.15.2 | detect_local_subnets() now validates that all detected subnets are RFC1918/link-local with prefix >= 8. Rejects 0.0.0.0/0 and public ranges that would open the kill switch to all traffic |
| SHROUD-VULN-022 | Critical | 1.15.2 | custom_doh_blocklist entries are now validated as IPv4 addresses before interpolation into iptables/nft rulesets. Previously, arbitrary strings from config.toml were format-interpolated into the nft ruleset piped to nft -f -, enabling complete firewall bypass via nft scripting injection |
| SHROUD-VULN-023 | High | 1.15.2 | VPN name validation now rejects all control characters (tab, form feed, vertical tab), not just newlines. Prevents log line injection via \t and \r |
| SHROUD-VULN-024 | High | 1.15.2 | removed legacy config migration from ~/.config/openvpn-tray/. The migration followed symlinks and trusted arbitrary content on first load, bypassing all reload protections |
| SHROUD-VULN-025 | Medium | 1.15.2 | boot kill switch now uses detect_local_subnets() with RFC1918 fallback, consistent with runtime kill switch. Eliminates broader-than-intended LAN access during boot window |
| SHROUD-VULN-026 | Medium | 1.15.2 | IPC server uses bounded take() before read_line() - prevents OOM DoS from connections sending data without newlines. Previously read_line() allocated unbounded memory before the 64KB size check |
| SHROUD-VULN-027 | Medium | 1.15.2 | nmcli output parsing uses rsplitn (right-split) instead of split(':') for colon-delimited fields. Connection names containing : no longer corrupt field alignment |
| SHROUD-VULN-031 | Critical | 1.15.3 | restart spawns child BEFORE releasing lock/socket - eliminates 100ms hijack window where an attacker could grab the instance lock and impersonate the daemon |
| SHROUD-VULN-032 | Critical | 1.15.3 | is_actually_enabled() returns false (not internal state) when sudo verification fails. Prevents silent kill switch desync where tray shows enabled but rules are gone after sudo -K |
| SHROUD-VULN-033 | High | 1.15.3 | TOGGLE_IN_PROGRESS moved from static AtomicBool to struct-owned bool field. Eliminates static lifetime issues with task cancellation and concurrent toggle races |
| SHROUD-VULN-035 | High | 1.15.3 | kill switch toggle "best-effort disable" path no longer persists kill_switch_enabled = false to config when iptables errors occur. Runtime state updates but config retains user intent |
| SHROUD-VULN-036 | High | 1.15.3 | resolve_restart_path() no longer falls back to user-writable ~/.local/bin or ~/.cargo/bin when the running binary is deleted. Refuses to restart and instructs user to restart manually |
| SHROUD-VULN-039 | Medium | 1.15.3 | config migration (migrate()) no longer writes to disk. Migrated values are validated in-memory first; only persisted after Config::validate() passes. Prevents poisoned configs from surviving validation rejection |
| SHROUD-VULN-041 | Critical | 1.15.4 | VPN hostname resolution removed from kill switch enable path - only direct IP addresses from NM connection profiles are whitelisted. DNS resolution on the unprotected network allowed kill switch whitelist poisoning via ARP spoofing or rogue DHCP |
| SHROUD-VULN-042 | High | 1.15.4 | detect_local_subnets() now filters virtual/container interfaces (docker*, veth*, virbr*, br-*, cni*, flannel*, podman*). Prevents attacker-created interfaces from widening the kill switch LAN exception |
| SHROUD-VULN-043 | High | 1.15.4 | panic hook changed to fail-closed - kill switch rules are preserved on panic. Only socket and lock are cleaned so daemon can restart. Prevents attacker-triggered panics from disabling protection |
| SHROUD-VULN-045 | High | 1.15.4 | KillSwitch::Drop now only warns, does not attempt rule cleanup. Eliminates double-cleanup race between panic hook and Drop |
| SHROUD-VULN-046 | High | 1.15.4 | IPC Reconnect command now calls handle_connect() directly instead of disconnect-sleep-connect. Eliminates 2-second unprotected window where kill switch was disabled during reconnection |
| SHROUD-VULN-047 | Medium | 1.15.4 | autostart find_binary() now prefers system-wide paths (/usr/local/bin, /usr/bin) over user-writable paths (~/.cargo/bin). Prevents autostart entry from pointing at attacker-controlled binary |
| SHROUD-VULN-048 | Critical | 1.9.1 | - **Critical: Duplicate iptables Rules Causing Network Lockout** - Race conditions during rapid kill switch toggles or crashes would leave stale/duplicate iptables rules that block network access. Root cause: iptables -D only removes ONE matching rule, but race conditions can create multiple identical rules. Previous cleanup only attempted to delete one rule, leaving the rest blocking traffic |
| SHROUD-VULN-049 | Critical | 1.13.1 | kill switch rules are no longer torn down during handle_restart(). Previously, restarting the daemon disabled iptables rules before spawning the new instance; the new daemon started in Disconnected state and the kill switch restore check fired before initial_nm_sync() could detect the still-active VPN, leaving traffic unprotected. Rules now survive across restarts and the new instance adopts them via sync_state() in the constructor |
| SHROUD-VULN-050 | Medium | 1.16.8 | socket_path() fallback changed from /tmp/shroud-{uid}.sock to ~/.local/share/shroud/shroud.sock. The /tmp path was predictable and DoS-able - a local attacker could pre-create the socket file (sticky bit prevents the daemon from removing others' files), preventing daemon startup. The new fallback uses a user-owned directory. XDG_RUNTIME_DIR remains the primary path (set by systemd on all modern systems) |
| SHROUD-VULN-051 | Low | 1.16.5 | deleted SOCKET_PATH legacy constant (/tmp/shroud.sock) - a pub world-readable path with no user isolation. The real socket_path() function uses XDG_RUNTIME_DIR with UID-suffixed /tmp fallback. Any accidental use of the constant would create a socket accessible to all users |
| SHROUD-VULN-052 | Medium | 1.16.1 | log file creation uses OpenOptionsExt::mode(0o600) directly instead of post-creation chmod. Eliminates TOCTOU window where log files were briefly world-readable during creation and rotation |
| SHROUD-VULN-053 | Medium | 1.17.0 | - **cli: UTF-8 panic in VPN name validation** - validate_vpn_name() panicked on multi-byte UTF-8 strings exceeding MAX_VPN_NAME_LENGTH due to byte-index slicing (&value[..50]). Fixed to use .chars().take(50) for safe truncation at character boundaries. Discovered by fuzz testing in <1 second |
| SHROUD-VULN-054 | High | 1.16.2 | DOH_PROVIDER_IPS in firewall.rs and DOH_PROVIDERS in rules.rs were two separate lists that had drifted - firewall.rs had 16 entries (AdGuard, CleanBrowsing, Comodo) while rules.rs had only 8. Deduplicated into single canonical list in rules::DOH_PROVIDERS (now 14 entries). firewall.rs uses a use ... as alias. A DNS leak from adding a provider to one list but not the other is no longer possible |
| SHROUD-VULN-055 | High | 2.4.4 | WireGuard VPN server IPs are now whitelisted by the kill switch. detect_all_vpn_server_ips() matched only OpenVPN connections (parts[0] == "vpn") and parsed vpn.data remote=host:port, which does not exist for WireGuard, so enabling the kill switch for a WireGuard connection added no server-IP allow rule and could block the tunnel handshake. Detection now accepts wireguard connections and reads the wireguard.peers endpoint field (IPv6-bracket aware, port stripped); the validated IP flows into both the iptables and nftables allowlists. No DNS resolution is performed (SHROUD-VULN-041 preserved) - hostname endpoints are surfaced to the user instead |
| SHROUD-VULN-056 | Medium | 2.4.4 | Kill switch server-IP detection no longer bypasses hardened nmcli resolution. The detection path used a bare Command::new("nmcli"), bypassing crate::nm::nmcli_command() and the SHROUD_NMCLI #[cfg(test)] gating (SHROUD-VULN-005) used everywhere else. Routed through the centralized helper; no bare Command::new("nmcli") remains in src/killswitch/ |

---

## External Advisory Index (RustSec)

| ID | Severity | Fixed In | Summary |
|----|----------|----------|---------|
| RUSTSEC-2026-0049 | Advisory | 2.0.1 | - **deps: fix RUSTSEC-2026-0049 (rustls-webpki CRL bypass)** - updated rustls-webpki from 0.103.9 to 0.103.10 via cargo update. The vulnerable version had faulty CRL Distribution Point matching logic that caused CRLs not to be considered authoritative, potentially allowing revoked certificates to be accepted. Dependency chain: ureq -> rustls -> rustls-webpki. Affects health check HTTPS requests to VPN exit IP validation endpoints. No direct exploit path in Shroud (health checks validate response content, not just TLS handshake), but the vulnerable dependency fails cargo audit. |
| RUSTSEC-2026-0097 | Advisory | 2.0.4 | - **deps: fix RUSTSEC-2026-0097 (rand unsoundness)** - updated rand 0.9.2 -> 0.9.3. No direct exploit path (rand used for reconnect jitter), but resolves cargo audit warning. |
| RUSTSEC-2026-0098 | Advisory | 2.0.6 | - **deps: fix RUSTSEC-2026-0098 (rustls-webpki URI name constraint bypass)** - same update resolves an issue where name constraints for URI names were incorrectly accepted during certificate validation. A certificate with a URI SAN that should have been rejected by name constraints could be accepted as valid, potentially allowing an attacker with a misissued certificate to impersonate a constrained domain over TLS. Advisory date: 2026-04-14. See: https://rustsec.org/advisories/RUSTSEC-2026-0098 |
| RUSTSEC-2026-0099 | Advisory | 2.0.6 | - **deps: fix RUSTSEC-2026-0099 (rustls-webpki wildcard name constraint bypass)** - same update resolves an issue where name constraints were accepted for certificates asserting a wildcard name. A certificate containing a wildcard SAN (e.g., *.example.com) could bypass name constraint validation, allowing a CA-constrained certificate to be treated as valid for names outside its intended scope. Advisory date: 2026-04-14. See: https://rustsec.org/advisories/RUSTSEC-2026-0099 |
| RUSTSEC-2026-0104 | Advisory | 2.0.6 | - **deps: fix RUSTSEC-2026-0104 (rustls-webpki reachable panic in CRL parsing)** - updated rustls-webpki from 0.103.11 to 0.103.13 via cargo update rustls-webpki. The vulnerable version contained a reachable panic when parsing certain certificate revocation lists (CRLs). A malicious or malformed CRL served by a TLS peer could trigger panic!() in the CRL parsing path, causing an unrecoverable process abort. Dependency chain: ureq -> rustls -> rustls-webpki. Affects health check HTTPS requests to VPN exit IP validation endpoints. Advisory date: 2026-04-22. See: https://rustsec.org/advisories/RUSTSEC-2026-0104 |

---

## Accepted / Not-Applicable Advisories

Advisories that are **not remediated** because the vulnerable code does not ship
in VPN Shroud. These are ignored in `cargo audit` (`--ignore`) and in `deny.toml`
with the justification recorded here.

| ID | Severity | Status | Justification |
|----|----------|--------|---------------|
| RUSTSEC-2026-0009 | Medium (6.8) | Not applicable (does not ship) | `time` <0.3.47 stack-exhaustion DoS. `time` reaches the lockfile only as a **macOS/Windows-only** transitive dependency of `notify-rust`'s native notification backends (`mac-notification-sys` / `tauri-winrt-notification`), which are `cfg(target_os = "macos")` / `cfg(windows)`-gated. VPN Shroud is **Linux-only**, so the vulnerable code never compiles into or ships in the binary and the DoS is unreachable. The fix (`time` >=0.3.47) requires rustc 1.88, above the project MSRV of 1.87; bumping the MSRV to satisfy a dependency shroud does not use on its target platform would be disproportionate. Re-evaluate if macOS/Windows support is added or the MSRV moves to >=1.88. See https://rustsec.org/advisories/RUSTSEC-2026-0009 |

---

## Detailed Remediation Notes (Internal)

### SHROUD-VULN-001
- Fixed in: 1.15.0
- Severity: Unspecified
- Changelog remediation: IPC peer PID logging - every non-trivial command is logged with the peer process ID and (self)/(external) source tag via SO_PEERCRED

### SHROUD-VULN-002
- Fixed in: 1.15.0
- Severity: Unspecified
- Changelog remediation: config reload refuses security downgrades - kill_switch_enabled, auto_reconnect, dns_mode, ipv6_mode, and block_doh cannot be weakened via config file reload. Explicit IPC commands still work

### SHROUD-VULN-003
- Fixed in: 1.15.0
- Severity: Unspecified
- Changelog remediation: sudoers rules (v3) scoped to SHROUD_* chain operations - bare iptables -F, bare nft -f /path no longer permitted. Only nft -f - (stdin) allowed

### SHROUD-VULN-004
- Fixed in: 1.15.0
- Severity: Unspecified
- Changelog remediation: IPC socket created with restrictive umask (0o077) before bind() - eliminates TOCTOU permission window. Symlink check before stale socket removal prevents symlink attacks

### SHROUD-VULN-005
- Fixed in: 1.15.0
- Severity: Unspecified
- Changelog remediation: SHROUD_NMCLI environment override gated behind #[cfg(test)] - production builds always use nmcli from PATH

### SHROUD-VULN-006
- Fixed in: 1.15.0
- Severity: Unspecified
- Changelog remediation: iptables jump rule (-I OUTPUT -j SHROUD_KILLSWITCH) now inserted LAST in script - chain is fully populated before traffic is directed to it, eliminating partial-chain window

### SHROUD-VULN-007
- Fixed in: 1.15.0
- Severity: Unspecified
- Changelog remediation: lan_restrict_ports config option - when true, only allows common LAN service ports (printing, file sharing, mDNS, SSDP, DNS, ICMP) instead of blanket LAN access Also: auto-detect actual LAN subnets from system interfaces instead of hardcoding full RFC1918 ranges. Falls back to RFC1918 if detection fails

### SHROUD-VULN-008
- Fixed in: 1.15.1
- Severity: Unspecified
- Changelog remediation: resolve_restart_path() removes $PATH fallback, verifies ELF headers, and warns on inode mismatch with running binary

### SHROUD-VULN-009
- Fixed in: 1.15.0
- Severity: Unspecified
- Changelog remediation: localhost DNS mode restricted to 127.0.0.1 and 127.0.0.53 only (was 127.0.0.0/8), preventing rogue resolver attacks on other loopback addresses

### SHROUD-VULN-010
- Fixed in: 1.15.0
- Severity: Unspecified
- Changelog remediation: setup script logs moved from world-readable /tmp to $XDG_DATA_HOME/shroud/ with 0600 permissions and cleanup-on-success trap

### SHROUD-VULN-012
- Fixed in: 1.15.0
- Severity: Unspecified
- Changelog remediation: VPN names now reject shell metacharacters (;|&$\<>!) and ANSI escape sequences. Real-world names with @, (), Unicode still accepted

### SHROUD-VULN-013
- Fixed in: 1.15.1
- Severity: Unspecified
- Changelog remediation: ureq agent disables redirect following (max_redirects(0)) and adds 5s connect timeout

### SHROUD-VULN-015
- Fixed in: 1.15.1
- Severity: Unspecified
- Changelog remediation: handle_disconnect() no longer persists kill_switch_enabled = false to config - kill switch is suspended for the session only and restores on next VPN connect

### SHROUD-VULN-016
- Fixed in: 1.15.1
- Severity: Unspecified
- Changelog remediation: nftables backend now uses detect_local_subnets() instead of hardcoded RFC1918 ranges, matching the iptables backend

### SHROUD-VULN-017
- Fixed in: 1.15.1
- Severity: Unspecified
- Changelog remediation: check() returns HealthResult::Suspended instead of Healthy during suspension - callers leave state unchanged instead of falsely affirming health

### SHROUD-VULN-021
- Fixed in: 1.15.2
- Severity: Critical
- Changelog remediation: detect_local_subnets() now validates that all detected subnets are RFC1918/link-local with prefix >= 8. Rejects 0.0.0.0/0 and public ranges that would open the kill switch to all traffic

### SHROUD-VULN-022
- Fixed in: 1.15.2
- Severity: Critical
- Changelog remediation: custom_doh_blocklist entries are now validated as IPv4 addresses before interpolation into iptables/nft rulesets. Previously, arbitrary strings from config.toml were format-interpolated into the nft ruleset piped to nft -f -, enabling complete firewall bypass via nft scripting injection

### SHROUD-VULN-023
- Fixed in: 1.15.2
- Severity: High
- Changelog remediation: VPN name validation now rejects all control characters (tab, form feed, vertical tab), not just newlines. Prevents log line injection via \t and \r

### SHROUD-VULN-024
- Fixed in: 1.15.2
- Severity: High
- Changelog remediation: removed legacy config migration from ~/.config/openvpn-tray/. The migration followed symlinks and trusted arbitrary content on first load, bypassing all reload protections

### SHROUD-VULN-025
- Fixed in: 1.15.2
- Severity: Medium
- Changelog remediation: boot kill switch now uses detect_local_subnets() with RFC1918 fallback, consistent with runtime kill switch. Eliminates broader-than-intended LAN access during boot window

### SHROUD-VULN-026
- Fixed in: 1.15.2
- Severity: Medium
- Changelog remediation: IPC server uses bounded take() before read_line() - prevents OOM DoS from connections sending data without newlines. Previously read_line() allocated unbounded memory before the 64KB size check

### SHROUD-VULN-027
- Fixed in: 1.15.2
- Severity: Medium
- Changelog remediation: nmcli output parsing uses rsplitn (right-split) instead of split(':') for colon-delimited fields. Connection names containing : no longer corrupt field alignment

### SHROUD-VULN-031
- Fixed in: 1.15.3
- Severity: Critical
- Changelog remediation: restart spawns child BEFORE releasing lock/socket - eliminates 100ms hijack window where an attacker could grab the instance lock and impersonate the daemon

### SHROUD-VULN-032
- Fixed in: 1.15.3
- Severity: Critical
- Changelog remediation: is_actually_enabled() returns false (not internal state) when sudo verification fails. Prevents silent kill switch desync where tray shows enabled but rules are gone after sudo -K

### SHROUD-VULN-033
- Fixed in: 1.15.3
- Severity: High
- Changelog remediation: TOGGLE_IN_PROGRESS moved from static AtomicBool to struct-owned bool field. Eliminates static lifetime issues with task cancellation and concurrent toggle races

### SHROUD-VULN-035
- Fixed in: 1.15.3
- Severity: High
- Changelog remediation: kill switch toggle "best-effort disable" path no longer persists kill_switch_enabled = false to config when iptables errors occur. Runtime state updates but config retains user intent

### SHROUD-VULN-036
- Fixed in: 1.15.3
- Severity: High
- Changelog remediation: resolve_restart_path() no longer falls back to user-writable ~/.local/bin or ~/.cargo/bin when the running binary is deleted. Refuses to restart and instructs user to restart manually

### SHROUD-VULN-039
- Fixed in: 1.15.3
- Severity: Medium
- Changelog remediation: config migration (migrate()) no longer writes to disk. Migrated values are validated in-memory first; only persisted after Config::validate() passes. Prevents poisoned configs from surviving validation rejection

### SHROUD-VULN-041
- Fixed in: 1.15.4
- Severity: Critical
- Changelog remediation: VPN hostname resolution removed from kill switch enable path - only direct IP addresses from NM connection profiles are whitelisted. DNS resolution on the unprotected network allowed kill switch whitelist poisoning via ARP spoofing or rogue DHCP

### SHROUD-VULN-042
- Fixed in: 1.15.4
- Severity: High
- Changelog remediation: detect_local_subnets() now filters virtual/container interfaces (docker*, veth*, virbr*, br-*, cni*, flannel*, podman*). Prevents attacker-created interfaces from widening the kill switch LAN exception

### SHROUD-VULN-043
- Fixed in: 1.15.4
- Severity: High
- Changelog remediation: panic hook changed to fail-closed - kill switch rules are preserved on panic. Only socket and lock are cleaned so daemon can restart. Prevents attacker-triggered panics from disabling protection

### SHROUD-VULN-045
- Fixed in: 1.15.4
- Severity: High
- Changelog remediation: KillSwitch::Drop now only warns, does not attempt rule cleanup. Eliminates double-cleanup race between panic hook and Drop

### SHROUD-VULN-046
- Fixed in: 1.15.4
- Severity: High
- Changelog remediation: IPC Reconnect command now calls handle_connect() directly instead of disconnect-sleep-connect. Eliminates 2-second unprotected window where kill switch was disabled during reconnection

### SHROUD-VULN-047
- Fixed in: 1.15.4
- Severity: Medium
- Changelog remediation: autostart find_binary() now prefers system-wide paths (/usr/local/bin, /usr/bin) over user-writable paths (~/.cargo/bin). Prevents autostart entry from pointing at attacker-controlled binary

### SHROUD-VULN-048
- Fixed in: 1.9.1
- Severity: Critical
- Changelog remediation: - **Critical: Duplicate iptables Rules Causing Network Lockout** - Race conditions during rapid kill switch toggles or crashes would leave stale/duplicate iptables rules that block network access. Root cause: iptables -D only removes ONE matching rule, but race conditions can create multiple identical rules. Previous cleanup only attempted to delete one rule, leaving the rest blocking traffic

### SHROUD-VULN-049
- Fixed in: 1.13.1
- Severity: Critical
- Changelog remediation: kill switch rules are no longer torn down during handle_restart(). Previously, restarting the daemon disabled iptables rules before spawning the new instance; the new daemon started in Disconnected state and the kill switch restore check fired before initial_nm_sync() could detect the still-active VPN, leaving traffic unprotected. Rules now survive across restarts and the new instance adopts them via sync_state() in the constructor

### SHROUD-VULN-050
- Fixed in: 1.16.8
- Severity: Medium
- Changelog remediation: socket_path() fallback changed from /tmp/shroud-{uid}.sock to ~/.local/share/shroud/shroud.sock. The /tmp path was predictable and DoS-able - a local attacker could pre-create the socket file (sticky bit prevents the daemon from removing others' files), preventing daemon startup. The new fallback uses a user-owned directory. XDG_RUNTIME_DIR remains the primary path (set by systemd on all modern systems)

### SHROUD-VULN-051
- Fixed in: 1.16.5
- Severity: Low
- Changelog remediation: deleted SOCKET_PATH legacy constant (/tmp/shroud.sock) - a pub world-readable path with no user isolation. The real socket_path() function uses XDG_RUNTIME_DIR with UID-suffixed /tmp fallback. Any accidental use of the constant would create a socket accessible to all users

### SHROUD-VULN-052
- Fixed in: 1.16.1
- Severity: Medium
- Changelog remediation: log file creation uses OpenOptionsExt::mode(0o600) directly instead of post-creation chmod. Eliminates TOCTOU window where log files were briefly world-readable during creation and rotation

### SHROUD-VULN-053
- Fixed in: 1.17.0
- Severity: Medium
- Changelog remediation: - **cli: UTF-8 panic in VPN name validation** - validate_vpn_name() panicked on multi-byte UTF-8 strings exceeding MAX_VPN_NAME_LENGTH due to byte-index slicing (&value[..50]). Fixed to use .chars().take(50) for safe truncation at character boundaries. Discovered by fuzz testing in <1 second

### SHROUD-VULN-054
- Fixed in: 1.16.2
- Severity: High
- Changelog remediation: DOH_PROVIDER_IPS in firewall.rs and DOH_PROVIDERS in rules.rs were two separate lists that had drifted - firewall.rs had 16 entries (AdGuard, CleanBrowsing, Comodo) while rules.rs had only 8. Deduplicated into single canonical list in rules::DOH_PROVIDERS (now 14 entries). firewall.rs uses a use ... as alias. A DNS leak from adding a provider to one list but not the other is no longer possible

### SHROUD-VULN-055
- Fixed in: 2.4.4
- Severity: High
- Changelog remediation: WireGuard VPN server IPs are now whitelisted by the kill switch. Server-IP detection previously matched only OpenVPN connections and parsed the OpenVPN-shaped vpn.data remote=host:port, so WireGuard connections received no server-IP allow rule and the kill switch could block the handshake that establishes the tunnel. Detection now also accepts wireguard connections, reads the wireguard.peers endpoint host (IPv6-bracket aware, port stripped) via pure parsers in src/nm/parsing.rs, re-validates each IPv4 via killswitch::rules::is_valid_ipv4, and feeds the result into both the iptables and nftables allowlists. No DNS resolution occurs on the enable path (SHROUD-VULN-041 preserved); hostname-only configs are surfaced via a connect/enable warning and a `shroud verify` check.

### SHROUD-VULN-056
- Fixed in: 2.4.4
- Severity: Medium
- Changelog remediation: Kill switch server-IP detection routed through a bare Command::new("nmcli"), bypassing the centralized crate::nm::nmcli_command() helper and its SHROUD_NMCLI #[cfg(test)] gating (SHROUD-VULN-005). The detection logic was moved into the nm module and now uses nmcli_command() exclusively; no bare Command::new("nmcli") remains in src/killswitch/.

## Detailed Remediation Notes (External Advisories)

### RUSTSEC-2026-0049
- Fixed in: 2.0.1
- Severity category in this document: Advisory
- Changelog remediation: - **deps: fix RUSTSEC-2026-0049 (rustls-webpki CRL bypass)** - updated rustls-webpki from 0.103.9 to 0.103.10 via cargo update. The vulnerable version had faulty CRL Distribution Point matching logic that caused CRLs not to be considered authoritative, potentially allowing revoked certificates to be accepted. Dependency chain: ureq -> rustls -> rustls-webpki. Affects health check HTTPS requests to VPN exit IP validation endpoints. No direct exploit path in Shroud (health checks validate response content, not just TLS handshake), but the vulnerable dependency fails cargo audit.

### RUSTSEC-2026-0097
- Fixed in: 2.0.4
- Severity category in this document: Advisory
- Changelog remediation: - **deps: fix RUSTSEC-2026-0097 (rand unsoundness)** - updated rand 0.9.2 -> 0.9.3. No direct exploit path (rand used for reconnect jitter), but resolves cargo audit warning.

### RUSTSEC-2026-0098
- Fixed in: 2.0.6
- Severity category in this document: Advisory
- Changelog remediation: - **deps: fix RUSTSEC-2026-0098 (rustls-webpki URI name constraint bypass)** - same update resolves an issue where name constraints for URI names were incorrectly accepted during certificate validation. A certificate with a URI SAN that should have been rejected by name constraints could be accepted as valid, potentially allowing an attacker with a misissued certificate to impersonate a constrained domain over TLS. Advisory date: 2026-04-14. See: https://rustsec.org/advisories/RUSTSEC-2026-0098

### RUSTSEC-2026-0099
- Fixed in: 2.0.6
- Severity category in this document: Advisory
- Changelog remediation: - **deps: fix RUSTSEC-2026-0099 (rustls-webpki wildcard name constraint bypass)** - same update resolves an issue where name constraints were accepted for certificates asserting a wildcard name. A certificate containing a wildcard SAN (e.g., *.example.com) could bypass name constraint validation, allowing a CA-constrained certificate to be treated as valid for names outside its intended scope. Advisory date: 2026-04-14. See: https://rustsec.org/advisories/RUSTSEC-2026-0099

### RUSTSEC-2026-0104
- Fixed in: 2.0.6
- Severity category in this document: Advisory
- Changelog remediation: - **deps: fix RUSTSEC-2026-0104 (rustls-webpki reachable panic in CRL parsing)** - updated rustls-webpki from 0.103.11 to 0.103.13 via cargo update rustls-webpki. The vulnerable version contained a reachable panic when parsing certain certificate revocation lists (CRLs). A malicious or malformed CRL served by a TLS peer could trigger panic!() in the CRL parsing path, causing an unrecoverable process abort. Dependency chain: ureq -> rustls -> rustls-webpki. Affects health check HTTPS requests to VPN exit IP validation endpoints. Advisory date: 2026-04-22. See: https://rustsec.org/advisories/RUSTSEC-2026-0104

## Notes

- This file is intentionally changelog-driven and does not infer missing severity levels.
- SHROUD-VULN-007 has two distinct remediation notes in the same release and is merged into one entry above.
- If a future release remediates additional vulnerabilities, update this file and keep IDs append-only.
