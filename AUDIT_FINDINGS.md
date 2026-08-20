# Audit Findings

This is the ledger layer of Promise Driven Development (PDD) for VPN Shroud:
the memory that keeps drift from returning. Each finding records a claim that
drifted or shipped unguarded, why it mattered, the change that closed it, and
the canary that now fails loudly if it recurs.

Rules:
- Findings are numbered and severity-graded, and tracked from open to closed.
- A finding closed without a canary is not closed. "Fixed" is not a status; a
  named canary is.
- New self-claims are expected to add a promise, a canary, and, when they
  correct drift, a finding here. See [CONTRIBUTING.md](CONTRIBUTING.md).

Severity: **High** (a safety or trust claim could be false without notice),
**Medium** (a privacy or correctness claim could rot silently), **Low**
(a boundary could erode), **Info** (process gap).

| ID | Severity | Status | Promise | Canary |
|----|----------|--------|---------|--------|
| AF-001 | High | Closed | PR4, PR5 | C-CONNECTED-REQUIRES-HEALTH, C-FAIL-VERDICT-NO-AFFIRM |
| AF-002 | Medium | Closed | PR1, PR2, PR3 | C-TELEMETRY-EGRESS, C-TELEMETRY-DEFAULT-ENDPOINTS, C-NO-IP-PERSIST |
| AF-003 | High | Closed | PR6, PR7 | C-INSTALL-FINGERPRINT, C-PUBLICATION-AGREEMENT |
| AF-004 | Low | Closed | PR8 | C-NO-PROVIDER-HARDCODING |
| AF-005 | Info | Closed | PR9 | C-CI-GATE |
| AF-006 | Low | Closed | PR10 | C-SIGNED-TAG (git verify-tag) |
| AF-007 | Low | Closed | PR6, PR7 | C-INSTALL-FINGERPRINT (no literal + derive), C-PUBLICATION-AGREEMENT |
| AF-008 | Info | Closed | PR6 | C-INSTALL-FINGERPRINT, install_trust_pin_derivation_reads_local_only |

---

## AF-001: Protection and connection were assertable without proof

- **Severity:** High
- **Status:** Closed
- **Principle / Promise:** P2 / PR4, PR5

**What drifted.** The README states "Kill switch that actually works: Traffic
blocked when VPN drops. No leaks." and the tool reports "Connected" and prints
kill-switch reassurance. Nothing bound those affirmations to live evidence.
A refactor could have left an affirmative "you are protected" message on a
failing verification verdict, or kept the state at "Connected" through a
detected leak, and no test would have noticed.

**Why it mattered.** A kill switch that claims safety it cannot prove converts
caution into false confidence. This is the single most dangerous failure mode
for this tool, because the user acts on the claim.

**Closing change.** PDD retrofit (this change set). The affirmative protection
message remains bound to a passing verdict in
[src/cli/handlers.rs](src/cli/handlers.rs); the state machine already forces a
leak signal out of `Connected`. Both are now guarded.

**Canary that prevents recurrence.**
`killswitch_connected_state_cannot_survive_a_leak_signal`
(C-CONNECTED-REQUIRES-HEALTH) and
`killswitch_failure_verdict_never_affirms_protection`
(C-FAIL-VERDICT-NO-AFFIRM) in [tests/pdd_canaries.rs](tests/pdd_canaries.rs).

---

## AF-002: Zero telemetry was a policy claim, not an enforced property

- **Severity:** Medium
- **Status:** Closed
- **Principle / Promise:** P1 / PR1, PR2, PR3

**What drifted.** "No telemetry. No analytics. No phoning home." was stated in
the README and in [docs/PRINCIPLES.md](docs/PRINCIPLES.md) with no executable
guard. A single commit adding an analytics SDK, a first-party endpoint, or a
line that persisted the detected exit IP would have contradicted the claim
silently. The one HTTP client (health checks) also had no guard keeping its
default endpoints third-party and leak-check only.

**Why it mattered.** Privacy is the reason many users choose this tool. A
telemetry regression would break the core promise without any loud signal.

**Closing change.** PDD retrofit (this change set). No telemetry SDK or
first-party endpoint exists; default health endpoints remain three third-party
IP-echo services over `https`; the detected exit IP is used only in memory and
ephemeral logs.

**Canary that prevents recurrence.**
`zero_telemetry_no_analytics_sdk_or_endpoint_in_source` (C-TELEMETRY-EGRESS,
also a CI grep over source and the built artifact),
`zero_telemetry_default_health_endpoints_are_third_party_only`
(C-TELEMETRY-DEFAULT-ENDPOINTS), and
`zero_telemetry_detected_exit_ip_is_never_persisted` (C-NO-IP-PERSIST).

