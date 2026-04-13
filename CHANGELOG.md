# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** This project underwent rapid initial development from January 25 to February 3, 2026.
> Version 1.0.0 was never released (jumped from 0.1.0 → 1.1.0).
> Version 1.3.0 was never released (jumped from 1.2.0 → 1.3.1).
> Dates below are derived from git commit history.

---

## [2.0.5] - 2026-04-13

### Changed
- **repo: migrate repository URLs from `loujr/shroud` to `lousclues-labs/shroud`** — updated all hardcoded repository references across documentation, metadata, legal files, assets, and CLI help text to reflect the canonical repository location after transfer. No code or build logic changes. GitHub maintains automatic redirects from the old URL.

---

## [2.0.4] - 2026-04-11

### Security
- **deps: fix RUSTSEC-2026-0097 (rand unsoundness)** — updated `rand` 0.9.2 → 0.9.3. No direct exploit path (rand used for reconnect jitter), but resolves `cargo audit` warning.

### Fixed
- **branding: remove redundant "VPN" from user-facing strings** — notifications, desktop entry, and install messages no longer say "VPN Shroud VPN".

---

## [2.0.3] - 2026-04-11

### Changed
- **docs: update project branding and LICENSE header** — added copyright preamble and trademark disclaimer to LICENSE. Updated product name references in documentation, notifications, and user-facing strings. No code, CLI, or packaging changes.

---

## [2.0.2] - 2026-03-29

### Changed
- **deps: bump MSRV from 1.85 to 1.87** — `zvariant` 5.10.0 and `zvariant_derive` 5.10.0 (transitive via `zbus` → `ksni`) raised their MSRV to 1.87. Pinning to older versions is not viable — `ksni` 0.3.3 fails to compile with `zvariant` 5.9.0 due to public dependency resolution requirements. `zbus` and `zbus_macros` pinned to 5.13.0 (MSRV 1.85) to avoid the 5.14.0 bump to 1.87. Updated `Cargo.toml` `rust-version` and README badge.

---

## [2.0.1] - 2026-03-29

### Security
- **deps: fix RUSTSEC-2026-0049 (rustls-webpki CRL bypass)** — updated `rustls-webpki` from 0.103.9 to 0.103.10 via `cargo update`. The vulnerable version had faulty CRL Distribution Point matching logic that caused CRLs not to be considered authoritative, potentially allowing revoked certificates to be accepted. Dependency chain: `ureq` → `rustls` → `rustls-webpki`. Affects health check HTTPS requests to VPN exit IP validation endpoints. No direct exploit path in Shroud (health checks validate response content, not just TLS handshake), but the vulnerable dependency fails `cargo audit`.
- **deps: resolve yanked crate warnings** — updated `js-sys` (0.3.88 → 0.3.92) and `wasm-bindgen` (0.2.111 → 0.2.115). Both were yanked upstream. Transitive dependencies of `uuid` via `zbus`/`ksni`. No runtime impact (wasm targets are not used), but yanked crates cause `cargo audit` warnings.

### Changed
- **ci: security audit schedule changed to monthly** — `scheduled.yml` cron changed from weekly (Sunday 9am UTC) to monthly (1st of each month, 9am UTC). Weekly was excessive for a project with stable dependencies. Monthly cadence still catches advisories before they age, and `workflow_dispatch` allows on-demand runs.

---

## [2.0.0] - 2026-03-01

### Changed
- **release: v2.0 public launch** — first public release. Transferred repository from staging organization to `loujr/shroud`. Version bumped from 1.18.2 to 2.0.0 to mark the public release milestone. Crate renamed from `shroud` to `vpn-shroud` on crates.io (`shroud` was taken). Binary name remains `shroud` — install with `cargo install vpn-shroud`, run with `shroud`. No code changes from 1.18.2.

---

## [1.18.2] - 2026-02-28

### Changed
- **tray: brand teal for connected icon** — replaced the green lock icon color with Shroud's brand teal (`#3FA88C` / `rgb(63, 168, 140)`) from the website design token `--shroud-connected`. Border darkened proportionally to `rgb(42, 112, 94)`. Aligns the system tray with vpnshroud.org brand identity.

---

## [1.18.1] - 2026-02-21

### Added
- **fuzz: smoke test target** — `fuzz_state_machine_smoke` runs identical chaos cannon logic as `fuzz_state_machine` but designed for 60-second CI runs. Uses the same shared infrastructure (`state_machine_common.rs`), same 32-slot event generator, same 13 invariants. Proves the fuzz binary compiles, the event generator covers all 14 variants plus chaos strings, and no shallow bugs exist. Not included in the MOAB workflow -- this runs on every push to `main` and every pull request.
- **ci: fuzz smoke test workflow** — `.github/workflows/fuzz-smoke.yml`. Runs on push to `main` and pull requests when `src/**`, `fuzz/**`, `Cargo.toml`, or `Cargo.lock` change. 15-minute timeout. Builds the smoke target with `cargo +nightly fuzz build`, runs for 60 seconds with `-max_total_time=60`, uploads corpus as artifact with 7-day retention.

### Fixed
- **ci: MOAB shard timeout** — shards were killed by GitHub Actions' hard 6-hour job limit before libfuzzer could exit cleanly. Setup (apt-get, toolchain, cargo-fuzz, build, seed corpus) consumed ~10-15 minutes, leaving no margin. The report job saw `cancelled` instead of `success`. Three fixes: default hours per shard reduced from 6 to 5, `timeout-minutes` reduced from 400 to 330, `max_total_time` cap reduced from 21600 to 18000 seconds. Leaves ~60 minutes for setup and artifact upload before the hard limit.
- **ci: missing `libdbus-1-dev` in MOAB workflow** — fuzz build jobs failed because `libdbus-sys` requires `libdbus-1-dev` and `pkg-config` on Ubuntu. Added `apt-get install -y libdbus-1-dev pkg-config` to both the build and fuzz jobs, matching the existing CI workflow.

---

## [1.18.0] - 2026-02-21

### Added
- **fuzz: state machine MOAB** — 5 new `cargo-fuzz` targets that attack the state machine from every angle. This is not incremental coverage. This is a fuzz battery designed to prove the core is unbreakable.
  - **`fuzz_state_machine`** (chaos cannon) — throws millions of random event sequences at the state machine. All 14 `Event` variants mapped across 32 input slots: 14 normal, 6 chaos server names (empty, null byte, control characters, ANSI escapes, shell injection), 6 huge/edge-case payloads (10,000-char strings, empty reasons), 6 wrong-server scenarios. ~4% probability of rapid-fire (same event 3x) driven by a timing byte. 13 invariants checked after every single event.
  - **`fuzz_state_machine_determinism`** (twin test) — two state machines start identical (`max_retries: 5`). Every event applied to both. After every event: states compared via `PartialEq`, retry counts compared, transition reasons compared via `Display`. Any divergence is a determinism violation. Proves the state machine has no hidden randomness, no timing dependence, no uninitialized memory influence.
  - **`fuzz_state_machine_lifecycle`** (escape hatch proof) — 5-phase structured test. Phase 1: fuzzer-controlled chaos events. Phase 2: force `UserDisable`, assert `Disconnected` with `retries == 0`. Phase 3: force `UserEnable` + `NmVpnUp`, assert `Connecting` then `Connected` with `retries == 0`. Phase 4: more chaos. Phase 5: force `UserDisable` again, assert `Disconnected` again. Proves `UserDisable` is always an escape hatch from any state, and the `UserEnable` → `NmVpnUp` connect sequence always works from `Disconnected`.
  - **`fuzz_state_machine_config`** (config extremes) — first 4 bytes of fuzz input interpreted as little-endian `u32` for `max_retries`. Remaining bytes are event sequence. Tests the state machine with `max_retries` values from 0 to `u32::MAX`. Special case for `max_retries == 0`: verifies `Timeout` from `Connecting` goes directly to `Failed`. Proves configuration values cannot break state machine invariants.
  - **`fuzz_state_machine_differential`** (cross-config comparator) — two machines, same event sequence, different configs (`max_retries: 3` vs `max_retries: 100`). Config-independent guarantees verified on both: `UserDisable` always reaches `Disconnected`, retry counters stay within their own config bounds. Config-dependent divergence is allowed (one may reach `Failed` while the other is still `Reconnecting`). Proves configuration doesn't violate universal structural properties.
- **fuzz: shared infrastructure** — `fuzz/fuzz_targets/state_machine_common.rs` shared by all 5 targets. Contains:
  - **Event generator** (`event_from_byte`) — deterministic mapping from byte to `Event`. 32 slots covering all 14 variants with normal inputs, chaos server names, huge payloads, injection attempts, and wrong-server scenarios.
  - **Parameterized event generator** (`event_from_byte_with_string`) — 14 slots using a fuzz-provided string for all string-carrying variants. Lets libfuzzer's mutation engine craft arbitrary server/reason strings.
  - **Server name pools** — `NORMAL_SERVERS` (5 realistic names), `CHAOS_SERVERS` (11 pathological names: empty, null byte, control chars, ANSI escapes, shell metacharacters, injection attempts, Unicode, RTL overrides), `huge_server_name()` (10,000-char string).
  - **Invariant checker** (`check_invariants`) — 13 invariants verified after every event on any `StateMachine`:
    - I1: Retry counter bounded by `max(max_retries, 2)`. The bound accounts for `Connected/Degraded → Reconnecting` setting `retries=1` without checking `max_retries`, followed by a `Timeout` incrementing to 2.
    - I2: `Disconnected` → `retries == 0`.
    - I3: `Reconnecting` → `retries > 0`.
    - I4: `Reconnecting.attempt == retries` (canonical counter sync).
    - I5: `Reconnecting.max_attempts == config.max_retries`.
    - I6: `Failed` → reason is non-empty.
    - I7: `Display` impl never panics (exercised via `format!`).
    - I8: `name()` returns one of the 6 known state names.
    - I9: `server_name()` never panics.
    - I10: `is_active()` never panics.
    - I11: `is_busy()` never panics.
    - I12: `Connecting` or `Reconnecting` → `is_busy() == true`.
    - I13: `Disconnected` or `Failed` → `is_active() == false`.
  - **Transition reason validator** (`check_transition_reason`) — verifies 7 `TransitionReason` → state mappings: `UserRequested` → `Disconnected` or `Connecting`, `RetriesExhausted` → `Failed`, `VpnEstablished`/`VpnReestablished` → `Connected`, `ConnectionFailed` → `Disconnected`, `VpnLost` → `Reconnecting`, `HealthCheckFailed` → `Degraded`, `HealthCheckDead` → `Reconnecting`.
