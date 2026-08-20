# Promises

This is the commitments layer of Promise Driven Development (PDD) for VPN
Shroud. Each promise descends from a principle in [PRINCIPLES.md](PRINCIPLES.md)
and is guarded by a canary. A promise is narrow, observable, and falsifiable.

Rule: **no promise without a canary.** A claim we cannot guard is demoted to an
aspiration at the bottom of this file, not stated as a promise.

Canary types, matched to where each promise lives:
- **test** runs under `cargo test` (see [tests/pdd_canaries.rs](tests/pdd_canaries.rs)).
- **ci** is a required, merge-blocking job (see [.github/workflows/pdd-canaries.yml](.github/workflows/pdd-canaries.yml)).
- **install** ships in [setup.sh](setup.sh) and runs on the user's machine.
- **release** is verifiable on the published tag (a GPG-signed, GitHub-verified tag).

---

## From P1: Zero telemetry is architecture, not policy

### PR1. No first-party telemetry endpoint exists in the source or the built artifact
The tree contains no analytics, crash-reporting, or usage-telemetry SDK, and no
first-party analytics host (for example `telemetry.vpnshroud.org` or
`analytics.vpnshroud.org`), in code or in the shipped binary.

- Falsifiable by: adding any such SDK or endpoint string.
- Canary `C-TELEMETRY-EGRESS` (test + ci): `zero_telemetry__no_analytics_sdk_or_endpoint_in_source`
  scans `src/**/*.rs` with comment lines stripped; the ci job repeats the grep
  over the source and the release binary.

### PR2. The only outbound requests are user-configurable, third-party leak checks
The single HTTP client in the tree is the health checker
([src/health/checker.rs](src/health/checker.rs)). Its default endpoints are
exactly three third-party IP-echo services, all `https://`, none first-party:
`https://1.1.1.1/cdn-cgi/trace`, `https://ifconfig.me/ip`,
`https://api.ipify.org`. These probes traverse the VPN tunnel and exist to
detect leaks, not to report on the user. They are fully overridable in config.

- Falsifiable by: adding a first-party or non-`https` default endpoint.
- Canary `C-TELEMETRY-DEFAULT-ENDPOINTS` (test):
  `zero_telemetry__default_health_endpoints_are_third_party_only` asserts every
  default in `HealthConfig::default()` is `https://`, is on the third-party
  leak-check allowlist, and contains no first-party marker.

### PR3. The detected exit IP is never persisted
The public exit IP that a health check reads back is used only for in-memory
leak comparison and ephemeral log lines. It is never written to the config file
or any data file.

- Falsifiable by: writing the detected IP into persisted config or a data file.
- Canary `C-NO-IP-PERSIST` (test): `zero_telemetry__detected_exit_ip_is_never_persisted`
  scans [src/health/checker.rs](src/health/checker.rs) and fails if the detected
  IP binding appears on the same line as a persistence sink (`fs::write`,
  `.write(`, `save`, `persist`, `to_file`, or a `config.` assignment).

---

## From P2: The kill switch may never claim a state it cannot prove

### PR4. A leak signal forces the tool out of the Connected state
When a health check reports degraded or dead connectivity (which includes an
exit-IP mismatch, treated as a leak), the state machine must leave
`Connected`. The tool cannot report "connected" while a leak is detected.

- Falsifiable by: a transition table that keeps `Connected` on `HealthDegraded`
  or `HealthDead`.
- Canary `C-CONNECTED-REQUIRES-HEALTH` (test):
  `killswitch__connected_state_cannot_survive_a_leak_signal` drives the real
  `shroud::state::StateMachine` to `Connected`, feeds `Event::HealthDegraded`,
  and asserts the state is no longer `Connected`; it also asserts
  `HealthDead` moves a `Degraded` connection to `Reconnecting`.

### PR5. A failing kill-switch verdict never affirms protection
The user-facing protection message is derived from the live verification verdict
in [src/cli/handlers.rs](src/cli/handlers.rs). Only a `Verdict::Pass` prints the
affirmative "Non-VPN traffic is blocked." A `Verdict::Fail` prints a warning
that traffic may be leaking. The affirmative claim is never attached to a
non-passing verdict.

- Falsifiable by: routing an affirmative protection line under the `Warn` or
  `Fail` arm.
- Canary `C-FAIL-VERDICT-NO-AFFIRM` (test):
  `killswitch__failure_verdict_never_affirms_protection` reads the handler
  source and asserts the `Verdict::Fail` arm carries a warning and no
  affirmative protection phrase, and that the affirmative phrase appears only
  with `Verdict::Pass`.

