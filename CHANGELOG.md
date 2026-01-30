# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