---

## AF-003: Install trust was implied, not pinned

- **Severity:** High
- **Status:** Closed
- **Principle / Promise:** P3 / PR6, PR7

**What drifted.** [setup.sh](setup.sh) wrote a passwordless sudoers rule to
`/etc/sudoers.d/shroud` with no pinned signing identity and no verification
step. Trust in the artifact was implied by the fact that it ran. There was no
published fingerprint to check against, and nothing to refuse a swapped key.

**Why it mattered.** A privileged install path that trusts unverified material
is how a compromised mirror or a man in the middle earns root. Trust must be
pinned in the open and checked before anything privileged happens.

**Closing change.** PDD retrofit (this change set). `setup.sh` gates the sudoers
write behind `require_pinned_signing_key`, whose mismatch branch exists only to
refuse. The pin is the fingerprint of the maintainer's GitHub-verified
release-signing key (loujr@github.com), read from the canonical published file
([.well-known/security.txt](.well-known/security.txt)) rather than hardcoded in
the script (see AF-007); README.md and SECURITY.md publish the same value. No new
key is provisioned (see AF-006). The mechanism and canaries are live.

**Canary that prevents recurrence.** `require_pinned_signing_key` /
`fingerprint_matches_pin` reading `pinned_fingerprint` in [setup.sh](setup.sh)
plus `install_trust_setup_derives_pin_not_hardcoded` (C-INSTALL-FINGERPRINT), and
the publication-agreement job (C-PUBLICATION-AGREEMENT) with its test half
`install_trust_fingerprint_agrees_across_surfaces`.

---

## AF-004: Provider-agnosticism had no boundary guard

- **Severity:** Low
- **Status:** Closed
- **Principle / Promise:** P4 / PR8

**What drifted.** "Works with any VPN provider" was a README claim. Provider
brand names appear legitimately in help-text examples and comments, but nothing
prevented a brand name from entering the core connection manager as behavior,
for example an `if provider == "..."` special case.

**Why it mattered.** The first provider-specific branch is where a
provider-agnostic manager begins rotting into a single-provider client, quietly
falsifying the claim.

**Closing change.** PDD retrofit (this change set). The core modules
(`src/nm`, `src/state`, `src/supervisor`, `src/config`, `src/killswitch`) carry
no brand name in code.

**Canary that prevents recurrence.**
`provider_agnostic_core_connection_manager_has_no_brand_names`
(C-NO-PROVIDER-HARDCODING), with a CI grep over the whole surface.

---

## AF-005: Cross-cutting invariants had no merge-blocking gate

- **Severity:** Info
- **Status:** Closed
- **Principle / Promise:** P5 / PR9

**What drifted.** Several invariants (a telemetry-free tree, a brand-free core,
fingerprint agreement across surfaces) are cross-cutting: no single unit test
sees them, and CI did not enforce them as a required check. Drift could merge.

**Why it mattered.** A promise with a canary that is not required to pass is a
suggestion. Drift merges the moment the gate is optional.

**Closing change.** PDD retrofit (this change set). A dedicated workflow runs
the canary surface and a final gate job aggregates the results.

**Canary that prevents recurrence.** The `pdd-canary-gate` job in
[.github/workflows/pdd-canaries.yml](.github/workflows/pdd-canaries.yml)
(C-CI-GATE). Branch protection should require this status.

---

## AF-006: Bespoke release-signing infrastructure was promise inflation

- **Severity:** Low
- **Status:** Closed
- **Principle / Promise:** P3, P5 / PR10

**What drifted.** The first PDD pass added a bespoke signed release receipt
(`RELEASE_RECEIPT.md` plus a detached `.asc`) and gestured at a separate
release-signing keypair, keyserver upload, and DNS TXT record. That is a second
guard for a promise (release provenance) that the maintainer's existing
GitHub-verified GPG-signed tags already keep. Standing up parallel trust
infrastructure is promise inflation, and the receipt signature was optional
anyway, so it proved nothing on its own. The published install pin also named
the lousclues master key (6C0ADEA327A0EC6DF44971FC460658C51682945B) rather than
the key that actually signs releases, so the pin did not equal the
release-signing identity GitHub verifies.

**Why it mattered.** Redundant trust infrastructure is not free: a second,
weaker guard invites confusion about which one is authoritative and rots
untended. The honest guard is the one that already exists and is already used.