---

## From P3: Trust is pinned, not implied

### PR6. The installer refuses privileged writes on a fingerprint mismatch, without hardcoding the pin
[setup.sh](setup.sh) does not hardcode the fingerprint. It reads the pin at
runtime from the canonical published file
([.well-known/security.txt](.well-known/security.txt)). Before any write to
`/etc` (the sudoers rule), if a signing key is presented it is reduced to a
fingerprint and compared to that pin. On mismatch the installer refuses the
privileged action. The matching branch is the only path that writes; the
mismatch branch exists only to fail.

- Falsifiable by: hardcoding a fingerprint literal in the installer, or
  presenting a key whose fingerprint is not the pin and having the sudoers rule
  written anyway.
- Canary `C-INSTALL-FINGERPRINT` (install + test): `require_pinned_signing_key`
  gates `do_install_sudoers` and reads `pinned_fingerprint`; `fingerprint_matches_pin`
  is the pure comparator. The test `install_trust_setup_derives_pin_not_hardcoded`
  asserts the installer derives the pin from the canonical source and ships no
  hardcoded literal.

### PR7. The published fingerprint agrees with the canonical source
The signing fingerprint is the maintainer's GitHub-verified release-signing key
(loujr@github.com). Its canonical home is [.well-known/security.txt](.well-known/security.txt),
the single source of truth the installer reads. [README.md](README.md) and
[SECURITY.md](SECURITY.md) publish the same value for readers. Because these
independent surfaces must agree, any single-surface edit is itself the alarm.
The check is value-agnostic: it holds for either the placeholder token or the
real 40-hex value, failing only on disagreement.

- Falsifiable by: changing the fingerprint on one surface only.
- Canary `C-PUBLICATION-AGREEMENT` (test + ci):
  `install_trust_fingerprint_agrees_across_surfaces` and the publication job both
  check that README.md and SECURITY.md agree with the canonical
  `.well-known/security.txt`, failing on disagreement or a missing value.

---

## From P4: Provider-agnosticism is a hard boundary

### PR8. No provider brand name appears in the core connection manager
The core modules (`src/nm`, `src/state`, `src/supervisor`, `src/config`,
`src/killswitch`) contain no VPN provider brand name in code. Brand names are
allowed only in help-text examples ([src/cli/help.rs](src/cli/help.rs)) and in
explanatory comments.

- Falsifiable by: adding a brand name to core control flow, for example
  `if provider == "..."`.
- Canary `C-NO-PROVIDER-HARDCODING` (test + ci):
  `provider_agnostic__core_connection_manager_has_no_brand_names` scans the core
  directories with comment lines stripped; the ci job repeats the scan across
  the whole surface.

---

## From P5: A self-claim ships with a proof that fails loud

### PR9. A required, merge-blocking gate runs the whole canary surface
Every push and pull request runs the canary suite. A final gate job depends on
all canary jobs and fails the check if any breach is present, so drift cannot
merge.

- Falsifiable by: a breach that still reports a green required check.
- Canary `C-CI-GATE` (ci): the `pdd-canary-gate` job in
  [.github/workflows/pdd-canaries.yml](.github/workflows/pdd-canaries.yml) is
  the required status.

### PR10. Every release is a GPG-signed, GitHub-verified tag
Releases are cut from annotated tags signed with the maintainer's GitHub-verified
release-signing key (loujr@github.com), the same key behind the project's
verified signed commits. GitHub marks the commits and tags Verified, and anyone
can confirm provenance with `git verify-commit` and `git verify-tag`. No separate
release-signing keypair, keyserver upload, or DNS record is stood up; a bespoke
signed receipt would be a second guard for a promise the signed tag already keeps
(promise inflation).

- Falsifiable by: an unsigned or unverifiable release tag.
- Canary `C-SIGNED-TAG` (release): the tag's GPG signature, verifiable with
  `git verify-commit` and `git verify-tag` and shown as Verified on GitHub.

---

## Aspirations (not yet promises: no canary yet)

These are things we want but cannot currently falsify cheaply. They are stated
here honestly as aspirations, not as promises, until a canary exists.

- **Reproducible builds.** We aspire to bit-for-bit reproducible release
  binaries. Until a reproducibility canary exists, this is not a promise.
- **Provider-agnostic import coverage.** We aspire to import profiles from every
  provider that exports OpenVPN or WireGuard. We guard the absence of brand
  hardcoding (PR8), not the completeness of import coverage.
