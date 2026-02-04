# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.8.7] - 2026-02-04

### Fixed

- **CRITICAL: Kill Switch State Flicker and Incorrect State Reporting**

  The kill switch would flicker between enabled/disabled states, and the tray UI would incorrectly show the kill switch as disabled even when firewall rules were active and blocking traffic.

  **Symptoms:**
  - Tray menu showing kill switch disabled when it was actually enabled
  - Kill switch checkbox flickering every ~30 seconds
  - State desync between what Shroud reports and actual iptables rules
  - Potential false sense of security if user believes kill switch is off when it's on
  - Potential leak vulnerability if user believes kill switch is on when it's off

  **Root Cause Analysis:**

  The bug stemmed from a **privilege escalation oversight** in three critical functions that check iptables state. These functions ran iptables commands directly without `sudo`, causing "Permission denied" errors:

  | Function | File | Problem |
  |----------|------|---------|
  | `is_actually_enabled()` | `killswitch/firewall.rs` | Ran `iptables -C OUTPUT -j SHROUD_KILLSWITCH` |
  | `verify_rules_exist()` | `killswitch/firewall.rs` | Ran `iptables -C OUTPUT -j SHROUD_KILLSWITCH` |
  | `rules_exist()` | `killswitch/cleanup.rs` | Ran `iptables -L SHROUD_KILLSWITCH -n` |
  | `rules_exist_ipv6()` | `killswitch/cleanup.rs` | Ran `ip6tables -L SHROUD_KILLSWITCH -n` |

  **The Failure Chain:**

  1. Every 30 seconds, `run_health_check()` calls `sync_killswitch_state()`
  2. `sync_killswitch_state()` calls `kill_switch.sync_state()`
  3. `sync_state()` calls `is_actually_enabled()` to verify rules exist
  4. `is_actually_enabled()` runs `iptables -C` **without sudo**
  5. iptables returns exit code 4: `Fatal: can't open lock file /run/xtables.lock: Permission denied`
  6. Function interprets this as "rules don't exist" and returns `false`
  7. `sync_state()` sets `enabled = false`
  8. `sync_killswitch_state()` updates `shared_state.kill_switch = false`
  9. Tray UI shows kill switch as disabled
  10. Next tick, actual rules are detected, state flips back — causing flicker

  **Why This Was Missed:**

  - The nftables path in `is_actually_enabled()` **did** use `sudo`, creating an inconsistency
  - The `enable()` and `disable()` functions correctly used `sudo` via `run_single_script()`
  - Testing was done with root permissions or with sudoers configured
  - The iptables lock file permission error is silent (stderr was piped to null)

  **The Fix:**

  1. All iptables state-checking functions now use `sudo -n` (non-interactive)
  2. If `sudo -n` fails (no NOPASSWD configured), functions fall back to the current internal state rather than incorrectly resetting to `false`
  3. Added consistent `-n` flag to nftables commands for parity
  4. This preserves state integrity when sudo access is unavailable

  **Files Changed:**
  - `src/killswitch/firewall.rs` - `is_actually_enabled()`, `verify_rules_exist()`, `verify_nft_rules_exist()`
  - `src/killswitch/cleanup.rs` - `rules_exist()`, `rules_exist_ipv6()`

  **Resilience Pattern:** Fail-safe state preservation. When we cannot verify external state, preserve internal state rather than assuming a dangerous default.

- **Log Timestamps Off by ~15 Days** - Debug log timestamps showed dates roughly 15 days in the future (e.g., Feb 18 instead of Feb 3). The `chrono_lite_timestamp()` function used naive date math (`days / 365`) that ignored leap years, causing ~1 day drift per 4 years since 1970. Fixed by properly iterating through years and months accounting for leap years.

### Changed

- **Consistent `sudo -n` Usage Across Entire Kill Switch Module** - Following the principle of defensive consistency, all sudo invocations across the kill switch subsystem now use the `-n` (non-interactive) flag:

  | File | Functions Updated |
  |------|-------------------|
  | `killswitch/firewall.rs` | `has_permission()`, `run_nft()` |
  | `killswitch/cleanup.rs` | `run_cleanup_command()`, `cleanup_all()` |
  | `killswitch/boot.rs` | `run_iptables()`, `run_ip6tables()` |

  This prevents:
  - Password prompt hangs during cleanup
  - Supervisor blocking on boot-time kill switch operations
  - Inconsistent behavior between state checking and rule application