- **ci: state machine MOAB workflow** — `.github/workflows/fuzz-state-machine.yml`. Manual dispatch (`workflow_dispatch`) with configurable target selection and hours-per-shard. 5 targets × 8 shards = 40 parallel jobs. Each shard uses a unique `-seed` for deterministic but non-overlapping fuzzer exploration. Flags: `-max_len=65536`, `-use_value_profile=1` (deeper coverage-guided exploration), `-reduce_inputs=1` (minimize corpus). Corpus seeded with 10 hand-crafted inputs per target: happy path, retry exhaustion, health cascade, sleep/wake storm, chaos variants, wrong server, rapid-fire, definitive failures, VPN switch, device change. `fail-fast: false` so all shards finish and every crash is collected. Crash artifacts retained 90 days, corpus 30 days. Summary job reports pass/fail with itemized proof list.
- **docs: dependency justification in `docs/SECURITY.md`** — new "Dependency Justification" section after "Dependency Audits" and before "Security Model". Every direct runtime dependency (19 crates) documented with: what it does in Shroud specifically, why it can't be eliminated, and what's exposed if supply-chain compromised. Dependencies grouped by role: Runtime (`tokio`, `async-trait`, `scopeguard`), System Integration (`ksni`, `zbus`, `futures-lite`, `notify-rust`, `ctrlc`, `libc`), Serialization (`serde`, `toml`, `serde_json`), Utilities (`tracing`, `tracing-subscriber`, `dirs`, `walkdir`, `thiserror`, `ureq`, `rand`). `libc` explicitly flagged as highest-risk dependency. Includes `cargo tree` and `cargo license` commands for transitive dependency inspection. References `licenses/DEPENDENCY-AUDIT.md` for license audit and `licenses/THIRD-PARTY-LICENSES` for full list.
- **docs: v2 launch framing in `README.md`** — temporary paragraph between tagline and lock shroud description: "v2.0 is here. I've been running v1 as my only VPN manager on my daily driver. Every bug I hit, I fixed. Every annoyance, I smoothed out. v1 was built for me. v2 is built for you." Not bolded, no heading, no banner. Removed when v2.0 is tagged and repo goes public.

---

## [1.17.0] - 2026-02-17

### Added
- **health: DNS leak detection** — passive DNS leak detection during health checks. Reads `/etc/resolv.conf` and verifies all nameservers are localhost or RFC 1918 private IPs. Public resolvers (8.8.8.8, 1.1.1.1, etc.) indicate DNS queries may bypass the VPN tunnel. Reports `Degraded` state when leaking DNS is detected. Auto-enabled when `dns_mode` is `tunnel` or `strict`. No network requests — inspects local config only (Principle IV). Config: `dns_leak_check = true/false`.
- **health: VPN exit IP validation** — optional `expected_exit_ip` config option. When set, health checks extract the detected exit IP from the response body (Cloudflare trace or plain text) and compare against the expected value. Mismatch reports `Dead` with "IP leak detected" reason. No extra HTTP requests — IP extracted from existing health check response. Config: `expected_exit_ip = "203.0.113.1"`.
- **fuzz testing** — three `cargo-fuzz` targets for critical parser surfaces:
  - `fuzz_ipc_command` — JSON deserialization of `IpcCommand` with round-trip verification.
  - `fuzz_config_parse` — TOML deserialization of `Config` with validation round-trip.
  - `fuzz_vpn_name` — `validate_vpn_name()` with arbitrary string input.
  - Infrastructure: `fuzz/Cargo.toml`, `scripts/fuzz.sh` convenience runner, `.gitignore` entries for corpus/artifacts.
  - Added `src/lib.rs` library target exposing `state`, `health`, `config`, `cli::validation`, `ipc::protocol`, and `notifications` modules for fuzz/integration test access.
- **docs: IPC security model** — comprehensive section in `docs/SECURITY.md` documenting: why IPC is not encrypted (Unix domain sockets are local-only, 0600 permissions), socket path selection (`XDG_RUNTIME_DIR`, `/tmp` avoidance), symlink protection (TOCTOU mitigation), peer PID logging (`SO_PEERCRED`), connection limits (semaphore, 64KB message cap, 100 commands/session), protocol versioning handshake, and trust boundary (Unix user model).
- **regression tests: behavioral state machine tests** — replaced fragile `include_str!` regression tests with real behavioral tests exercising the actual `StateMachine`, `HealthChecker`, and state types. 7 behavioral tests added: `ConnectionFailed` from Connecting/Reconnecting, retry exhaustion, `UserDisable` from all states, Display/Clone/Debug trait verification, `HealthChecker::with_config`. Remaining `include_str!` tests for killswitch/ipc/nm modules retained with `TODO` markers.
- **cargo-deny config** — `deny.toml` for automated license compliance enforcement in CI. Allows permissive licenses (MIT, Apache-2.0, BSD, ISC, Unlicense, Zlib, CC0), denies copyleft (GPL, AGPL, SSPL).

### Changed
- **legal: comprehensive licensing framework overhaul** — all 96+ `.rs` source files and 7 shell scripts now carry correct SPDX headers with `// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>`. Copyright holder updated from alias (`loujr`) to legal name (`Louis Nelson Jr.`) across all legally consequential documents.
  - `NOTICE` — full identity mapping (Louis Nelson Jr. ↔ loujr ↔ lousclues), license summary, third-party attribution, trademark notice.
  - `TRADEMARKS.md` — trademark ownership tied to legal name, common-law status documented, detailed permitted/prohibited uses, fork naming policy.
  - `CONTRIBUTOR-LICENSE.md` — versioned CLA (v1.0) with plain-language summary, patent grant, employer/contractor IP provisions, future licensing acknowledgment.
  - `LICENSE-COMMERCIAL.md` — formal commercial license template (v1.0) with numbered clauses: grant, restrictions, fees, warranty, liability cap, termination with GPL fallback, audit rights.
  - `LICENSE-DOCS.md` — CC BY 4.0 with explicit scope (included/excluded files).
  - `LICENSING.md` — file-type license coverage map, `LicenseRef-Commercial` SPDX definition, canonical header formats, CI verification script template.
  - `GOVERNANCE.md` — succession plan with three scenarios (temporary absence, permanent unavailability, voluntary transfer), designated successor placeholder.
  - `DEPENDENCY-AUDIT.md` — dependency license compatibility matrix (20+ license types), all 19 direct deps audited, `cargo-deny` config template, LGPL/static-linking warning.
- **legal: governing law** — updated LICENSE-COMMERCIAL.md and CONTRIBUTOR-LICENSE.md from vague "United States" to "Commonwealth of Virginia" with conflict-of-laws exclusion.
- **legal: directory reorganization** — supplementary license files moved to `licenses/` directory (LICENSE-COMMERCIAL.md, LICENSE-DOCS.md, LICENSING.md, CONTRIBUTOR-LICENSE.md, THIRD-PARTY-LICENSES, DEPENDENCY-AUDIT.md). LICENSE, NOTICE, TRADEMARKS.md, GOVERNANCE.md stay in root. All internal cross-references updated.
- **state machine: retry counter deduplication** — `self.retries` is now the canonical source of truth for retry count. `VpnState::Reconnecting { attempt }` is always derived from `self.retries` at transition time. Fixed 3 transitions (Connected→Reconnecting, Degraded→Reconnecting via HealthDead/NmVpnDown) where `attempt` was set to `1` without updating `self.retries`. Added `debug_assert!` after every Reconnecting transition to catch future desyncs. `max_attempts` always derived from `self.config.max_retries`.

### Fixed
- **cli: UTF-8 panic in VPN name validation** — `validate_vpn_name()` panicked on multi-byte UTF-8 strings exceeding `MAX_VPN_NAME_LENGTH` due to byte-index slicing (`&value[..50]`). Fixed to use `.chars().take(50)` for safe truncation at character boundaries. Discovered by fuzz testing in <1 second.
- **formatting** — applied `cargo fmt` to resolve CI lint failures in state machine debug assertion and regression test formatting.

### Security
- DNS leak detection prevents silent DNS bypass when VPN is connected with kill switch active.
- VPN exit IP validation prevents health checker from reporting "Healthy" when traffic bypasses the VPN.
- Fuzz testing covers three critical parser surfaces — found and fixed a real bug immediately.

---

## [1.16.20] - 2026-02-16

### Added
- **licensing**: comprehensive licensing framework for dual-licensed distribution:
  - `LICENSE` — rewritten with clean GPL-3.0 preamble (copyright notice + project attribution) so GitHub’s license detector correctly identifies GPL-3.0. Full verbatim GPL-3.0 text preserved. Removes custom dual-license preamble that caused GitHub to show "Other".
  - `LICENSE-COMMERCIAL.md` — explains commercial licensing option, when it’s needed, and how to obtain it.
  - `LICENSE-DOCS.md` — CC BY 4.0 license for all project documentation.
  - `TRADEMARKS.md` — trademark policy for "Shroud" and "lousclues" — permitted uses, fork naming guidelines.
  - `CONTRIBUTOR-LICENSE.md` — contributor license agreement for dual-licensing compatibility. License grant (not copyright assignment) — contributors retain copyright, grant broad license for commercial use.
  - `NOTICE` — central attribution file with copyright, license references, project URL.
  - `THIRD-PARTY-LICENSES` — inventory of all direct dependency licenses (19 crates, all MIT/Apache-2.0 compatible with GPL-3.0).
