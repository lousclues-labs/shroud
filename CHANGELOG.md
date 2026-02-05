# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** This project underwent rapid initial development from January 25 to February 3, 2026.
> Version 1.0.0 was never released (jumped from 0.1.0 → 1.1.0).
> Version 1.3.0 was never released (jumped from 1.2.0 → 1.3.1).
> Dates below are derived from git commit history.

---

## [1.9.1] - 2026-02-05

### Fixed

- **Critical: Duplicate iptables Rules Causing Network Lockout** - Race conditions during rapid kill switch toggles or crashes would leave stale/duplicate iptables rules that block network access. Root cause: `iptables -D` only removes ONE matching rule, but race conditions can create multiple identical rules. Previous cleanup only attempted to delete one rule, leaving the rest blocking traffic.

  - Boot kill switch (`boot.rs`): `insert_boot_chain_jump()` now removes ALL existing jump rules before inserting; `disable_boot_killswitch()` now loops to remove ALL duplicate jump rules (up to 100).
  
  - Cleanup module (`cleanup.rs`): `run_cleanup_command()` now loops to remove ALL duplicate jump rules for both SHROUD_KILLSWITCH and boot chains (iptables and ip6tables); `cleanup_all()` now uses loop-based removal for boot chain rules; `cleanup_stale_on_startup()` now also detects and cleans boot chain rules; added `boot_chain_exists()` helper function.
  
  - Firewall module (`firewall.rs`): Added `robust_iptables_cleanup()` that removes ALL duplicate rules (loops to remove all SHROUD_KILLSWITCH jump rules from OUTPUT, loops to remove all IPv6 direct rules, cleans up both IPv4 and IPv6 chains); `enable()` now calls `robust_iptables_cleanup()` BEFORE adding new rules; `disable()` now uses `robust_iptables_cleanup()` instead of script-based cleanup.

- **Coverage Tests Burning CI Minutes** - E2E tests requiring D-Bus session (`test_socket_cleanup_on_exit`) and chaos tests would hang or panic during tarpaulin coverage runs, burning 60+ CI minutes. Now excluded from coverage runs via `--exclude-files tests/e2e.rs --exclude-files tests/chaos.rs`.

### Changed

- **Coverage Script** - Added `EXCLUDE_ARGS` to exclude E2E and chaos tests that require system resources (D-Bus, iptables) and are unreliable in CI/coverage environments.

- **Scheduled Workflow** - Tarpaulin now excludes `tests/e2e.rs` and `tests/chaos.rs` from coverage runs.

### Technical Details

#### Root Cause Analysis

When the kill switch was enabled/disabled rapidly (either through user clicks or system events), the following sequence could occur:

1. Enable starts: cleanup runs (removes 1 rule), adds new rules
2. Disable starts: cleanup runs (removes 1 rule), state shows disabled
3. Enable starts again before step 2 fully completes
4. Result: Multiple identical rules in OUTPUT chain

Observed in production: 44+ duplicate `SHROUD_BOOT_KS` jump rules in ip6tables OUTPUT chain, causing complete IPv6 blockage even after "disabling" the kill switch.

#### New Functions

| Function | Module | Purpose |
|----------|--------|---------|
| `robust_iptables_cleanup()` | `firewall.rs` | Async cleanup that loops to remove ALL duplicates |
| `boot_chain_exists()` | `cleanup.rs` | Check if boot kill switch chain exists |

---

## [1.9.0] - 2026-02-05

### Added

- **Stability Test Suite** - New `tests/stability.rs` with 22 tests covering race condition prevention patterns, event deduplication, debounce logic, and scopeguard cleanup verification.

- **Health Check Suspension** - `HealthChecker::suspend(duration)` method to temporarily pause health checks during system events (wake from sleep). Prevents false positive "tunnel dead" alerts when network is briefly unavailable during wake.

- **D-Bus Event Deduplication** - `NmMonitor` now tracks recent events with a 500ms deduplication window. Prevents processing the same VPN state change multiple times when NetworkManager emits duplicate signals.