- **nftables Command Timeout Protection** - `run_nft()` now has a 30-second timeout wrapper (matching iptables behavior). Previously, nftables commands could block indefinitely if the kernel module froze or sudo hung.

## [1.8.6] - 2026-02-02

### Fixed
- **False Positive Latency Warnings During Builds** - Health checks no longer spam "VPN Degraded - High latency" notifications during cargo builds, system updates, or other CPU-intensive tasks. The degraded threshold was increased from 2000ms to 5000ms, and the system now requires 2 consecutive degraded checks before showing a warning. Latency counter resets when connectivity returns to normal.

## [1.8.5] - 2026-02-02

### Added
- **Chaos Engineering Test Suite** - Comprehensive chaos tests in `tests/chaos/` that systematically test resilience against failure modes: config corruption, IPC flood, signal storms, rapid state transitions, crash recovery, and resource exhaustion.
- **Panic Hook for Emergency Cleanup** - If Shroud panics, a panic hook now attempts to clean up kill switch rules before exiting, preventing user lockout.
- **RESILIENCE.md Documentation** - New documentation in `docs/RESILIENCE.md` describing failure modes, recovery procedures, and hardening patterns.

### Fixed
- **D-Bus Connection Timeout** - Added 10-second timeout to D-Bus connection. Previously, if D-Bus was unavailable (container environments, frozen daemon), Shroud would hang forever. Now fails fast with clear error message.
- **sudo/iptables Command Timeout** - Added 30-second timeout to all sudo iptables commands with `-n` (non-interactive) flag. Prevents hanging on password prompts or frozen kernel modules.
- **Kill Switch Checkbox Inverted in Tray** - The tray menu checkbox now correctly reflects the actual iptables state, not just the config setting. Previously, the checkbox could show the opposite of reality if rules were cleaned up externally.
- **Restart Command Breaks Daemon** - Fixed `shroud restart` leaving users without a running daemon. The new process is now properly detached using `setsid()` to create a new session, and resources are cleaned up before spawning.
- **Multiple Restarts Required to Stabilize** - Added stale lock file detection that checks if the locking PID is still running. Dead process locks are now automatically cleaned up.
- **Corrupted Config Not Cleaned Up** - Corrupted config files are now backed up to `config.toml.corrupted` and a fresh default config is written, instead of just logging a warning.
- **Config Loaded Twice on Startup** - Removed duplicate config loading from main.rs; VpnSupervisor now loads config once.
- **XDG_RUNTIME_DIR Panic** - Lock file path now uses fallback `/tmp/shroud-{uid}` if XDG_RUNTIME_DIR is not set, instead of panicking.

### Changed
- **sudo Commands Use -n Flag** - All iptables commands now use `sudo -n` to fail immediately if password is required instead of hanging. This ensures timeout protection works correctly.

## [1.8.4] - 2026-02-01