- **copyright**: SPDX headers added to all 72 `.rs` source files under `src/`: `// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial` + `// Copyright (C) 2026 loujr (lousclues)`.
- **contributing**: `CONTRIBUTING.md` now references the Contributor License with a plain-language summary.

### Changed
- **readme**: license section expanded to reference all license files (GPL, commercial, docs, third-party, trademarks, contributor license).
- **cargo**: `authors` field updated from `loujr` to `loujr (lousclues)`.

---

## [1.16.19] - 2026-02-16

### Changed
- **tray**: redesigned system tray menu for cleaner visual grouping. Branding header uses Unicode small caps (`ᨆʜʀᴏᴜᴅ`) for a quietly authoritative look. Connection list uses `●` for active VPN (was `✓ server (connected)`). Disconnect button only appears when connected (was always present, greyed out). Tools section groups Refresh, Debug Logging, and Open Log together. Removed icon names from Restart/Quit (cleaner on most themes). Open Log always visible for accessibility.

---

## [1.16.18] - 2026-02-16

### Added
- **tray**: version branding at the top of the system tray menu. Shows `Shroud v{version}` as a subtle disabled label above the status line. Uses `env!("CARGO_PKG_VERSION")` at compile time — always matches the binary version, no hardcoding.

---

## [1.16.17] - 2026-02-16

### Removed
- **ci**: removed `cargo-outdated` from the security audit workflow entirely. The tool has a known resolver bug that creates synthetic dependency pins (`libc = "=0.2.182"`) conflicting with transitive dependencies (`nix 0.31.0` requires `libc ^0.2.180`), causing persistent CI failures unrelated to actual security issues. `cargo audit` (which checks for real CVEs) remains and works correctly. Dependency freshness can be checked locally with `cargo outdated` when needed.

---

## [1.16.16] - 2026-02-16

### Fixed
- **ci**: `cargo-outdated` step in the security audit workflow no longer fails the job. The tool has a known resolver bug where it creates synthetic dependency pins (e.g., `libc = "=0.2.182"`) that conflict with transitive dependencies (`nix 0.31.0` requires `libc ^0.2.180`). Added `continue-on-error: true` at both job and step level, and consolidated the two redundant `cargo outdated` steps into one. Actual security auditing is done by `cargo audit` which is unaffected.

---

## [1.16.15] - 2026-02-16

