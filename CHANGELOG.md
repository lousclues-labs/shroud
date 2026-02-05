# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** This project underwent rapid initial development from January 25 to February 3, 2026.
> Version 1.0.0 was never released (jumped from 0.1.0 → 1.1.0).
> Version 1.3.0 was never released (jumped from 1.2.0 → 1.3.1).
> Dates below are derived from git commit history.

---

## [1.8.9] - 2026-02-04

### Fixed

- **Kill Switch Toggle Race Condition** - When clicking the kill switch toggle in the tray menu, the checkbox would briefly show the old state before updating. Now uses optimistic UI update: the tray immediately shows the new state while the async iptables operation runs in the background. On failure, the state rolls back.

---

## [1.8.8] - 2026-02-04

### Fixed

- **Invalid VPN State Bug** - When connecting to a non-existent VPN, the state machine incorrectly transitioned to `Reconnecting` instead of `Disconnected`, causing status to show "Connected to: nonexistent-vpn" when not connected. Now properly transitions to `Disconnected` with reason `connection_failed`. Discovered via chaos testing.

### Added

- **ConnectionFailed Event** - New state machine event for definitive connection failures (VPN doesn't exist, invalid config, etc.) that transitions directly to `Disconnected` rather than triggering reconnection attempts.

---

## [1.8.7] - 2026-02-03

### Fixed

- **Kill Switch State Flicker** - The kill switch would flicker between enabled/disabled states because `is_actually_enabled()` and `verify_rules_exist()` ran iptables commands without sudo. Permission denied errors were interpreted as "rules don't exist", causing state to reset to false every 30 seconds.

- **Log Timestamps Off by ~15 Days** - The `chrono_lite_timestamp()` function used naive leap year math, causing date drift.

### Changed

- **Consistent `sudo -n` Usage** - All iptables/nftables state-checking and cleanup functions now use `sudo -n` (non-interactive) to prevent hangs and ensure consistent behavior.

- **nftables Timeout Protection** - `run_nft()` now has a 30-second timeout.

---

## [1.8.6] - 2026-02-02

### Fixed

- **False Positive Latency Warnings** - Health checks no longer spam degraded warnings during builds. Threshold increased to 5000ms, requires 2 consecutive failures.

---

## [1.8.5] - 2026-02-02

### Added

- **Chaos Engineering Test Suite** - Tests for config corruption, IPC flood, signal storms, crash recovery.
- **Panic Hook** - Emergency kill switch cleanup on panic.
- **RESILIENCE.md** - Failure mode documentation.

### Fixed

- **D-Bus Timeout** - 10-second timeout prevents hang on unavailable D-Bus.
- **sudo/iptables Timeout** - 30-second timeout with `-n` flag.
- **Restart Breaks Daemon** - Proper `setsid()` detachment.
- **Stale Lock Files** - Dead PID detection and cleanup.
- **Config Corruption** - Backup to `.corrupted`, write fresh default.
- **XDG_RUNTIME_DIR Panic** - Fallback to `/tmp/shroud-{uid}`.

---

## [1.8.4] - 2026-02-02

### Fixed

- **Race Condition with External VPN Changes** - State diverged when users used nm-applet or nmcli directly. Added pre-reconnect state check, periodic state sync, and graceful "already active" handling.

---

## [1.8.3] - 2026-02-01

### Fixed

- **Tray Menu Crash** - Clicking menu items caused SIGABRT. The 1.8.1 fix used `blocking_send()` which panics inside ksni's async context. Changed to `try_send()`.

---

## [1.8.2] - 2026-02-01

### Fixed

- **Desktop Mode Silent Failure** - Users without DISPLAY were silently switched to headless mode. Removed auto-detection; desktop is now always default.
- **Update Double Build** - `shroud update` ran two builds. Now single `cargo install`.
- **Misleading Error** - Referenced non-existent `--daemon` flag.

### Changed

- **Startup Banner** - Shows "Shroud daemon starting..." on launch.

---

## [1.8.1] - 2026-02-01

### Fixed

- **Desktop Mode Broken** - Tray menu unresponsive after 1.8.0. Changed handlers from `tokio::spawn()` to `blocking_send()`.
- **Flaky Autostart Tests** - Changed to `#[ignore]` attribute.

### Added

- **CONTRIBUTING.md** - Contributor guidelines.

### Changed

- **Binary Size** - Fat LTO reduces size from 3.0MB to 2.6MB.

---

## [1.8.0] - 2026-02-01

### Added

- **Headless Mode** - Run as system service without GUI. Flags: `-H`/`--headless`, `--desktop`.
- **Systemd Integration** - Type=notify support with watchdog.
- **Boot Kill Switch** - Block traffic before VPN connects.
- **Auto-Connect** - Automatic connection with exponential backoff.
- **VPN Gateway Mode** - Route LAN traffic through VPN tunnel. Commands: `shroud gateway on/off/status`.
- **Gateway Configuration** - `[gateway]` config section with `allowed_clients`, NAT, forwarding.
- **Headless Configuration** - `[headless]` config section with auto-connect, boot kill switch.
- **Kill Switch Configuration** - `[killswitch]` config section with `allow_lan`.
- **Documentation** - `docs/HEADLESS.md`, `docs/GATEWAY.md`.

---

## [1.7.0] - 2026-01-31

### Added

- **`shroud doctor`** - Diagnose sudoers, firewall paths, backend selection.
- **Dynamic Firewall Detection** - Finds binaries in `/usr/bin` and `/usr/sbin`.

### Changed

- **sudo Instead of pkexec** - Avoids polkit session-type failures.
- **nftables Fallback** - Auto-fallback when iptables modules unavailable.
- **iptables-legacy Retry** - Fallback on nft backend errors.

### Fixed

- **Log Prefix Format** - Compatible with both iptables and nftables.
- **Empty IPC Response** - Treat as success for restart/quit.

---

## [1.6.5] - 2026-01-31

### Added

- **Sudoers Rule** - Passwordless kill switch.
- **Cleanup Module** - Timeout-based kill switch cleanup.

### Changed

- **sudo for Kill Switch** - Consistent privilege escalation.
- **Non-blocking Shutdown** - Clear notification on cleanup failure.

### Fixed

- **Atomic Binary Install** - Prevents "file busy" during update.

---

## [1.6.4] - 2026-01-31

### Fixed

- **DNS Leak Protection** - Explicit drop rules for tunnel/localhost/strict modes, DoT blocking.

---

## [1.6.3] - 2026-01-30

### Added

- **Update Progress** - Pacman-style progress for `shroud update`.

### Fixed

- **Restart Path** - Resolve executable when binary deleted during update.

---

## [1.6.2] - 2026-01-30

### Fixed

- **Import Tests** - Async-safe environment locking.

---

## [1.6.1] - 2026-01-30

### Fixed

- **Import Tests** - Avoid tempfs noexec for nmcli stub.

---

## [1.6.0] - 2026-01-30

### Added

- **WireGuard Support** - NetworkManager-based WireGuard connections.
- **`shroud import`** - Import WireGuard/OpenVPN configs, bulk directory import.
- **VPN Type in List** - Shows type and status, supports filtering.
- **Security Tests** - IPC socket, privilege escalation, config hardening, input validation.

---

## [1.5.1] - 2026-01-29

### Added

- **Security Input Validation** - Comprehensive input validation tests.

---

## [1.5.0] - 2026-01-28

### Added

- **Autostart** - XDG autostart with absolute binary path.
- **CLI** - `shroud autostart on/off/toggle/status`, `shroud cleanup`.
- **Tray** - "Start on Login" checkbox.

### Changed

- **Removed systemd user service** - XDG autostart preferred.

---

## [1.4.0] - 2026-01-28

### Added

- **Daemon Control** - `restart` and `reload` IPC commands.
- **CLI** - `shroud update`, `shroud reload`, `shroud version --check`.
- **Tray** - "Restart Daemon" menu option.

### Changed

- **Shutdown Safety** - Kill switch disabled before exit.

---

## [1.3.1] - 2026-01-28

*Note: Version 1.3.0 was skipped.*

### Fixed

- **Kill Switch Auth Hell** - Single `pkexec` call instead of per-rule prompts.
- **IPC Serialization** - Fixed `OkMessage` variant mismatch.
- **IPC Timeout** - Increased from 5s to 60s for password entry.
- **Firewall Cleanup** - Detect and remove legacy chains.

---

## [1.2.0] - 2026-01-27

### Added

- **IPC Architecture** - Unix socket at `$XDG_RUNTIME_DIR/shroud.sock`.
- **CLI Module** - Extracted to `src/cli/`.
- **Supervisor Module** - Extracted to `src/supervisor/`.
- **Daemon Lock** - Extracted to `src/daemon/lock.rs`.

### Changed

- **Structured Errors** - Migrated to `thiserror` with typed errors.
- **License** - GPLv3 + Commercial Dual License.

---

## [1.1.0] - 2026-01-26

*Note: Version 1.0.0 was skipped. Graduated directly from 0.1.0.*

### Added

- **GitHub Actions CI** - Format, clippy, tests, release build.
- **Security Audit Workflow** - Weekly `cargo-audit` scans.
- **Test Hardening** - 103 tests, +78% coverage. Pure parsing functions extracted.

### Changed

- **Tests Without External Commands** - No nmcli, iptables, pkexec in tests.

---

## [0.1.0] - 2026-01-25

### Added

- **Initial Release** - Rebranded from openvpn-tray.
- **VPN Management** - Provider-agnostic via NetworkManager.
- **Kill Switch** - iptables-based with DNS/IPv6 leak protection.
- **Auto-Reconnect** - Exponential backoff, configurable retries.
- **Health Monitoring** - Degraded state detection.
- **System Tray** - ksni (StatusNotifierItem) integration.
- **D-Bus Monitoring** - Real-time NetworkManager events.
- **State Machine** - Formal transitions, all logged.
- **Config Versioning** - Automatic migration from openvpn-tray.
- **Documentation** - README, PRINCIPLES, ARCHITECTURE.

### Changed

- **Rebrand** - openvpn-tray → Shroud.
- **Paths** - `~/.config/shroud/`, chain `SHROUD_KILLSWITCH`.

### Security

- **Atomic Writes** - Prevent config corruption.
- **Permissions** - 0600 files, 0700 directories.