### Fixed
- **CRITICAL: Race Condition with External VPN State Changes** - When users interacted with NetworkManager directly (e.g., disconnecting via nm-applet, switching to a different VPN), Shroud's internal state would diverge from reality, causing reconnection loops, "Connection already active" error spam, and incorrect kill switch state.

  **Symptoms:**
  - "Connection already active" errors flooding logs
  - Reconnection attempts when VPN is already connected
  - Kill switch enabled when it shouldn't be (or vice versa)
  - State showing "Connecting" when actually connected
  - State showing "Connected" when user disconnected via nm-applet
  
  **Root Cause Analysis:**
  Shroud relied on D-Bus events from NetworkManager to track state changes. However, D-Bus events can be delayed, arrive out of order, or be missed during high activity. When users bypass Shroud by using nm-applet, GNOME Settings, or `nmcli` directly, Shroud's internal state diverged from NetworkManager's actual state.
  
  **The Fix:**
  1. **Pre-reconnect State Check**: Before each reconnection attempt, query NetworkManager for actual VPN state. If target VPN is already active, cancel reconnection and sync internal state. If a different VPN is active, assume user switched manually and stop reconnecting.
  
  2. **Handle "Already Active" Gracefully**: Enhanced `nm::connect()` to pre-check if connection is already active. If so, treat as success instead of error. Also detect "already active" in nmcli error output and treat as success.
  
  3. **Periodic State Sync**: Added `sync_state_from_nm()` method called during health checks. Compares internal state with NetworkManager reality and corrects discrepancies. Handles edge cases like:
     - Internal: Disconnected, NM: VPN Active → Sync to Connected
     - Internal: Connected, NM: No VPN → Sync to Disconnected
     - Internal: Connected to VPN-A, NM: VPN-B Active → Sync to VPN-B
  
  4. **Kill Switch State Verification**: Added `sync_killswitch_state()` to verify kill switch rules are actually in iptables. Uses `is_actually_enabled()` to detect stale state.
  
  **New Methods:**
  - `nm::is_connection_active()` - Check if specific connection is active in NM
  - `supervisor::should_attempt_reconnect()` - Pre-flight check before reconnect
  - `supervisor::sync_state_from_nm()` - Full state sync from NM reality
  - `supervisor::sync_killswitch_state()` - Verify kill switch consistency
  
  **Affected Files:**
  - `src/nm/client.rs` - Added `is_connection_active()`, enhanced `connect()`
  - `src/supervisor/reconnect.rs` - Added `should_attempt_reconnect()`
  - `src/supervisor/state_sync.rs` - Added `sync_state_from_nm()`, `sync_killswitch_state()`
  - `src/supervisor/handlers.rs` - Integrated state sync into health check

## [1.8.3] - 2026-02-01

### Fixed
- **CRITICAL: Tray Menu Actions Crash Application** - Clicking any tray menu item (Kill Switch, Connect, Disconnect, etc.) caused the application to crash with `SIGABRT` and core dump. The daemon would silently disappear from the system tray.

  **Symptoms:**
  - Clicking "Kill Switch" toggle in tray menu kills the application
  - Tray icon disappears immediately
  - Core dump generated with `SIGABRT` signal
  - No error message displayed to user
  
  **Root Cause Analysis:**
  The v1.8.1 fix changed tray menu handlers from `tokio::spawn()` to `blocking_send()`. This was based on the assumption that ksni runs in a pure `std::thread`. However, ksni internally creates its own async runtime for D-Bus communication. When `blocking_send()` is called from within ksni's callback context, tokio detects it's already inside an async runtime and panics:
  
  ```
  thread '<unnamed>' panicked at src/tray/service.rs:281:33:
  Cannot block the current thread from within a runtime. This happens because
  a function attempted to block the current thread while the thread is being
  used to drive asynchronous tasks.
  ```
  
  **The Fix:**
  Changed all 10 tray menu action handlers from `blocking_send()` to `try_send()`. The `try_send()` method is completely non-blocking and does not interact with any runtime context. It returns immediately with `Ok(())` if the channel has capacity, or `Err(TrySendError::Full)` if full. Since the channel capacity is 16 and commands are processed quickly, this is safe.
  
  **Affected Code:**
  - `src/tray/service.rs` lines 236, 255, 269, 281, 293, 304, 318, 329, 343
  - All `VpnCommand` variants: Connect, Disconnect, ToggleAutoReconnect, ToggleKillSwitch, ToggleAutostart, RefreshConnections, ToggleDebugLogging, OpenLogFile, Restart
  
  **Version History:**
  - v1.8.0: Used `tokio::spawn()` - worked but was theoretically incorrect
  - v1.8.1: Changed to `blocking_send()` - caused crash inside ksni's async context
  - v1.8.3: Changed to `try_send()` - correct non-blocking approach

## [1.8.2] - 2026-02-01

### Fixed
- **CRITICAL: Desktop Mode Silent Failure** - Desktop users were silently switched to headless mode when running from terminals without DISPLAY/WAYLAND_DISPLAY environment variables set. This caused the tray icon to not appear and the application to seem "hung" (actually running headless in foreground). Root cause: overly aggressive auto-detection heuristics in mode.rs were checking for display variables, SSH sessions, and systemd INVOCATION_ID to determine mode.
- **Mode Detection Now Explicit** - Removed all auto-detection heuristics. Desktop mode is now ALWAYS the default. Headless mode requires explicit opt-in via `--headless` flag or `SHROUD_MODE=headless` environment variable. This prevents accidental mode switching that breaks user workflows.
- **Update Command Double Build** - The `shroud update` command was running `cargo build --release` followed by `cargo install --path .`, causing two separate builds (0s cached build + 98s fresh install build). Now uses single `cargo install` step.
- **Misleading Error Message** - Error messages incorrectly told users to run `shroud --daemon` which doesn't exist. Changed to correct instruction: `shroud`.