- **Reconnect Race Prevention** - Atomic `RECONNECT_IN_PROGRESS` flag prevents concurrent reconnection attempts. 5-second debounce period between reconnect starts prevents thrashing.

- **Kill Switch Toggle Protection** - Atomic `TOGGLE_IN_PROGRESS` flag prevents concurrent enable/disable operations. 500ms cooldown between toggles prevents race conditions under rapid user input.

- **scopeguard Dependency** - Added `scopeguard = "1"` for guaranteed cleanup of atomic flags on all exit paths (normal return, early return, panic).

### Fixed

- **Time Jump Detection Thrashing** - After resuming from sleep, the supervisor would emit multiple Wake events in rapid succession, causing state machine thrashing and duplicate notifications. Added 5-second cooldown (`TIME_JUMP_COOLDOWN_SECS`) between wake events and 2-second delay (`WAKE_EVENT_DELAY_MS`) before dispatch to let the system stabilize.

- **Health Check False Positives During Wake** - Health checks would immediately fail after system wake (network not yet ready), triggering unnecessary reconnection attempts. Now suspends health checks for 10 seconds after wake events.

- **Unknown VPN Disconnect Events** - D-Bus events for "unknown" VPN names (transient states during rapid connect/disconnect) would cause state corruption. Now filtered out in `should_process_event()`.

- **Reconnect Race with Active VPN** - If a user manually connected a VPN during an auto-reconnect loop, both connections could race. Now checks actual NetworkManager state before each reconnect attempt.

- **Kill Switch State Corruption** - Rapid enable/disable toggling (chaos testing) could leave iptables in an inconsistent state. Toggle lock and cooldown prevent concurrent operations.

### Changed

- **Time Jump Threshold** - Now uses explicit `TIME_JUMP_THRESHOLD_SECS` constant (6 seconds = 3× poll interval) instead of inline calculation for clarity.

- **NmMonitor::run()** - Changed from `run(self)` to `run(mut self)` to support internal state mutation for event deduplication.

- **Handler Method Signatures** - `handle_message()`, `handle_vpn_state_changed()`, and `handle_active_state_changed()` now take `&mut self` to support deduplication cache updates.

### Technical Details

#### New Constants

| Constant | Value | Location | Purpose |
|----------|-------|----------|----------|
| `TIME_JUMP_THRESHOLD_SECS` | 6 | `event_loop.rs` | Threshold for detecting resume from sleep |
| `TIME_JUMP_COOLDOWN_SECS` | 5 | `event_loop.rs` | Minimum seconds between wake events |
| `WAKE_EVENT_DELAY_MS` | 2000 | `event_loop.rs` | Delay before dispatching wake event |
| `EVENT_DEDUP_WINDOW_MS` | 500 | `monitor.rs` | D-Bus event deduplication window |
| `RECONNECT_DEBOUNCE_SECS` | 5 | `reconnect.rs` | Minimum seconds between reconnect attempts |
| `TOGGLE_COOLDOWN_MS` | 500 | `firewall.rs` | Minimum ms between kill switch toggles |

#### New Struct Fields

| Field | Type | Struct | Purpose |
|-------|------|--------|----------|
| `last_wake_event` | `Option<Instant>` | `VpnSupervisor` | Track last wake dispatch for cooldown |
| `last_reconnect_time` | `Option<Instant>` | `VpnSupervisor` | Track last reconnect for debounce |
| `suspended_until` | `Option<Instant>` | `HealthChecker` | When suspension expires |
| `recent_events` | `HashMap<(String, String), Instant>` | `NmMonitor` | Event dedup cache |
| `last_toggle_time` | `Option<Instant>` | `KillSwitch` | Track last toggle for cooldown |

#### Static Atomics

| Flag | Location | Purpose |
|------|----------|----------|
| `RECONNECT_IN_PROGRESS` | `reconnect.rs` | Prevent concurrent reconnect attempts |
| `TOGGLE_IN_PROGRESS` | `firewall.rs` | Prevent concurrent kill switch toggles |

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
