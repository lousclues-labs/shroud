# Principles

This is the values layer of Promise Driven Development (PDD) for VPN Shroud.

A principle here is not a slogan. It is a value that a real frustration forced
us to name, and it earns its place only by forbidding something the tool would
otherwise be tempted to do. Every principle below must spawn at least one
promise in [PROMISES.md](PROMISES.md). A principle that guards nothing has been
cut or sharpened until it does.

The longer-form philosophy that these operationalize lives in
[docs/PRINCIPLES.md](docs/PRINCIPLES.md). This file is the part we can prove.

The spine: **Principles -> Promises -> Canaries -> Ledger.** A claim VPN Shroud
makes about itself must be something the tool can prove, and the proof must
fail loudly when the claim stops being true.

---

## P1. Zero telemetry is architecture, not policy

**The frustration.** Every "privacy" VPN client eventually ships an analytics
SDK, a crash reporter, or an "anonymous usage" ping. A policy promise ("we do
not track you") is worthless the moment a well-meaning commit adds one line.

**What we value.** The absence of telemetry is a property of the binary, not a
line in a privacy policy. There is no first-party endpoint to phone home to,
because the code that would reach one does not exist and cannot pass review.

**This principle forbids:**
- Any network egress to a first-party analytics or telemetry endpoint.
- Bundling any analytics, crash-reporting, or usage-tracking SDK.
- Persisting a user's real IP or identifying connection metadata beyond what a
  VPN tunnel inherently requires to function.

Spawns promises: PR1, PR2, PR3.

---

## P2. The kill switch may never claim a state it cannot prove

**The frustration.** A kill switch that says "protected" while packets leak is
worse than no kill switch, because it converts caution into false confidence.
The dangerous version of this tool is the one that reports safety from an
intention ("the switch is enabled in config") instead of from evidence ("the
drop rule is live and traffic is actually blocked").

**What we value.** Protection is a claim backed by live inspection of the
firewall and the tunnel, never by a stored flag. If the tunnel is unhealthy or
leaking, the tool leaves the "connected" state loudly rather than sitting on a
comfortable lie.

**This principle forbids:**
- Reporting "protected" or "connected" while a leak path is open.
- Deriving the protection claim from stored intent instead of a proof verdict.
- Remaining in a "Connected" state after a leak or health failure is detected.

Spawns promises: PR4, PR5.

---

## P3. Trust is pinned, not implied

**The frustration.** Installers that pipe a fetched key straight into a
privileged action trust whatever the network hands them. "It downloaded, so it
is fine" is how a mirror or a man in the middle earns root on your machine.

**What we value.** The identity we trust is pinned in the code as a constant,
in the open, before anything privileged happens. A fetched key is compared
against that pin. The privileged branch runs only when they match; the unsafe
branch exists only to refuse.

**This principle forbids:**
- Writing to `/etc` (sudoers, polkit) on the strength of unverified material.
- Shipping an installer without a pinned signing fingerprint.
- Publishing that fingerprint in only one place, where a single edit could
  rewrite it without contradiction.

Spawns promises: PR6, PR7.

---

## P4. Provider-agnosticism is a hard boundary

**The frustration.** The moment a connection manager grows an `if provider ==
"acme"` branch, it starts to rot into a single-provider client with special
cases, and the "works with any VPN" claim quietly becomes false.

**What we value.** The core connection manager speaks NetworkManager and
standard VPN profiles, and knows nothing about any brand. Provider names may
appear in help-text examples and explanatory comments, never in control flow.

**This principle forbids:**
- Provider-specific branching or hardcoding in the core connection manager
  (`src/nm`, `src/state`, `src/supervisor`, `src/config`, `src/killswitch`).
- Encoding a provider's quirks as behavior instead of as user configuration.

Spawns promise: PR8.

---

## P5. A self-claim ships with a proof that fails loud, or it does not ship

**The frustration.** Claims rot silently. A README line that was true at commit
time drifts out of truth three refactors later, and nothing notices until a
user gets burned. Some invariants (no telemetry anywhere in the tree, no brand
name in the core) are cross-cutting, so no single unit test ever sees them.

**What we value.** Each promise is matched to a canary that lives where the
promise lives, and the loud failure of that canary is the alarm. Drift is
caught by a required, merge-blocking gate and recorded in the ledger so the
same regression cannot return unguarded.

**This principle forbids:**
- Adding or keeping a self-claim that has no canary.
- Merging past a red canary.
- Closing a ledger finding without naming the canary that prevents recurrence.

Spawns promises: PR9, PR10.

---

*These principles are the values layer. The commitments that discharge them,
and the canaries that prove them, are in [PROMISES.md](PROMISES.md). Drift is
tracked in [AUDIT_FINDINGS.md](AUDIT_FINDINGS.md).*