### Changed
- **Startup Banner** - Added visible startup message "Shroud daemon starting..." so users know the daemon launched successfully.

## [1.8.1] - 2026-02-01

### Fixed
- **CRITICAL: Desktop Mode Broken** - Tray menu actions (connect, disconnect, toggle kill switch, etc.) were completely unresponsive after the 1.8.0 headless implementation. Root cause: the tray runs in a `std::thread` (required by ksni), but menu action callbacks used `tokio::spawn()` which requires a tokio runtime context. Changed all 9 menu action handlers to use `blocking_send()` instead, which correctly works from blocking (non-async) contexts.
- **Autostart Tests Flaky in CI** - Changed 6 autostart-related tests to use `#[ignore]` attribute instead of runtime skip checks. Tests that create/modify XDG desktop files now require explicit `--ignored` flag, preventing race conditions in parallel test execution.
- **README Duplicate Section** - Removed duplicate "Kill Switch Privileges" section that appeared twice (at lines 337 and 448).
- **AllowedClients Test Coverage** - Added 7 comprehensive tests for `AllowedClients` enum serialization via `GatewayConfig` wrapper (TOML cannot serialize bare enums).

### Added
- **CONTRIBUTING.md** - Added contributor guidelines covering: design principles reference, development setup for Arch/Debian/Fedora, code quality requirements, PR process, commit message format, testing checklist, and code style guide.

### Changed
- **Binary Size Optimization** - Changed LTO from `"thin"` to `true` (fat LTO) in release profile. Reduces binary size from 3.0MB to 2.6MB (~13% reduction) through more aggressive cross-crate dead code elimination.

## [1.8.0] - 2026-02-01

### Added
- **Headless Mode**: Run Shroud as a system service without GUI dependencies.
  - New `-H` / `--headless` CLI flag to force headless mode.
  - New `--desktop` CLI flag to force desktop mode with tray.
  - Auto-detection based on environment (DISPLAY, systemd, SSH session).
  - Environment variable `SHROUD_MODE=headless|desktop` for configuration.
- **Systemd Integration**: Full Type=notify service support.
  - Systemd notify protocol: READY, STOPPING, STATUS, WATCHDOG messages.
  - Watchdog keep-alive for service health monitoring.
  - Service file at `assets/shroud.service` with security hardening.
- **Boot Kill Switch**: Block all traffic before VPN connects.
  - New `SHROUD_BOOT_KS` iptables chain for boot-time protection.
  - Configurable via `[headless] kill_switch_on_boot` option.
  - Transitions to runtime kill switch after VPN connects.
- **Auto-Connect**: Automatic VPN connection on startup with exponential backoff.
  - Configurable retry attempts (0 = infinite) and delay.
  - Jitter added to prevent thundering herd on reconnection.
- **VPN Gateway Mode**: Route LAN traffic through the VPN tunnel.
  - New `shroud gateway on/off/status` commands (alias: `gw`).
  - IP forwarding control via `/proc/sys/net/ipv4/ip_forward`.
  - NAT MASQUERADE rules for VPN interface.
  - FORWARD chain rules with client filtering (`allowed_clients`).
  - Gateway kill switch: blocks forwarded traffic if VPN drops.
  - Interface auto-detection for LAN (eth0, enp*) and VPN (tun*, wg*).
- **Gateway Configuration**: New `[gateway]` config section.
  - `enabled`: Auto-enable gateway on startup.
  - `lan_interface`: Override LAN interface (auto-detected by default).
  - `allowed_clients`: Filter by "all", CIDR, or IP list.
  - `kill_switch_forwarding`: Block forwarded traffic on VPN drop.
  - `persist_ip_forward`: Keep IP forwarding after exit.
  - `enable_ipv6`: Enable IPv6 forwarding (disabled by default for leak prevention).