### Fixed
- **build**: `Cargo.lock` is now committed to version control. Previously it was in `.gitignore`, which meant CI, users building from source, and packagers all resolved dependency versions independently — potentially getting different (and incompatible) versions. This directly caused the CI `cargo-outdated` failure where `libc` resolved to `0.2.182` in CI but `0.2.181` locally, conflicting with `nix 0.31.0` (via `ctrlc`). Per [Cargo’s official guidance](https://doc.rust-lang.org/cargo/faq.html#why-have-cargolock-in-version-control): commit `Cargo.lock` for binary applications, gitignore it only for libraries. Shroud is a binary.

### Changed
- **build**: removed `Cargo.lock` from `.gitignore`. All builds now use the exact same dependency versions pinned in the lockfile. Dependency updates are explicit via `cargo update` and reviewed in diffs.

---

## [1.16.14] - 2026-02-14

### Fixed
- **supervisor**: auto-connect on startup now actually works for existing users. The `auto_connect` field added in v1.16.13 defaulted to `false` with no migration — users who had autostart enabled and `last_server` set would still get an idle daemon after reboot. Three changes fix this:
  1. **Migration block**: if autostart is enabled but `auto_connect` is `false`, the startup path enables `auto_connect` and saves config. Runs once, subsequent boots load the saved value.
  2. **NM initialization delay**: 3-second sleep + `refresh_connections()` re-fetch before auto-connect. On login, NetworkManager may not have loaded VPN profiles yet — the initial connection list can be empty.
  3. **Fallback to first VPN**: if `last_server` is missing or refers to a deleted connection, auto-connect uses the first available VPN in NM instead of silently skipping.
- **supervisor**: `toggle_autostart()` now couples `auto_connect` with autostart state. When autostart is toggled ON, `auto_connect` is set to `true` and saved. When toggled OFF, `auto_connect` is set to `false`. Notification text updated to reflect this: "Shroud will start and auto-connect on login" / "Autostart and auto-connect disabled".
- **cli**: `shroud autostart on` / `off` / `toggle` now couples `auto_connect` with autostart state, matching the tray toggle behavior. CLI output updated: "Autostart enabled (auto-connect on login)" / "Autostart and auto-connect disabled".
- **docs**: `CONFIGURATION.md` updated — `auto_connect` description notes autostart coupling and first-VPN fallback. Added to defaults block and full example.

---

## [1.16.13] - 2026-02-14

### Added
- **config**: `auto_connect` option (boolean, default `false`). When enabled and `last_server` is set, desktop-mode Shroud automatically connects to the last used VPN on startup if no VPN is already active. Pair with `shroud autostart on` for automatic VPN protection on login. The field uses `#[serde(default)]` so existing config files without it deserialize safely with `false`.
- **supervisor**: auto-connect logic in `VpnSupervisor::run()` — after `initial_nm_sync()` and kill switch reconciliation, checks if state is `Disconnected`, `auto_connect` is `true`, and `last_server` exists in the current NM connection list. Calls `handle_connect()` if all conditions are met. Shows a tray notification before connecting. Logs a warning and skips gracefully if `last_server` is no longer in NetworkManager.
- **docs**: `auto_connect` documented in `CONFIGURATION.md` under Core Options.
- **setup**: `auto_connect = false` added to the default config template in `setup.sh` with explanatory comment.

---

## [1.16.12] - 2026-02-13

### Fixed
- **killswitch**: `enable()` and `disable()` `toggle_in_progress` flag now has explicit doc comment explaining why `scopeguard::defer!` cannot be used (borrows `&mut self`, conflicting with `enable_inner(&mut self)`). The manual `flag = true; let result = inner().await; flag = false; result` pattern is safe because `inner()` returns a `Result` (flag resets on both `Ok` and `Err`). The only unhandled case is panic — which is caught by the panic hook that preserves kill switch rules (fail-closed).
- **import**: `validate_wireguard()` `.unwrap()` changed to `.expect("contains check above")` for self-documenting intent on the `[peer]` find after a confirmed `.contains()` check.
- **health**: `check_endpoint()` doc now documents the `spawn_blocking` thread leak behavior — if `ureq` hangs on DNS timeout (up to 30s on some resolvers), the outer `tokio::time::timeout` cancels the future but the blocking thread persists until `ureq` returns. At most one leaked thread per health check interval.

### Changed
- **nm**: cleaned up `nm/mod.rs` re-exports. Removed three `#[allow(unused_imports)]` blocks. Only re-exports that are actually used externally remain: `connect`, `get_active_vpn` (headless runtime), `get_vpn_type`, `list_vpn_connections_with_types` (supervisor handlers), `NmCliClient`, `NmClient` (supervisor constructor). `NmError` re-exported under `#[cfg(test)]` only.
- **nm**: `nmcli_command()` doc now includes security note explaining why `SHROUD_NMCLI` env var is trusted (daemon environment set at launch by owning user, IPC clients cannot influence it).

---

## [1.16.11] - 2026-02-13

### Fixed
- **daemon**: `is_process_running(0)` now returns `false` instead of calling `kill(0, 0)` which signals the entire process group (always succeeds). A corrupted lock file containing "0" would make Shroud think another instance is running, preventing daemon startup.
- **daemon**: `acquire_instance_lock()` bounded to 1 retry for stale locks. Previously recursed unboundedly — if another process raced to create a new stale lock between `remove_file()` and retry, the recursion was infinite. Now returns a clear error on second failure.
- **dbus**: `NmMonitor` event sending changed from `tx.send().await` to `tx.try_send()`. If the supervisor blocks during reconnect and the D-Bus channel fills (capacity 64), `send().await` would suspend the monitor task. While suspended, the D-Bus `MessageStream` isn't drained, backing up the system D-Bus connection until it disconnects the client. `try_send()` drops the event with a `warn!` log and keeps the monitor alive — the 2-second poll fallback catches any missed state changes.
- **killswitch**: `cleanup_with_timeout()` doc now states that synchronous commands must not be called from the daemon event loop (CLI and startup use only). Use `KillSwitch::disable()` for async contexts.

### Changed
- **killswitch**: cleaned up `killswitch/mod.rs` re-exports. Removed three `#[allow(unused_imports)]` blocks — `cleanup_all` (used via direct path `cleanup::cleanup_all`, not the re-export), `paths::*` (only used within killswitch submodules), and `sudo_check::*` (only `validate_sudoers_on_startup` used externally). Keeps re-exports honest.
- **notifications**: removed `#[allow(unused_imports)]` on re-exports. Dropped unused `NotificationAction` and `Urgency` from the public API surface — only `Notification` and `NotificationCategory` are used outside the module.
- **logging**: added comment documenting that lowering `MAX_LOG_FILES` will orphan higher-numbered rotated log files.

---

## [1.16.10] - 2026-02-13

### Fixed
- **state**: `sync_state_from_nm()` now handles `Degraded + Some(different)` — if the user manually switches VPNs while in a degraded state, the poll fallback now detects the switch and updates state. Previously fell through to the wildcard arm which logged "internal state matches NetworkManager" (wrong). D-Bus events catch this in real-time, but the poll is the safety net.
- **import**: force-delete path (`--force` import) now uses `nmcli_output()` with `sh` fallback instead of `nmcli_command()` which lacked it. On NixOS or systems where nmcli needs a shim, `--force` imports would silently fail to delete the old connection. Also removed the now-unused `nmcli_command()` wrapper function.
- **tray**: `TrayBridge::update()` now logs `warn!` on thread spawn failure instead of `let _ =`. If the system hits RLIMIT_NPROC or runs out of threads, the operator sees it in logs.
- **main**: panic hook cleanup instructions now include IPv6 (`ip6tables`) and recommend `shroud cleanup` as the primary recovery command. Previously only showed IPv4 iptables commands — users with IPv6 kill switch rules (the default: `ipv6_mode = block`) would remain locked out on IPv6.
- **supervisor**: `RECONNECT_BASE_DELAY_SECS` doc comment changed from "exponential backoff" to "linear backoff" (third and final location — reconnect.rs and mod.rs module doc were fixed in v1.16.9, this constant doc was missed).
- **main**: D-Bus event channel capacity increased from 32 to 64. During VPN flapping, NM generates rapid activate/deactivate pairs at machine speed. 32 slots could fill while the supervisor is blocked in a reconnect loop, causing dropped events.
- **tray**: `SharedState` now derives `Debug` (was `Clone` only). All other state types (`VpnState`, `TimingState`, `SwitchContext`, `ExitState`) derive `Debug` — `SharedState` was the odd one out.
- **boot**: `detect_local_subnets()` call in boot kill switch documented as intentionally synchronous (no async runtime at boot).

### Removed
- **config**: removed unused `KillSwitchConfig` re-export from `config/mod.rs` (was `#[allow(unused_imports)]` — the type is only used within `settings.rs`).
- **import**: removed dead `nmcli_command()` wrapper (sole call site migrated to `nmcli_output()`).

### Changed
- **headless**: test-only module declarations in `headless/mod.rs` now have clarifying comments (`// Test-only config parsing helpers`, `// Test-only runtime helpers`).

---

## [1.16.9] - 2026-02-13

### Fixed
- **state**: `sync_state_from_nm()` now handles `Degraded + None` (VPN died silently while in degraded state). Previously this fell through to the wildcard arm which logged "internal state matches NetworkManager" — wrong. The supervisor would stay in `Degraded` forever while no VPN was actually active. Now transitions to `Disconnected` with `VpnLost` reason.
- **tray**: `TrayBridge::update()` now uses `std::thread::Builder::new().stack_size(64 * 1024)` instead of `std::thread::spawn()`. The default 8MB stack per OS thread wastes virtual address space when many updates queue during VPN flapping (each blocked on the ksni Mutex). 64KB is sufficient for the trivial lock+clone operation.
- **notifications**: `TrayBridge::notify()` fallback category changed from `Connected` to `FirstRun`. Unrecognized notification titles (first-run tips, "VPN Switched", etc.) were previously categorized as `Connected`, causing them to be throttled/filtered with actual connection notifications.
- **killswitch**: `detect_local_subnets()` no longer appends a duplicate `169.254.0.0/16` entry when a link-local interface is already detected. Prevents duplicate iptables rules in `iptables -L` output.
- **ipc**: `socket_path()` fallback now logs `warn!` on `create_dir_all` failure (was `let _ =`) and warns when `HOME` is unset (falls back to `/tmp` which is insecure).
- **docs**: `reconnect.rs` and `supervisor/mod.rs` doc comments changed from "exponential backoff" to "linear backoff" (matched the Cargo.toml fix from v1.16.8).
- **notifications**: removed unnecessary `#[allow(dead_code)]` from `notifications/mod.rs` module declarations — both `manager` and `types` are actively used. Moved `#![allow(dead_code)]` into the module files themselves with doc comments explaining these are prepared API surfaces with convenience methods not yet wired into all callers.

---

## [1.16.8] - 2026-02-13

### Fixed
- **import**: `validate_wireguard()` and `validate_openvpn()` now reject files larger than 1MB before reading (`MAX_CONFIG_SIZE`). Previously `fs::read_to_string()` read the entire file unbounded — a crafted multi-GB `.conf` file passing the 4KB detector could OOM the CLI import process.
- **import**: `validate_wireguard()` byte-index safety fix — now searches the lowercased copy consistently for `[Peer]` section content instead of using a byte offset from the lowercased string to index into the original. The old pattern could misalign on multi-byte UTF-8 characters before `[Peer]` (theoretical — WireGuard configs are ASCII, but correctness matters).
- **import**: `importer.rs` no longer has its own duplicate `nmcli_command()` / `nmcli_output()` / `nmcli_output_with_path()` functions with `#[cfg(test)]`-gated `SHROUD_NMCLI` overrides. Now delegates to centralized `crate::nm::nmcli_command()` (consolidated in v1.16.0 for `nm/client.rs` and `nm/connections.rs` but the importer was missed). Retains `sh` fallback for test stub compatibility.
- **ipc**: `socket_path()` fallback changed from `/tmp/shroud-{uid}.sock` to `~/.local/share/shroud/shroud.sock`. The `/tmp` path was predictable and DoS-able — a local attacker could pre-create the socket file (sticky bit prevents the daemon from removing others' files), preventing daemon startup. The new fallback uses a user-owned directory. `XDG_RUNTIME_DIR` remains the primary path (set by systemd on all modern systems).

### Removed
- **tray**: deleted dead modules `drawing.rs` (icon drawing primitives) and `state.rs` (TrayIcon, MenuItem, build_menu) — compiled into the binary but never called by `service.rs` which uses ksni types directly. Both had `#[allow(dead_code)]` on their `pub mod` declarations.
- **cli**: deleted dead modules `install.rs` and `output.rs` — both `#[allow(dead_code)]`, zero consumers anywhere in the codebase.
- **deps**: removed unused `libc::getuid()` call from `socket_path()` fallback.
- **cargo**: fixed rand dependency comment from "exponential backoff" to "linear backoff" (exponential was removed in v1.16.2).

---

## [1.16.7] - 2026-02-13

### Fixed
- **killswitch**: `classify_ip()` now correctly classifies IPv6 link-local addresses (`fe80::/10`) as `IpClass::LinkLocal` instead of `Public`. Uses manual segment check `(segments[0] & 0xffc0) == 0xfe80` since `Ipv6Addr::is_unicast_link_local()` is unstable in std.
- **dbus**: D-Bus monitor reconnect loop now resets backoff attempt counter on `Ok(())` (clean stream exit). Previously the counter only incremented, so after hours of intermittent reconnects the backoff saturated at 60s permanently. On a systemd D-Bus restart, the monitor would wait the full 60s before reconnecting. Both `main.rs` and `headless/runtime.rs` fixed.
- **supervisor**: `initial_nm_sync()` multi-VPN cleanup documented — keeps first VPN reported by NM (arbitrary nmcli order), not newest. The D-Bus handler uses "newest wins" policy. Both are valid but were undocumented as different.
- **supervisor**: `handle_connect()` kill switch pre-enable documented — notes that `enable()` reads server IPs from NM profiles, so a just-imported config where NM hasn't fully registered the profile may not have its server IP whitelisted.

---

## [1.16.6] - 2026-02-13

### Fixed
- **killswitch**: fixed kill switch UI desync where tray showed "Enabled" but no iptables rules existed. Root cause: `toggle_kill_switch()` optimistically updated `shared_state.kill_switch` before calling `enable()`/`disable()`, then trusted `Ok(())` as success. But `enable()`/`disable()` return `Ok(())` without acting when a cooldown or `toggle_in_progress` guard fires — so the UI showed the new state while the kill switch struct's `enabled` field and iptables reality hadn't changed. Fix: removed optimistic UI update; now reads `self.kill_switch.is_enabled()` after the operation and syncs shared state to actual state. Config is only persisted when actual state matches desired state.
- **killswitch**: IPC `KillSwitch { enable }` handler had the same pattern — set `shared_state.kill_switch = enable` on `Ok(())` without verifying actual state. Now reads `is_enabled()` after operation and persists config (was missing entirely before).
- **killswitch**: `sync_killswitch_state()` now runs on every NM poll cycle (every 2 seconds) in addition to health checks. Previously it only ran inside `run_health_check()`, which only fires when VPN is Connected or Degraded — meaning a kill switch desync while VPN was disconnected would persist in the tray indefinitely.
- **headless**: `auto_connect_nmcli()` doc changed from "exponential backoff" to "linear backoff" — the function calls `linear_backoff_secs()`.
- **headless**: `auto_connect_nmcli()` now logs `warn!` when connected to a different VPN than requested instead of silent `debug!` + `Ok(())`. The caller (and operator reading logs) now knows the actual connection doesn't match the configured `startup_server`.
- **killswitch**: `cleanup_with_timeout()` doc rewritten to accurately describe behavior — the timeout is a post-hoc duration check, not an enforced deadline. `sudo -n` prevents password prompts; the timeout detects unexpectedly slow commands.
- **killswitch**: IPv6 boot kill switch chain creation failures now logged at `warn!` level instead of silently swallowed with `let _ =`. Documents that IPv6 boot protection is best-effort.
- **ipc**: symlink check comment corrected from "TOCTOU mitigation" to "best-effort symlink check" with explanation of the residual TOCTOU window and why it's acceptable (`XDG_RUNTIME_DIR` is user-owned, mode 0700).
- **supervisor**: `health_check_interval_secs: 0` now correctly disables health checks as documented. Previously 0 fell through to the default (30s), contradicting the config doc ("0 to disable"). Uses `tokio::select!` precondition guard (`if health_checks_enabled`).

---

## [1.16.5] - 2026-02-13

### Fixed
- **headless**: `disable_boot_killswitch()` error no longer swallowed with `let _ =` after auto-connect. Now logs the error at `warn!` level. If the boot kill switch can’t be disabled after VPN connects, the user now sees it in logs instead of silently running with both boot and runtime chains active. Principle II: Fail Loud.
- **headless**: `shutdown()` now uses `tokio::join!` for concurrent task cancellation instead of sequential `.await` inside a 5-second timeout. Previously, if the supervisor took 4.9s to respond to abort, the remaining three tasks had only 100ms total and could be abandoned. Now all four tasks share the full timeout concurrently. Principle III: Leave No Trace.
- **headless**: SIGHUP handler no longer logs "Received SIGHUP, reloading config" (misleading — reload is not implemented). Changed to single `info!` message: "Received SIGHUP (config reload not yet implemented, ignoring)".
- **ipc**: deleted `SOCKET_PATH` legacy constant (`/tmp/shroud.sock`) — a `pub` world-readable path with no user isolation. The real `socket_path()` function uses `XDG_RUNTIME_DIR` with UID-suffixed `/tmp` fallback. Any accidental use of the constant would create a socket accessible to all users.
- **ipc**: `IpcCommand::List` validation now accepts `"all"` as a valid VPN type filter, consistent with the doc comment (`wireguard/openvpn/all`). Previously `"all"` was rejected with "Invalid VPN type filter" despite being documented as valid.

---

## [1.16.4] - 2026-02-13

### Fixed
- **killswitch**: `cleanup_logic.rs` promoted from `#[cfg(test)]` to production module. `cleanup.rs` now uses `cleanup_logic::SHROUD_CHAINS`, `build_remove_jump()`, `build_flush_chain()`, `build_delete_chain()`, and `manual_cleanup_instructions()` instead of hardcoding chain names and iptables argument arrays inline. This was the last shadow module in the codebase.
- **killswitch**: `run_cleanup_command()` now cleans **all** Shroud chains (`SHROUD_KILLSWITCH` + `SHROUD_BOOT_KS`) via the `SHROUD_CHAINS` constant. Previously `run_cleanup_command()` only cleaned `SHROUD_KILLSWITCH` — the boot chain cleanup was duplicated separately in `cleanup_all()` with `let _ =` on every result. Single source of truth for chain names. Principle III: Leave No Trace.
- **killswitch**: `cleanup_all()` simplified from 60+ lines of duplicated chain cleanup to a single `run_cleanup_command()` call + post-verification. The boot chain `let _ =` error swallowing is eliminated.
- **killswitch**: `cleanup_with_fallback()` and `log_manual_cleanup_instructions()` now use `cleanup_logic::manual_cleanup_instructions()` — instructions include both `SHROUD_KILLSWITCH` and `SHROUD_BOOT_KS` chains automatically.
- **health**: fixed stale doc comment on `HealthChecker::check()` — said "Returns Healthy immediately if checks are suspended" but code returns `HealthResult::Suspended`. Doc now accurately describes the `Suspended` return.
- **supervisor**: `dispatch()` comment changed from "Always sync shared state" to "Best-effort sync" with `debug!` log when `try_write()` is contended. The comment was misleading — `try_write()` can skip the sync, which is architecturally correct (the subsequent `sync_shared_state().await` guarantees consistency) but the comment implied guaranteed behavior. Principle VII: State Is Sacred.
- **nm**: `SHROUD_NMCLI` environment variable override now works in release builds, not just `#[cfg(test)]`. Users on NixOS or custom-prefix installations can set `SHROUD_NMCLI=/path/to/nmcli` to use a non-standard nmcli location. Principle VI: Speak the System’s Language.

### Removed
- **supervisor**: deleted dead `WAKE_EVENT_DELAY_MS` constant (2000ms). It had a comment explaining why it’s dead and an `#[allow(dead_code)]` suppressing the warning. The health check suspension (10s) replaced its purpose. Dead code with an apology is still dead code.

---

## [1.16.3] - 2026-02-13

### Removed
- **supervisor**: deleted four `#[cfg(test)]`-only modules that compiled only in test builds but appeared as real architecture in `mod.rs`: `command_validation.rs` (552 lines), `connection_stats.rs` (180 lines), `response_builder.rs` (495 lines), and `reconnect_logic.rs` (342 lines). None were imported or referenced by any production or test code outside their own files. `ReconnectTracker`, `ReconnectConfig`, `ReconnectDecision`, `ConnectionStats`, `ResponseBuilder` — none of these types were ever used by the actual supervisor. This was a shadow architecture that created the illusion of structure while the real reconnect path used `TimingState` fields and `util::backoff::linear_backoff_secs()` directly. Principle V: Complexity Is Debt.
- **state**: removed dead `base_delay_secs` and `max_delay_secs` fields from `StateMachineConfig`. These were set in the supervisor constructor but never read by any code — the actual backoff uses the `RECONNECT_BASE_DELAY_SECS`/`RECONNECT_MAX_DELAY_SECS` constants directly. The struct-level `#[allow(dead_code)]` that hid this is also removed.
- **supervisor**: deleted dead `SwitchContext::start()`, `complete()`, and `reset()` methods (all `#[allow(dead_code)]`). The handlers set `switch_ctx` fields directly — these methods were never called.

### Fixed
- **killswitch**: `run_cleanup_command()` no longer returns `Ok(())` unconditionally. Previously every chain flush/delete result was `let _ =` — spawn failures were invisible. Now tracks failures from all iptables/ip6tables/nft cleanup commands and returns `Err(CleanupError::CommandFailed)` with details if any command’s process fails to spawn. Exit-code failures on `-F`/`-X` are logged at debug level (idempotent cleanup of non-existent chains is expected). The caller `cleanup_with_timeout()` still verifies with its post-check. Principle II: Fail Loud, Recover Quiet.

### Added
- **util**: backoff tests now live in `util/backoff.rs` alongside the actual `linear_backoff_secs()` and `jitter_millis()` functions — 7 tests covering zero base, saturation, capping, and jitter bounds. Previously the only tests for the canonical backoff function were in the now-deleted `reconnect_logic.rs` shadow module.

---

## [1.16.2] - 2026-02-13

### Fixed
- **backoff**: unified three competing backoff implementations into one. `reconnect_logic::calculate_delay()` now delegates to `util::backoff::linear_backoff_secs()` (formula: `base * attempt`, capped at max). Removed dead `StateMachine::backoff_delay_secs()` method which used a different formula (`base * (attempt + 1)`) and was never called outside its own test. The actual reconnect path in `reconnect.rs` already used `linear_backoff_secs` — this change makes `reconnect_logic.rs` consistent with it.
- **killswitch**: `DOH_PROVIDER_IPS` in `firewall.rs` and `DOH_PROVIDERS` in `rules.rs` were two separate lists that had drifted — `firewall.rs` had 16 entries (AdGuard, CleanBrowsing, Comodo) while `rules.rs` had only 8. Deduplicated into single canonical list in `rules::DOH_PROVIDERS` (now 14 entries). `firewall.rs` uses a `use ... as` alias. A DNS leak from adding a provider to one list but not the other is no longer possible.
- **killswitch**: `cleanup_all()` no longer swallows errors. Previously every `Command` result was ignored with `let _ =` and the function always returned `Ok(())`. Now tracks errors from `cleanup_with_timeout()`, verifies no iptables/ip6tables/boot rules remain after cleanup, and returns `Err(CleanupError::CommandFailed)` if rules persist. Transient errors where rules were still successfully removed are logged as warnings.
- **killswitch**: `select_backend()` now prefers nftables over iptables when both are available. nftables applies rules atomically (no traffic gap during rule updates); iptables applies rules sequentially with a brief unprotected window. Falls back to iptables/iptables-legacy only when `nft` is unavailable.
- **config**: `Config::default()` `health_degraded_threshold_ms` changed from `2000` to `5000` to match `HealthConfig::default()` and the TOML schema comment. The 5000ms value was the intentional one (increased from 2000ms to avoid false degradation during builds/updates). The mismatch meant the value a user got depended on whether the supervisor constructed `HealthConfig` from config (2000ms) or used `HealthConfig::default()` directly (5000ms).
- **dbus**: D-Bus monitor (`NmMonitor`) now reconnects with linear backoff + jitter instead of silently exiting on stream end. Previously, if D-Bus restarted or the socket disconnected, the spawned task logged one error and exited — leaving the supervisor in poll-only mode (2s latency) with no indication to the user. Both `main.rs` and `headless/runtime.rs` spawn paths are fixed. Added `NmMonitor::into_tx()` for sender reuse across reconnect iterations.
- **supervisor**: non-Disconnect commands received during reconnect are now queued in a `VecDeque<VpnCommand>` and drained after the reconnect loop completes. Previously `ToggleKillSwitch`, `Connect(different_server)`, `Quit`, and all other commands were silently dropped during reconnect backoff. Added `deferred_commands` field to `VpnSupervisor`.

### Changed
- **docs**: `toggle_in_progress` in `KillSwitch` and `reconnect_in_progress` in `TimingState` now have explicit safety-invariant doc comments explaining why a plain `bool` (not `AtomicBool`) is safe — both are guarded by the single-task `&mut self` event loop. Documents the exact condition under which they would need to change.

---

## [1.16.1] - 2026-02-13

### Fixed
- **nm**: consolidated `nmcli_command()` into a single function in `nm/mod.rs`. Eliminates divergent copies in `client.rs` and `connections.rs` that could drift independently.
- **nm**: `connections.rs` now has 30-second timeout protection on all nmcli calls, matching `client.rs`. Previously `list_vpn_connections_with_types()` and `get_vpn_type()` could hang indefinitely.
- **nm**: fixed `parse_vpn_uuid()` in `nm/parsing.rs` for VPN names containing colons (same fix as `client.rs` from v1.16.0).
- **logging**: log file creation uses `OpenOptionsExt::mode(0o600)` directly instead of post-creation `chmod`. Eliminates TOCTOU window where log files were briefly world-readable during creation and rotation.
- **import**: OpenVPN validator no longer accepts `<connection>` tag as substitute for `remote` directive. Prevents validation pass on malformed configs with no server address.
- **import**: `.conf` file detector reads only first 4KB instead of entire file. Prevents loading large non-WireGuard configs during directory import.
- **tests**: regression test `regression_nmcli_env_override` updated to check `nm/mod.rs` (where `nmcli_command()` now lives) instead of `nm/client.rs`.

## [1.16.0] - 2026-02-13

### Fixed
- **nm**: centralized `nmcli_command()` into `nm/mod.rs` — eliminates duplicate definitions in `client.rs` and `connections.rs` that could diverge. `connections.rs` now uses the shared function with timeout protection.
- **nm**: fixed `parse_vpn_uuid` colon-in-name bug in both `client.rs` and `parsing.rs` — uses `split_once`/`rsplit_once` instead of `rsplitn(3)` which misaligned UUID/NAME/TYPE fields when VPN names contained `:`.
- **import**: OpenVPN validator now requires `remote` directive or complete `<connection>...</connection>` block. Previously accepted bare `<connection>` tag without closing tag or remote.
- **import**: `.conf` file detector reads only first 4KB instead of entire file. Prevents loading large non-WireGuard configs during directory import.
- **reconnect**: `RECONNECT_IN_PROGRESS` moved from `static AtomicBool` to `TimingState` struct field. The static survived supervisor restart, permanently blocking reconnection.
- **event loop**: wake-from-sleep handling no longer blocks with 2-second sleep. IPC/D-Bus/tray commands process immediately during system wake.

## [1.15.5] - 2026-02-13

### Fixed
- **restart**: `resolve_restart_path()` now handles the Linux `(deleted)` suffix from `/proc/self/exe` by checking if a replacement binary exists at the original path. Fixes daemon restart failure during `shroud update` when the running binary is replaced.
- **update**: `scripts/update.sh` uses `quit` + direct start instead of IPC `restart` command. The old daemon's restart logic cannot be relied on across version boundaries.
- **update**: binary replacement changed from `rm + cp` to atomic `cp .shroud.new + mv`, avoiding the `(deleted)` state and ETXTBSY errors.

## [1.15.4] - 2026-02-13

### Fixed
- **security**: VPN hostname resolution removed from kill switch enable path — only direct IP addresses from NM connection profiles are whitelisted. DNS resolution on the unprotected network allowed kill switch whitelist poisoning via ARP spoofing or rogue DHCP (SHROUD-VULN-041, Critical).
- **security**: `detect_local_subnets()` now filters virtual/container interfaces (docker*, veth*, virbr*, br-*, cni*, flannel*, podman*). Prevents attacker-created interfaces from widening the kill switch LAN exception (SHROUD-VULN-042, High).
- **security**: panic hook changed to fail-closed — kill switch rules are preserved on panic. Only socket and lock are cleaned so daemon can restart. Prevents attacker-triggered panics from disabling protection (SHROUD-VULN-043, High).
- **security**: `KillSwitch::Drop` now only warns, does not attempt rule cleanup. Eliminates double-cleanup race between panic hook and Drop (SHROUD-VULN-045, High).
- **security**: IPC `Reconnect` command now calls `handle_connect()` directly instead of disconnect-sleep-connect. Eliminates 2-second unprotected window where kill switch was disabled during reconnection (SHROUD-VULN-046, High).
- **security**: autostart `find_binary()` now prefers system-wide paths (`/usr/local/bin`, `/usr/bin`) over user-writable paths (`~/.cargo/bin`). Prevents autostart entry from pointing at attacker-controlled binary (SHROUD-VULN-047, Medium).

## [1.15.3] - 2026-02-13

### Fixed
- **security**: restart spawns child BEFORE releasing lock/socket — eliminates 100ms hijack window where an attacker could grab the instance lock and impersonate the daemon (SHROUD-VULN-031, Critical).
- **security**: `resolve_restart_path()` no longer falls back to user-writable `~/.local/bin` or `~/.cargo/bin` when the running binary is deleted. Refuses to restart and instructs user to restart manually (SHROUD-VULN-036, High).
- **security**: `is_actually_enabled()` returns `false` (not internal state) when sudo verification fails. Prevents silent kill switch desync where tray shows enabled but rules are gone after `sudo -K` (SHROUD-VULN-032, Critical).
- **security**: `TOGGLE_IN_PROGRESS` moved from `static AtomicBool` to struct-owned `bool` field. Eliminates static lifetime issues with task cancellation and concurrent toggle races (SHROUD-VULN-033, High).
- **security**: kill switch toggle "best-effort disable" path no longer persists `kill_switch_enabled = false` to config when iptables errors occur. Runtime state updates but config retains user intent (SHROUD-VULN-035, High).
- **security**: config migration (`migrate()`) no longer writes to disk. Migrated values are validated in-memory first; only persisted after `Config::validate()` passes. Prevents poisoned configs from surviving validation rejection (SHROUD-VULN-039, Medium).
- **security**: IPC restart path now uses `setsid` detachment and spawn-before-release pattern, matching tray restart. No longer disables kill switch before restart (was inconsistent with tray path).

## [1.15.2] - 2026-02-13

### Fixed
- **security**: `custom_doh_blocklist` entries are now validated as IPv4 addresses before interpolation into iptables/nft rulesets. Previously, arbitrary strings from config.toml were format-interpolated into the nft ruleset piped to `nft -f -`, enabling complete firewall bypass via nft scripting injection (SHROUD-VULN-022, Critical).
- **security**: `detect_local_subnets()` now validates that all detected subnets are RFC1918/link-local with prefix ≥ 8. Rejects `0.0.0.0/0` and public ranges that would open the kill switch to all traffic (SHROUD-VULN-021, Critical).
- **security**: removed legacy config migration from `~/.config/openvpn-tray/`. The migration followed symlinks and trusted arbitrary content on first load, bypassing all reload protections (SHROUD-VULN-024, High).
- **security**: IPC server uses bounded `take()` before `read_line()` — prevents OOM DoS from connections sending data without newlines. Previously `read_line()` allocated unbounded memory before the 64KB size check (SHROUD-VULN-026, Medium).
- **security**: VPN name validation now rejects all control characters (tab, form feed, vertical tab), not just newlines. Prevents log line injection via `\t` and `\r` (SHROUD-VULN-023, High).
- **security**: nmcli output parsing uses `rsplitn` (right-split) instead of `split(':')` for colon-delimited fields. Connection names containing `:` no longer corrupt field alignment (SHROUD-VULN-027, Medium).
- **security**: boot kill switch now uses `detect_local_subnets()` with RFC1918 fallback, consistent with runtime kill switch. Eliminates broader-than-intended LAN access during boot window (SHROUD-VULN-025, Medium).

## [1.15.1] - 2026-02-13

### Fixed
- **security**: `handle_disconnect()` no longer persists `kill_switch_enabled = false` to config — kill switch is suspended for the session only and restores on next VPN connect (SHROUD-VULN-015).
- **security**: `resolve_restart_path()` removes `$PATH` fallback, verifies ELF headers, and warns on inode mismatch with running binary (SHROUD-VULN-008).
- **health**: `check()` returns `HealthResult::Suspended` instead of `Healthy` during suspension — callers leave state unchanged instead of falsely affirming health (SHROUD-VULN-017).
- **health**: `suspend()` no longer resets failure counters — preserved for post-wake detection.
- **health**: ureq agent disables redirect following (`max_redirects(0)`) and adds 5s connect timeout (SHROUD-VULN-013).
- **killswitch**: nftables backend now uses `detect_local_subnets()` instead of hardcoded RFC1918 ranges, matching the iptables backend (SHROUD-VULN-016).

### Added
- **config**: `Config::validate()` now enforces bounds on `health_check_interval_secs` (0 or 10–300), `health_degraded_threshold_ms` (100–30000), `max_reconnect_attempts` (≤100), and `health_check_endpoints` (≤10, HTTPS-only, ≤256 chars each).
- **config**: 8 new config validation unit tests.
- **docs**: expanded `SECURITY.md` mitigations list with all v1.15.x hardening.

## [1.15.0] - 2026-02-13

### Added
- **security**: IPC peer PID logging — every non-trivial command is logged with the peer process ID and `(self)`/`(external)` source tag via `SO_PEERCRED` (SHROUD-VULN-001).
- **security**: config reload refuses security downgrades — `kill_switch_enabled`, `auto_reconnect`, `dns_mode`, `ipv6_mode`, and `block_doh` cannot be weakened via config file reload. Explicit IPC commands still work (SHROUD-VULN-002).
- **security**: reload trigger source logging — reload_configuration logs whether triggered by IPC, SIGHUP, or startup (NEW-C).
- **killswitch**: `lan_restrict_ports` config option — when true, only allows common LAN service ports (printing, file sharing, mDNS, SSDP, DNS, ICMP) instead of blanket LAN access (SHROUD-VULN-007).
- **killswitch**: auto-detect actual LAN subnets from system interfaces instead of hardcoding full RFC1918 ranges. Falls back to RFC1918 if detection fails (SHROUD-VULN-007).
- **killswitch**: `backend_name()` method for backend identification (NEW-A).
- **killswitch**: `Drop` implementation attempts emergency synchronous cleanup of firewall rules (NEW-B).
- **docs**: comprehensive threat model in `docs/SECURITY.md` documenting local attacker limitations and mitigations.

### Changed
- **security**: IPC socket created with restrictive umask (`0o077`) before `bind()` — eliminates TOCTOU permission window. Symlink check before stale socket removal prevents symlink attacks (SHROUD-VULN-004).
- **security**: `SHROUD_NMCLI` environment override gated behind `#[cfg(test)]` — production builds always use `nmcli` from PATH (SHROUD-VULN-005).
- **killswitch**: iptables jump rule (`-I OUTPUT -j SHROUD_KILLSWITCH`) now inserted LAST in script — chain is fully populated before traffic is directed to it, eliminating partial-chain window (SHROUD-VULN-006).
- **killswitch**: localhost DNS mode restricted to `127.0.0.1` and `127.0.0.53` only (was `127.0.0.0/8`), preventing rogue resolver attacks on other loopback addresses (SHROUD-VULN-009).
- **security**: sudoers rules (v3) scoped to `SHROUD_*` chain operations — bare `iptables -F`, bare `nft -f /path` no longer permitted. Only `nft -f -` (stdin) allowed (SHROUD-VULN-003).
- **security**: setup script logs moved from world-readable `/tmp` to `$XDG_DATA_HOME/shroud/` with `0600` permissions and cleanup-on-success trap (SHROUD-VULN-010).
- **validation**: VPN names now reject shell metacharacters (`;|&$\`<>!`) and ANSI escape sequences. Real-world names with `@`, `()`, Unicode still accepted (SHROUD-VULN-012).

## [1.14.0] - 2026-02-13

### Removed
- **gateway**: removed gateway mode entirely — violated three core principles: "Wrap, don't replace" (gateway replaced router routing), "Single purpose" (expanded scope to entire LAN), and "Leave no trace" (stopping gateway broke other devices' connectivity). Deleted `src/gateway/`, all CLI commands (`gateway`/`gw`), `[gateway]` config section (`GatewayConfig`, `AllowedClients`), `SHROUD_GATEWAY`/`SHROUD_GATEWAY_KS` chain cleanup, gateway help text, gateway tests, and `docs/GATEWAY.md`.

### Changed
- **deps**: `notify-rust` now uses `dbus` backend (no macOS-only transitive deps).

### Preserved
- Kill switch (host-level INPUT/OUTPUT), headless mode, auto-reconnect, health monitoring, state machine, tray, import, IPC, autostart, D-Bus, notifications — all untouched.

## [1.13.1] - 2026-02-10

### Fixed
- **killswitch**: kill switch rules are no longer torn down during `handle_restart()`. Previously, restarting the daemon disabled iptables rules before spawning the new instance; the new daemon started in `Disconnected` state and the kill switch restore check fired before `initial_nm_sync()` could detect the still-active VPN, leaving traffic unprotected. Rules now survive across restarts and the new instance adopts them via `sync_state()` in the constructor.
- **supervisor**: startup kill switch reconciliation now checks whether iptables rules already exist (restart / crash-recovery case) before deciding whether to re-enable. Three-branch logic: (1) rules present → preserve and sync shared state, (2) config enabled + VPN connected but no rules → re-enable, (3) config enabled but VPN not connected → defer until `handle_connect`.

## [1.13.0] - 2026-02-10

### Added
- **ipc**: protocol versioning with `Hello`/`HelloOk`/`VersionMismatch` handshake. Clients send `PROTOCOL_VERSION` on connect; server validates and rejects mismatched versions. Backward-compatible with unversioned legacy clients.
- **ipc**: `Version` command returns daemon binary version and protocol version.
- **config**: `health_check_endpoints` field — user-configurable list of health check URLs. When empty (default), built-in endpoints (Cloudflare, ifconfig.me, ipify) are used.
- **logging**: switched to `tracing` + `tracing-subscriber` with size-based rotating file writer, runtime toggle, and `--log-file` support.
- **supervisor**: `#[instrument]` spans on handlers and reconnect for richer context.
- **tests**: tracing subscriber initialized via `tests::common::init()` with `with_test_writer`.

### Changed
- **logging**: replaced `log` macros crate-wide with `tracing` macros; stderr filter uses runtime toggle; docs updated.
- **supervisor**: health checker now respects `health_degraded_threshold_ms` and `health_check_endpoints` from config (previously only interval was wired).
- **deps**: added `tracing`/`tracing-subscriber`, removed `log`/`env_logger`.
- **deps**: removed unused `tracing-appender` (Principle V — complexity is debt).

### Fixed
- **ipc**: handshake now validates protocol version and surfaces clear VersionMismatch errors.

## [1.12.5] - 2026-02-10

### Fixed
- **release**: `panic = "unwind"` so `install_panic_hook()` runs and cleans kill switch rules on panic.
- **mode**: call `check_headless_requirements` / `check_desktop_requirements` during `detect_mode()` with warnings.
- **state**: `StateMachine::handle_event` is `#[must_use]`; call sites updated.

### Changed
- **deps**: replace `once_cell` with `std::sync::LazyLock` (MSRV 1.85).
- **logging**: add TODO for `tracing` migration.

## [1.12.4] - 2026-02-09

### Added
- **docs**: `/// # Errors` rustdoc sections for public fallible APIs across killswitch, nm, gateway, IPC, config, import, logging, headless, dbus.

## [1.12.3] - 2026-02-09

### Fixed
- **daemon/exit**: replaced `process::exit` in daemon paths with graceful returns; `main` returns `ExitCode`; version flag parsed into `ParsedCommand::Version`.
- **logging**: removed legacy CLI parser/helpers; timestamp uses `libc::localtime_r` (no hand-rolled calendars).
- **IPC**: added connection semaphore and per-connection command cap.
- **headless**: shared linear backoff helper; shutdown now awaits aborted tasks with timeout.
- **dead code**: gated unused modules under `cfg(test)`; removed module-level `#[allow(dead_code)]`; removed backup `killswitch/firewall.rs.bak`.

### Added
- **util**: `backoff` helper for linear backoff + jitter.

## [1.12.2] - 2026-02-09

### Added
- **nm**: `NmClient` async trait + `NmCliClient` wrapper for free functions; `MockNmClient` for tests; `async-trait` dependency.
- **supervisor**: `VpnSupervisor::with_nm` for injection; handlers/reconnect/state_sync now use trait methods.
- **tests**: behavioral supervisor tests exercising commands/reconnect/state-sync against `MockNmClient` (no NetworkManager/iptables required).

### Changed
- **exports**: `nm` module re-exports `NmClient`, `NmCliClient`, `NmError`.

## [1.12.1] - 2026-02-09

### Changed
- **supervisor**: decomposed `VpnSupervisor` into `TrayBridge`, `ConfigStore`, and `TimingState`; centralized tray updates & notifications; unified config persistence. No behavioral changes intended.
- **handlers/state_sync/reconnect/event_loop**: migrated to new subcomponents; preserved reconnect debouncing and state sync semantics.
- **tests/clippy**: `cargo test` and `cargo clippy --all-targets -D warnings` passing.

## [1.12.0] - 2026-02-09

### Added / Improved
- `verify-killswitch`: colored output, tip when KS off; missing-chain tolerant; relaxed detection; JSON and tests improved.
- `tray`: recover from poisoned `cached_state` lock (no crash).
- `gateway`: `GatewayError` now uses `thiserror`.
- `logging`: flush error/warn logs to disk immediately.
- `config`: migrations use atomic temp-file+rename.
- `health`: HTTP checks no longer shell out to curl; use `ureq` with timeouts.
- `supervisor`: graceful shutdown (no `process::exit`); tray quit uses channel.
- `supervisor`: extracted `SwitchContext` and `ExitState` for clarity.
- `README`: MSRV badge updated to 1.85.

### Fixed
- Clippy warnings (`is_some_and`).

---

## [1.11.9] - 2026-02-08

### Fixed

- **clippy**: resolved `unnecessary_map_or` (use `is_some_and`) in `verify-killswitch` output code.

---

## [1.11.8] - 2026-02-08

### Improved

- **`verify-killswitch`** human output now uses colored symbols (✅/⚠/❌) and padded alignment for readability.
- Shows a friendly tip when the kill switch appears off (`shroud killswitch on`).

---

## [1.11.7] - 2026-02-08

### Fixed

- **`verify-killswitch`**: No longer errors when kill switch is disabled/missing; reports FAIL with details instead of exiting with iptables error.
- **`verify-killswitch`**: Added tests for missing-chain handling.

---

## [1.11.6] - 2026-02-08

### Fixed

- **`verify-killswitch`**: Tolerate `iptables -S` formatting for DHCP detection (matches `--dport 67/--sport 68` regardless of `-m udp`).
- **`verify-killswitch`**: Improve DNS tunnel/strict detection; note DoT drop missing as detail; updated tests.

---

## [1.11.5] - 2026-02-08

### Added

- **`shroud verify-killswitch`** — Read-only verification command that inspects live iptables/nftables to ensure the kill switch is active and correctly configured. Produces PASS/WARN/FAIL verdicts, supports `--json`, and `-v` to show raw rules.

### Security

- Verifies kill switch reality matches state machine belief (Principle VII) and exposes all rules for auditability (Principle XI).

---

## [1.11.4] - 2026-02-08

### Fixed

- **`shroud debug tail` auto-disables logging on exit** — Previously, `debug tail` enabled debug logging on the daemon but never disabled it when the user pressed Ctrl+C. The daemon would continue logging at DEBUG level indefinitely, flooding stderr in the terminal where it was launched. Now tracks whether it was the one that enabled logging: if so, sends `debug off` on exit; if logging was already on (user explicitly enabled it or via tray), leaves it alone.

---

## [1.11.4] - 2026-02-08

### Fixed

- **`shroud debug tail` auto-disables logging on exit** — Previously, `shroud debug tail` enabled debug logging on the daemon but never disabled it when the user pressed Ctrl+C. The daemon would continue logging at DEBUG level, flooding stderr in the terminal where it was launched. Now auto-sends `debug off` to the daemon when tail exits. Respects user intent: if logging was already enabled before tail started (via `shroud debug on` or tray toggle), it leaves it on.

---

## [1.11.4] - 2026-02-08

### Fixed

- **`shroud debug tail` auto-disables logging on exit** — Previously, `shroud debug tail` enabled debug logging on the daemon but never disabled it when the user pressed Ctrl+C. The daemon would continue logging at DEBUG level, flooding stderr in the terminal where it was launched. Now auto-sends `debug off` to the daemon when tail exits. Respects user intent: if logging was already enabled before tail started (via `shroud debug on` or tray toggle), it leaves it on.

---

## [1.11.3] - 2026-02-08

### Added

- **`shroud debug tail` level filtering** — Default output now shows INFO, WARN, and ERROR only, filtering out the DEBUG-level noise (NM polling every 2s, health check pings, tray state updates). Use `shroud debug tail -v` or `--verbose` for the full firehose. Uses `grep --line-buffered` for real-time output through the filter pipe.

### Fixed

- **Update script ETXTBSY bug** — `scripts/update.sh` and the inline fallback in `shroud update` used `cp` to overwrite the running binary, which fails silently with "Text file busy" (ETXTBSY) on Linux. The error was swallowed by `2>/dev/null || true`, causing `shroud restart` to spawn the old binary. Fixed by `rm -f` before `cp` (unlinks the inode so the running process keeps its mapping while the new binary takes the path).

- **Raw nmcli multiline log output** — nmcli stdout with embedded newlines was passed directly to `debug!()`, causing connection lines to appear without log prefixes. Now joined with ` | ` separator so all output stays on one properly-prefixed log line.

### Changed

- **Debug arg parsing refactored** — `parse_debug_args` now takes the full sub-argv slice instead of a single action string, enabling proper flag parsing for `tail -v`.

---

## [1.11.2] - 2026-02-08

### Added

- **`shroud update` restored** — Thin CLI wrapper that locates and runs `scripts/update.sh` (build, install, restart). Falls back to inline `cargo install` if script not found. No build tooling logic in the binary itself.

- **`shroud version --check` restored** — Quick binary staleness check comparing binary mtime vs `Cargo.toml` and `src/main.rs`. No `walkdir` dependency — just two file stats.

### Fixed

- **Raw nmcli output leaking into debug log** — Multi-line nmcli stdout was passed to `debug!()` with embedded newlines, causing connection lines (`Wired connection 1:802-3-ethernet:activated`, `lo:loopback:activated`) to appear without log prefixes. Now joined with ` | ` separator so all output stays on one properly-prefixed log line.

---

## [1.11.1] - 2026-02-08

### Fixed

- **`shroud debug dump` now works** — Previously returned "Command not implemented" because the `IpcCommand::DebugDump` handler was missing from the supervisor. Now returns a JSON snapshot of daemon internal state: state machine status, connected server, kill switch, auto-reconnect, available connections, switching status, reconnect retries, and config settings.

- **`shroud debug log-path` now works** — Same issue — `IpcCommand::DebugLogPath` had no handler. Now returns the log file path and whether debug logging is enabled.

- **`shroud debug tail` auto-enables logging** — Previously required running `shroud debug on` first, otherwise `tail -f` would hang on a nonexistent file. Now auto-enables debug logging on the daemon via IPC, creates the log file if missing, shows the last 50 lines immediately, and displays the file path.

- **Removed unreachable IPC catch-all** — All 20 `IpcCommand` variants are now explicitly handled in the supervisor, so the `_ => "Command not implemented"` fallback was dead code.

---

## [1.11.0] - 2026-02-07

### Changed

- **Notifications wired into supervisor** — The `notifications` module is now integrated into the VPN supervisor. All 37 `show_notification()` calls now route through `NotificationManager` with automatic category inference, per-category throttling, configurable urgency levels, and category-specific icons/timeouts. The old hardcoded 5-second `notify_rust::Notification` calls are replaced.

- **NotificationConfig added to Config** — New `[notifications]` section in `config.toml` with 11 fields: master enable, per-category toggles (connection, disconnection, reconnection, kill switch, error, health, first-run tips), throttle interval, timeout, and critical sound. All fields use `#[serde(default)]` for backward compatibility with existing configs.

### Removed

- **`shroud audit` command** — Moved to `scripts/audit.sh`. This was a developer tool (`cargo audit`) inside the user-facing binary, violating Principle VIII (One Binary, One Purpose).

- **`shroud update` command** — Moved to `scripts/update.sh`. This was a development workflow (`cargo install --path .`) baked into the production binary.

- **`shroud version --check` flag** — Removed source-vs-binary mtime comparison. `shroud version` now simply shows the version and daemon status.

- **`cli::install` module** — Marked `#[allow(dead_code)]` as its only consumer (`update` command) was removed.

---

## [1.10.1] - 2026-02-07

### Fixed

- **Kill switch idempotent guard** — IPC `killswitch on`/`off` commands now short-circuit when the kill switch is already in the desired state, preventing redundant iptables cleanup + VPN server IP re-detection (~600ms saved per no-op toggle).

- **Duplicate D-Bus activating events** — `VpnActivating` events are now suppressed when the VPN is already in `Connected` state (not just `Connecting`), eliminating duplicate "activating (external)" log entries.

### Changed

- **Kill switch toggle logging** — `toggle_kill_switch()` now logs the state transition direction (`true → false` / `false → true`) for easier debugging of unexpected toggles.

---

## [1.10.0] - 2026-02-07

### Added

- **Notification System** — New `notifications` module providing categorized, configurable, throttled desktop notifications for VPN events.

  - **`notifications::types`** — `NotificationCategory` enum (13 variants: Connected, Disconnected, ConnectionLost, Reconnecting, Reconnected, ReconnectionFailed, KillSwitchEnabled, KillSwitchDisabled, HealthDegraded, HealthRestored, ConnectionFailed, Error, FirstRun) with per-category icon names, urgency levels, default timeouts, sound policy, action support, and config key mapping. `Notification` builder with urgency/timeout/action overrides. `NotificationAction` with standard Reconnect/Dismiss factories. `Urgency` enum (Low/Normal/Critical).

  - **`notifications::manager`** — `NotificationManager` with `NotificationConfig` (11 configurable fields), per-category enable/disable, time-based throttling with dedup, suppressed-count tracking, and 10 convenience methods (`vpn_connected`, `vpn_disconnected`, `vpn_connection_lost`, `vpn_reconnected`, `reconnection_failed`, `connection_failed`, `kill_switch_changed`, `health_changed`, `error`, `first_run_tip`).

- **Test Coverage Overhaul (372 → 985 unit tests)** — Added 613 new unit tests across the entire codebase, increasing coverage from ~25% to ~35%. New pure-function modules extract testable logic from I/O-heavy code.

  - **New Modules (14 files):**
    - `supervisor::command_validation` — validate/format commands, parse kill-switch actions, tray-update decisions
    - `supervisor::reconnect_logic` — backoff calculation, reconnect decisions
    - `supervisor::connection_stats` — lifecycle statistics tracking
    - `supervisor::response_builder` — IPC response construction, NM event classification
    - `gateway::validation` — interface/subnet validation, route parsing
    - `gateway::rule_builder` — GatewayRule enum, NAT/forwarding builders, ForwardingState
    - `gateway::status_fmt` — GatewaySnapshot Display formatting
    - `killswitch::rules` — firewall rule generation, IP classification, chain validation
    - `killswitch::cleanup_logic` — cleanup command builders, iptables output parsing
    - `nm::parsing` — nmcli output parsing (active VPNs, connections, UUIDs)
    - `dbus::types` — NM state enums, D-Bus path parsing, failure reasons
    - `tray::state` — icon selection, tooltip, menu building, action mapping
    - `tray::drawing` — pixel-level icon drawing primitives, IconVariant
    - `headless::config` — stdin command parser, log levels, systemd messages
    - `headless::runtime_helpers` — lifecycle phases, signals, PID, watchdog
    - `cli::output` — duration formatting, list output, exit codes

  - **Expanded Tests in Existing Files:**
    - `state::machine` — 25 new transition tests (external connection, VPN changed, health recovery, wake/sleep, full lifecycle)
    - `health::checker` — 18 new tests (reset, suspend, thresholds, HealthResult traits)
    - `tray::service` — 17 new tests (SharedState, VpnCommand variants)
    - `ipc::protocol` — 35+ roundtrip serialization, validation, description tests
    - `killswitch::firewall` — 40 new tests (nft ruleset, KillSwitchError, DOH_PROVIDER_IPS validation)
    - `cli::handlers` — 40 new tests (args_to_command mapping, handle_response formatting)
    - `dbus::monitor` — 11 new tests (vpn_failure_reason, should_process_event dedup)
    - `config::settings` — 18 new tests (DnsMode, validate, HeadlessConfig, GatewayConfig)
    - `killswitch::sudo_check` — 7 new tests (SudoAccessStatus traits)
    - `killswitch::paths` — 5 new tests (binary path content, log_detected_paths)
    - `nm::connections` — 9 new tests (VpnType, VpnConnection, nmcli_command)
    - `ipc::server` — 4 new tests (validation failure, multi-command, Status roundtrip)
    - `ipc::client` — 3 new tests (error variants, connect_to_daemon)
    - `logging` — 15 new tests (timestamp, leap year, parse_level, Args)

---

## [1.9.1] - 2026-02-05

### Removed

- **End-to-End Tests** - Removed the entire E2E test suite (~2,400 lines), including:
  - `tests/e2e/` directory (Dockerfile, container scripts)
  - `tests/e2e.rs` (process-spawning integration tests)
  - `tests/chaos.rs` (chaos/fault injection tests)
  - `tests/stability.rs` (long-running stability tests)
  - `tests/common/process.rs` (ShroudProcess subprocess utilities)
  - `tests/common/harness.rs` (CleanupGuard test harness)

  **Rationale:** These tests were removed intentionally after extensive debugging revealed fundamental issues:
  
  1. **CI Reliability** - Process-spawning tests hung indefinitely in CI after completing successfully. The cargo test binary would finish all tests but never exit due to Tokio runtime shutdown issues. Multiple fix attempts (timeouts, watchdogs, background processes, non-blocking waits) failed to resolve the underlying issue.
  
  2. **No Coverage Value** - Subprocess-based tests spawn the shroud binary as a child process, which is not instrumented by tarpaulin. These tests consumed CI time without contributing to coverage metrics.
  
  3. **Redundant Coverage** - Integration tests using mock infrastructure (`MockNetworkManager`, `MockCommandExecutor`, `MockDbusClient`) cover the same code paths reliably and deterministically.
  
  4. **Maintenance Burden** - E2E infrastructure required constant debugging across different CI environments and caused repeated pipeline failures.

  The mock-based integration test suite provides equivalent coverage with better reliability and performance (~370 tests in <5 seconds).

- **Extended CI Workflow** - Removed `.github/workflows/extended-ci.yml` (duplicate of main CI with E2E tests).

### Added

- **Testing Documentation** - Added `docs/TESTING.md` documenting the testing strategy, explaining why E2E tests were removed, and providing manual testing instructions.

### Changed

- **CI Pipeline** - Simplified to a linear `check → test → coverage → msrv` flow without process-spawning tests.

- **Test Script** - Simplified `scripts/test.sh` to support unit, integration, security, regression, and coverage modes.

- **Security Tests** - Relaxed permission checks to only flag world-writable files/directories (the actual security concern) rather than any world access. Config files with 644 permissions are acceptable.

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