**Closing change.** This change set. The receipt steps and their assets were
removed from [.github/workflows/release.yml](.github/workflows/release.yml), the
DNS TXT suggestion was dropped from
[.well-known/security.txt](.well-known/security.txt), and the provenance promise
(PR10) was repointed to the existing signed-tag mechanism. No new key, secret,
keyserver upload, or DNS record was added. The install-time fingerprint pin is
retained as the distinct canary that signed tags do not provide: it protects a
user running `setup.sh` on their own machine, without ever contacting GitHub.
The published pin was repointed from the lousclues master to the GitHub-verified
release-signing key (fingerprint 4EEFBCAFDB57ECFD00A0CA8A4A2D22286FC38416,
identity loujr@github.com), so the pin now equals the key that signs releases
and that GitHub marks Verified.

**Canary that closes it.** Release provenance is guarded by C-SIGNED-TAG: the
maintainer's GitHub-verified, GPG-signed commits and tags, verifiable by anyone
with `git verify-commit` and `git verify-tag`. User-machine install trust remains guarded by
C-INSTALL-FINGERPRINT (`require_pinned_signing_key` / `fingerprint_matches_pin`
in [setup.sh](setup.sh)) and cross-surface agreement by C-PUBLICATION-AGREEMENT.

---

## AF-007: The signing fingerprint was hardcoded as a literal in the installer

- **Severity:** Low
- **Status:** Closed
- **Principle / Promise:** P3 / PR6, PR7

**What drifted.** When the real lousclues fingerprint was pasted in, it was
written as a 40-hex literal directly inside [setup.sh](setup.sh) and duplicated
across the publication surfaces. A cryptographic pin must be committed in the
open, but embedding it as a magic literal in the installer is a maintenance and
clarity hazard: the same value lived in four hand-edited places and could drift.

**Why it mattered.** A hardcoded literal invites silent drift and obscures the
single source of truth. It also blurred the line between the canonical pin and
its human-readable publications.

**Closing change.** This change set. [.well-known/security.txt](.well-known/security.txt)
is now the single canonical source of the fingerprint. [setup.sh](setup.sh) reads
the pin from it at runtime via `pinned_fingerprint` (the same trust boundary as a
literal: both ship in the reviewed source tree, so the pin stays in the open and
is not implied). README.md and SECURITY.md publish the same value for readers,
kept in agreement by the publication canary. No fingerprint literal remains in
the installer.

**Canary that closes it.** C-INSTALL-FINGERPRINT now asserts the installer ships
no fingerprint literal and derives the pin from the canonical source
(`install_trust_setup_derives_pin_not_hardcoded`), and C-PUBLICATION-AGREEMENT
checks README.md and SECURITY.md against the canonical
[.well-known/security.txt](.well-known/security.txt).

---

## AF-008: The pin derivation reads a local file only, so the piped-install path is not a gap

- **Severity:** Info
- **Status:** Closed
- **Principle / Promise:** P3 / PR6

**What was investigated.** Since AF-007, the install-time pin reads the
fingerprint from the in-tree [.well-known/security.txt](.well-known/security.txt)
instead of a hardcoded literal. That is sound only if the file read ships inside
the same reviewed artifact the user chose to run. The specific worry is a piped
install (`curl ... | bash`): with no on-disk script tree, a derivation that
fetched `security.txt` over the network would let a compromised server serve a
matching script and a matching fingerprint, defeating the pin in the exact
scenario it exists to guard.

**Evidence from the code.**
- Piped install is not supported. The only documented method is a checkout:
  `git clone` then `./setup.sh` ([README.md](README.md) Quick Start).
  `build_binary()` in [setup.sh](setup.sh) calls `die "Cargo.toml not found"`
  when run outside a project tree, and it runs before the sudoers gate, so the
  fingerprint path is never reached without a working tree.
- The derivation reads a local file only. `pinned_fingerprint()` resolves
  `$SHROUD_SIGNING_FINGERPRINT_SOURCE` (a relative in-tree path) against
  `script_dir()` and `$PWD`, then reads it with `[ -r "$file" ]`. Neither
  `script_dir()` nor `pinned_fingerprint()` contains `curl`, `wget`, or a URL.
  Under a hypothetical pipe the source resolves to the current directory and is
  read locally or returns empty; it is never fetched, and an empty pin fails
  closed when a key is presented.

**Conclusion.** No piped-path gap. The pin and its source travel in the same
reviewed tree (a checkout), the same trust boundary AF-007 established. The
derivation is local only.

**Canary that keeps it true.** `install_trust_setup_derives_pin_not_hardcoded`
(C-INSTALL-FINGERPRINT) asserts the installer derives from the canonical source
and ships no literal. The new `install_trust_pin_derivation_reads_local_only`
asserts `SHROUD_SIGNING_FINGERPRINT_SOURCE` is a local path (no `://`) and that
`script_dir()` and `pinned_fingerprint()` perform no network fetch. If a future
edit made the derivation fetch remotely, that canary fails under `cargo test`
and the CI canary suite.