- **Headless Configuration**: New `[headless]` config section.
  - `auto_connect`: Connect to VPN on startup.
  - `startup_server`: Server name to connect to.
  - `max_reconnect_attempts`: Retry limit (0 = infinite).
  - `reconnect_delay_secs`: Base delay for exponential backoff.
  - `kill_switch_on_boot`: Enable boot kill switch.
  - `require_kill_switch`: Fail startup if kill switch unavailable.
  - `persist_kill_switch`: Keep kill switch after Shroud exits.
- **Kill Switch Configuration**: New `[killswitch]` config section.
  - `allow_lan`: Allow LAN traffic when kill switch active.
- **Documentation**: Comprehensive guides for new features.
  - `docs/HEADLESS.md`: Headless deployment guide.
  - `docs/GATEWAY.md`: VPN gateway setup and usage.
  - `assets/shroud-headless.conf.example`: Example headless configuration.

### Changed
- **Main Entry Point**: Mode dispatch based on headless/desktop detection.
- **Config**: Extended `Config` struct with `headless`, `killswitch`, and `gateway` sections.
- **CLI Help**: Added gateway commands and headless examples.
- **Dependencies**: Added `rand` crate for jitter in exponential backoff.
- **Tokio**: Added `signal` feature for Unix signal handling.

### Fixed
- **Serialization**: Custom serde implementation for `AllowedClients` enum to handle TOML unit variants.

## [1.7.0] - 2026-03-01

### Added
- **CLI**: `shroud doctor` command to diagnose sudoers access, firewall paths, and backend selection.
- **Kill Switch**: Dynamic firewall binary detection across `/usr/bin` and `/usr/sbin`.
- **Setup**: Sudoers installation now generates multi-path rules and validates with `visudo`.

### Changed
- **Kill Switch**: Replace `pkexec` with `sudo` for privilege escalation to avoid session-type polkit failures.
- **Kill Switch**: Automatically fall back to nftables when iptables kernel modules are unavailable.
- **Kill Switch**: Retry with `iptables-legacy` when nft-style iptables backends report netlink/cache errors.
- **Cleanup**: Use detected firewall paths for cleanup commands and user guidance.

### Fixed
- **Kill Switch**: Log prefix format compatible with iptables/nftables logging.
- **IPC**: Treat empty restart/quit responses as success to avoid false failures.

## [1.6.5] - 2026-01-31

### Added
- **Sudoers**: Passwordless kill switch rule for reliable sudo-based escalation.
- **Setup**: `--install-polkit` and `--uninstall-polkit` options in `setup.sh`.
- **Cleanup**: Dedicated kill switch cleanup module with timeout-based cleanup.

### Changed
- **Kill Switch**: Execute rule changes via `sudo` for consistent privilege escalation.
- **Shutdown**: Non-blocking cleanup with clear user notification on failure.
- **Startup**: Stale rule detection and cleanup on launch.

### Fixed
- **Update/Restart**: Use atomic rename when installing the binary to avoid "file busy" errors while updating a running process.

## [1.6.4] - 2026-02-15

### Fixed
- **Kill Switch**: Explicit DNS drop rules in tunnel/localhost/strict modes, DoT blocking, and optional DoH blocking to prevent DNS leaks.
- **Cleanup**: Timeout-based kill switch cleanup with stale-rule detection and polkit policy support.

## [1.6.3] - 2026-02-14

### Added
- **Update UX**: Pacman-style progress line for `shroud update` build/install steps.

### Fixed
- **Restart**: Resolve restart executable path when the current binary is deleted during update.

## [1.6.2] - 2026-02-14

### Fixed
- **Tests**: Stabilized import tests with async-safe environment locking.

## [1.6.1] - 2026-02-14

### Fixed
- **Import Tests**: Avoid tempfs noexec issues for nmcli stub execution.

## [1.6.0] - 2026-02-14

### Added
- **WireGuard**: NetworkManager-based WireGuard connection support and type detection.
- **Import Helper**: `shroud import` for WireGuard/OpenVPN config files, including bulk directory import.
- **Status/List**: VPN type and status shown in list output, with type filtering.
- **Tests**: Expanded unit coverage for autostart, CLI handlers, IPC client/server, daemon lock, logging, and D-Bus monitor utilities.
- **Integration**: Added ignored daemon lifecycle integration tests.
- **Security Tests**: Added IPC socket, privilege escalation, config hardening, resource exhaustion, and CLI input validation security tests.
- **Security Tests**: Added crash recovery, race conditions, D-Bus validation, signal handling, and parsing validation coverage.

## [1.2.0] - 2026-01-27

### Changed
- **Architecture**: Moved CLI architecture documentation to `ARCHITECTURE.md`.
- **Error Handling**: Migrated to structured error handling using `thiserror` (Phase 2 complete).
  - Replaced `Result<T, String>` with specific error types: `ConfigError`, `ClientError`, `ServerError`, `NmError`, `KillSwitchError`.
  - Standardized error variants (Short naming convention).
  - Removed unused re-exports from module files.
  - Removed dead code error variants.
  - Added `#[allow(clippy::enum_variant_names)]` to error enums.
  - Improved error context and display.
- **CI/CD**: Fixed GitHub Actions workflow `toolchain` configuration.
- **Code Quality**: Applied `clippy` suggestions and strict formatting.

### Added
- **Documentation**: Enhanced `ARCHITECTURE.md` with CLI architecture diagram and error handling strategy.

## [1.5.0] - 2026-02-12

### Added
- **Autostart**: XDG autostart with absolute binary path (no PATH dependency).
- **CLI**: `shroud autostart on/off/toggle/status` and `shroud cleanup` for legacy cleanup.
- **Tray**: “Start on Login” checkbox in the tray menu.

### Changed
- **Startup**: Removed systemd user service support in favor of XDG autostart.

## [1.4.0] - 2026-02-12

### Added
- **Daemon Control**: New `restart` and `reload` IPC commands for daemon lifecycle management.
- **CLI**: Added `shroud update` (build + install + restart), `shroud reload`, and `shroud version --check`.
- **Tray**: Added “Restart Daemon” menu option.

### Changed
- **Shutdown Safety**: Restarts and shutdowns now disable the kill switch before exit to prevent lockout.
- **Dev Workflow**: Removed `update.sh` in favor of `shroud update`.

## [1.3.1] - 2026-02-12

### Fixed
- **Kill Switch**: Refactored to use a single `pkexec` call for all firewall operations. This eliminates the "authentication hell" loop where users were prompted for passwords multiple times (once per rule).
- **IPC Protocol**: Fixed a serialization mismatch (`OkMessage` variant) that caused client-daemon communication failures (`unknown variant` error).
- **Timeouts**: Increased IPC response timeout from 5s to 60s to accommodate the time required for users to enter their password during privilege escalation.
- **Firewall Cleanup**: Enhanced cleanup logic to reliably detect and remove legacy chains, preventing "chain already exists" errors.
- **Verification**: Removed non-root `iptables -C` checks that were causing permission denied errors or triggering unnecessary authentication prompts during rule verification.

## [1.3.0] - 2026-02-01

### Added
- **CLI command system** with single-binary dual-mode architecture
  - **Daemon mode** (`shroud`): Runs tray application, listens on Unix socket
  - **Client mode** (`shroud <command>`): Sends command to daemon and exits
  - Connection management: `connect`, `disconnect`, `reconnect`, `switch`
  - Status commands: `status`, `list` (with `--json` support)
  - Kill switch control: `killswitch on/off/toggle/status` (alias: `ks`)
  - Auto-reconnect control: `auto-reconnect on/off/toggle/status` (alias: `ar`)
  - Debug commands: `debug on/off/log-path/tail/dump`
  - Daemon control: `ping`, `refresh`, `quit`, `restart`
  - Unix socket IPC at `$XDG_RUNTIME_DIR/shroud.sock` with 0600 permissions
  - Exit codes: 0 (success), 1 (error), 2 (daemon not running), 3 (timeout)
  - JSON output (`--json` flag) for scripting and automation
  - Built-in `--help` for all commands and subcommands
  - Command aliases: `ls`, `ks`, `ar`, `stop`, `exit`
- Debug logging mode with multiple activation methods
  - CLI flags: `-v`, `--verbose`, `--log-level`, `--log-file`
  - Environment variable: `RUST_LOG=debug`
  - Runtime toggle via tray menu
- Log file rotation (10MB max, 3 files kept)
- Help (`--help`) and version (`--version`) command-line flags
- Dedicated logging module (`src/logging.rs`)
- Dedicated CLI module (`src/cli/`)
- "Open Log File" tray menu option
- `Quit` command for graceful daemon shutdown
- **Comprehensive setup.sh installation script** (1500+ lines)
  - Commands: `install`, `update`, `uninstall`, `check`, `repair`, `status`
  - Options: `--force`, `--dry-run`, `--verbose`, `--quiet`, `--help`
  - Pre-flight checks (distro, display, NetworkManager, desktop environment)
  - Multi-distro support (Arch primary, Debian/Fedora with appropriate packages)
  - Binary backup and rollback on installation failure
  - Default `config.toml` creation with full documentation
  - Systemd user service with automatic kill switch cleanup
  - Desktop entries for application menu and autostart
  - Shell completions for bash, zsh, and fish
  - Optional polkit policy for passwordless iptables (with security warnings)
  - Installation verification and summary
  - Detailed logging to `/tmp/shroud-setup-*.log`

### Changed
- **License**: Updated to GPLv3 + Commercial Dual License

### Fixed
- Kill switch now automatically disabled on intentional user disconnect to prevent network lockout
- Restart command properly cleans up resources before spawning new instance
- Quit command now properly exits the process instead of just returning from event loop
- **Signal handler now cleans up kill switch rules before exit** — prevents orphaned iptables rules
- **Startup now detects and cleans stale kill switch rules** — recovers from previous crashes

### Security
- CLI socket created with 0600 permissions (owner-only access)
- Socket located in `$XDG_RUNTIME_DIR` which is user-private
- Kill switch cleanup on SIGTERM/SIGINT prevents network lockout after crash

## [1.1.0] - 2026-01-26

### Added
- **GitHub Actions CI pipeline** (`ci.yml`)
  - Automated format checking (`cargo fmt --check`)
  - Clippy linting with warnings as errors
  - Full test suite execution
  - Release build verification
  - Rust caching via `Swatinem/rust-cache@v2`
- **Security audit workflow** (`security.yml`)
  - Weekly `cargo-audit` scans on schedule
  - Manual trigger via workflow dispatch
- **Test infrastructure hardening** (103 tests, +78% coverage)
  - Config module: `with_path()` constructor for test isolation
  - Config module: 7 new IO tests using tempfile (no env mutation)
  - NM client: `parse_vpn_connections()` extracted as pure function
  - NM client: `parse_vpn_uuid()` extracted as pure function
  - NM client: 15 new parsing tests (edge cases, state priority)
  - Kill switch: `build_ruleset()` extracted as pure function
  - Kill switch: 23 new tests (DNS modes, IPv6 modes, VPN allowlists)
- Added `tempfile` dev dependency for isolated testing

### Changed
- All tests now run without external commands (nmcli, iptables, pkexec)
- Pure parsing functions extracted from async I/O functions
- Version graduated from 0.x to stable 1.x series

## [0.1.0] - 2026-01-25

### Added
- Initial release as "Shroud" (rebranded from openvpn-tray)
- Provider-agnostic VPN connection management via NetworkManager
- Kill switch implementation using iptables
- DNS leak protection with three modes: tunnel (default), localhost, any
- IPv6 leak protection with three modes: block (default), tunnel, off
- Auto-reconnect with configurable retry attempts and exponential backoff
- Health monitoring with degraded state detection
- System tray integration via ksni (StatusNotifierItem)
- D-Bus monitoring for real-time NetworkManager events
- Polling fallback for event verification
- Formal state machine with logged transitions
- Config file versioning with automatic migration
- Config migration from old openvpn-tray location
- Systemd user service support
- XDG autostart desktop file
- XDG-compliant configuration and runtime paths
- Comprehensive documentation (README, PRINCIPLES, ARCHITECTURE)

### Changed
- Renamed from "openvpn-tray" to "Shroud"
- Config path changed from `~/.config/openvpn-tray/` to `~/.config/shroud/`
- Lock file changed from `shroud.lock` to `shroud.lock`
- iptables chain changed from `vpn_killswitch` to `SHROUD_KILLSWITCH`
- Binary name changed from `openvpn-tray` to `shroud`

### Security
- Atomic config file writes to prevent corruption
- File permissions set at creation time (0600 for files, 0700 for directories)
- Kill switch rules auditable via `iptables -S SHROUD_KILLSWITCH`
