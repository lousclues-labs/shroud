// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Promise Driven Development (PDD) canaries.
//!
//! These are the test-suite canaries of the PDD spine
//! (Principles -> Promises -> Canaries -> Ledger). Each test is written as an
//! assertion about a *promise* in `PROMISES.md`, not about a feature, and it
//! fails loudly when the promise stops being true.
//!
//! Traceability: every test names the promise (PRn) it guards and the canary
//! id (C-...) recorded in `PROMISES.md` and `AUDIT_FINDINGS.md`.
//!
//! A canary must live where the promise lives. Source-scanning canaries read
//! the tree from `CARGO_MANIFEST_DIR`; behavioral canaries drive real exported
//! types (`shroud::state`, `shroud::health`).

use std::fs;
use std::path::{Path, PathBuf};

use shroud::state::{Event, StateMachine, VpnState};

/// The release-signing key is the maintainer's GitHub-verified release-signing
/// key (loujr@github.com). Its fingerprint is a public pin, not a secret, whose canonical home is
/// `.well-known/security.txt` (the single source the installer reads); `README.md`
/// and `SECURITY.md` publish the same value. The installer does not hardcode it.
/// The canaries below are value-agnostic: they assert the published surfaces
/// agree with the canonical source, so they pass whether the surfaces carry the
/// placeholder token or the real 40-hex value, and fail only on disagreement (PR7).
const FINGERPRINT_PLACEHOLDER: &str = "<REPLACE_WITH_LOUSCLUES_FINGERPRINT>";

/// A well-formed fingerprint token is the placeholder or a 40-character hex string.
fn is_fingerprint_token(value: &str) -> bool {
    value == FINGERPRINT_PLACEHOLDER
        || (value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Extract the fingerprint published on a surface: the placeholder token, or a
/// 40-character hex run, taken from a line that mentions "fingerprint".
fn published_fingerprint(text: &str) -> Option<String> {
    for line in text.lines() {
        if !line.to_lowercase().contains("fingerprint") {
            continue;
        }
        if line.contains(FINGERPRINT_PLACEHOLDER) {
            return Some(FINGERPRINT_PLACEHOLDER.to_string());
        }
        for token in line.split(|c: char| !c.is_ascii_hexdigit()) {
            if token.len() == 40 {
                return Some(token.to_uppercase());
            }
        }
    }
    None
}

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Recursively collect every `.rs` file under `root`.
fn rs_files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    out
}

/// Drop whole-line comments so brand names and endpoints that appear only in
/// explanatory prose are not mistaken for behavior. Code lines are left intact,
/// so a URL such as `https://...` inside a string literal is never mangled.
fn code_only(src: &str) -> String {
    src.lines()
        .filter(|line| {
            let t = line.trim_start();
            !(t.starts_with("//") || t.starts_with('*') || t.starts_with("/*"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_surface(rel: &str) -> String {
    fs::read_to_string(manifest().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Extract a bash function body by name: the lines between `name() {` and the
/// closing `}` at column 0. Returns None if the function is absent.
fn bash_function_body(src: &str, name: &str) -> Option<String> {
    let header = format!("{name}() {{");
    let mut body: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in src.lines() {
        if !inside {
            if line.starts_with(header.as_str()) {
                inside = true;
            }
            continue;
        }
        if line == "}" {
            return Some(body.join("\n"));
        }
        body.push(line);
    }
    None
}

// ============================================================================
// P1 / Zero telemetry is architecture, not policy
// ============================================================================

/// PR1, canary C-TELEMETRY-EGRESS.
///
/// Negative space: the tree must contain no analytics / crash-reporting /
/// usage-telemetry SDK and no first-party analytics host. Fails the moment any
/// such endpoint or SDK string is added to code.
#[test]
fn zero_telemetry_no_analytics_sdk_or_endpoint_in_source() {
    const FORBIDDEN: &[&str] = &[
        "google-analytics.com",
        "googletagmanager.com",
        "sentry.io",
        "mixpanel.com",
        "segment.io",
        "posthog.com",
        "plausible.io",
        "amplitude.com",
        "datadoghq.com",
        "bugsnag.com",
        "telemetry.vpnshroud.org",
        "analytics.vpnshroud.org",
    ];

    let mut hits = Vec::new();
    for file in rs_files_under(&manifest().join("src")) {
        let text = match fs::read_to_string(&file) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let code = code_only(&text).to_lowercase();
        for &needle in FORBIDDEN {
            if code.contains(needle) {
                hits.push(format!("{}: {needle}", file.display()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "C-TELEMETRY-EGRESS breach (PR1): forbidden telemetry token in source:\n{}",
        hits.join("\n")
    );
}

/// PR2, canary C-TELEMETRY-DEFAULT-ENDPOINTS.
///
/// The single HTTP client's default endpoints must be third-party leak-check
/// IP-echo services only: all `https://`, none first-party.
#[test]
fn zero_telemetry_default_health_endpoints_are_third_party_only() {
    let cfg = shroud::health::checker::HealthConfig::default();
    assert!(
        !cfg.endpoints.is_empty(),
        "PR2: default health endpoints must exist"
    );

    const ALLOWED_HOSTS: &[&str] = &["1.1.1.1", "ifconfig.me", "api.ipify.org"];
    const FIRST_PARTY_MARKERS: &[&str] = &["vpnshroud", "lousclues"];

    for ep in &cfg.endpoints {
        assert!(
            ep.starts_with("https://"),
            "PR2: default endpoint is not https: {ep}"
        );
        let lower = ep.to_lowercase();
        for &marker in FIRST_PARTY_MARKERS {
            assert!(
                !lower.contains(marker),
                "PR2: first-party marker '{marker}' in default endpoint {ep}"
            );
        }
        assert!(
            ALLOWED_HOSTS.iter().any(|&h| ep.contains(h)),
            "C-TELEMETRY-DEFAULT-ENDPOINTS breach (PR2): {ep} is not on the third-party leak-check allowlist"
        );
    }
}

/// PR3, canary C-NO-IP-PERSIST.
///
/// The detected exit IP is used only for in-memory comparison and ephemeral
/// logs. It must never flow into a persistence sink.
#[test]
fn zero_telemetry_detected_exit_ip_is_never_persisted() {
    const IP_BINDINGS: &[&str] = &["detected_ip", "actual_ip"];
    const SINKS: &[&str] = &[
        "fs::write",
        ".write(",
        "write_all",
        "to_file",
        "save",
        "persist",
        "config.",
    ];

    let text = read_surface("src/health/checker.rs");
    let mut hits = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with('*') {
            continue;
        }
        if !IP_BINDINGS.iter().any(|&b| line.contains(b)) {
            continue;
        }
        for &sink in SINKS {
            if line.contains(sink) {
                hits.push(format!("{} -> {sink}", line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "C-NO-IP-PERSIST breach (PR3): detected exit IP reaches a persistence sink:\n{}",
        hits.join("\n")
    );
}

// ============================================================================
// P2 / The kill switch may never claim a state it cannot prove
// ============================================================================

/// PR4, canary C-CONNECTED-REQUIRES-HEALTH.
///
/// A leak signal (degraded / dead health, which includes an exit-IP mismatch)
/// must force the state machine out of `Connected`. Shroud cannot report
/// "connected" while a leak is detected.
#[test]
fn killswitch_connected_state_cannot_survive_a_leak_signal() {
    let mut sm = StateMachine::new();
    let _ = sm.handle_event(Event::UserEnable {
        server: "canary".to_string(),
    });
    let _ = sm.handle_event(Event::NmVpnUp {
        server: "canary".to_string(),
    });
    assert!(
        matches!(sm.state, VpnState::Connected { .. }),
        "setup precondition failed: expected Connected, got {:?}",
        sm.state
    );

    let _ = sm.handle_event(Event::HealthDegraded);
    assert!(
        !matches!(sm.state, VpnState::Connected { .. }),
        "C-CONNECTED-REQUIRES-HEALTH breach (PR4): stayed Connected after a leak signal: {:?}",
        sm.state
    );

    // A dead tunnel must escalate, never fall back to Connected.
    let _ = sm.handle_event(Event::HealthDead);
    assert!(
        matches!(sm.state, VpnState::Reconnecting { .. }),
        "C-CONNECTED-REQUIRES-HEALTH breach (PR4): dead tunnel did not escalate: {:?}",
        sm.state
    );
    assert!(
        !matches!(sm.state, VpnState::Connected { .. }),
        "C-CONNECTED-REQUIRES-HEALTH breach (PR4): reported Connected on a dead tunnel: {:?}",
        sm.state
    );
}

/// PR5, canary C-FAIL-VERDICT-NO-AFFIRM.
///
/// The user-facing protection affirmation must be bound to a passing live
/// verdict only. A failing verdict must warn, never affirm.
#[test]
fn killswitch_failure_verdict_never_affirms_protection() {
    let text = read_surface("src/cli/handlers.rs");
    let affirm = "Non-VPN traffic is blocked";
    assert!(
        text.contains(affirm),
        "PR5: expected the affirmative protection message to exist in handlers.rs"
    );

    for line in text.lines() {
        if line.contains(affirm) {
            assert!(
                line.contains("Verdict::Pass"),
                "C-FAIL-VERDICT-NO-AFFIRM breach (PR5): affirmative protection not bound to Pass: {}",
                line.trim()
            );
        }
        if line.contains("Verdict::Fail =>") {
            assert!(
                !line.contains(affirm) && !line.to_lowercase().contains("is working"),
                "C-FAIL-VERDICT-NO-AFFIRM breach (PR5): Fail arm affirms protection: {}",
                line.trim()
            );
        }
    }

    let fail_warns = text.lines().any(|l| {
        l.contains("Verdict::Fail =>")
            && (l.contains("NOT protecting") || l.to_lowercase().contains("leaking"))
    });
    assert!(
        fail_warns,
        "C-FAIL-VERDICT-NO-AFFIRM breach (PR5): the Fail arm does not warn about leaking"
    );
}

// ============================================================================
// P4 / Provider-agnosticism is a hard boundary
// ============================================================================

/// PR8, canary C-NO-PROVIDER-HARDCODING.
///
/// The core connection manager must contain no VPN provider brand name in code.
/// Brand names are allowed only in help-text examples (`src/cli/help.rs`) and in
/// comments, both excluded here.
#[test]
fn provider_agnostic_core_connection_manager_has_no_brand_names() {
    const CORE_DIRS: &[&str] = &[
        "src/nm",
        "src/state",
        "src/supervisor",
        "src/config",
        "src/killswitch",
    ];
    const BRANDS: &[&str] = &[
        "nordvpn",
        "mullvad",
        "protonvpn",
        "expressvpn",
        "surfshark",
        "windscribe",
        "cyberghost",
    ];

    let mut hits = Vec::new();
    for dir in CORE_DIRS {
        for file in rs_files_under(&manifest().join(dir)) {
            let text = match fs::read_to_string(&file) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let code = code_only(&text).to_lowercase();
            for &brand in BRANDS {
                if code.contains(brand) {
                    hits.push(format!("{}: {brand}", file.display()));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "C-NO-PROVIDER-HARDCODING breach (PR8): provider brand name in the core connection manager:\n{}",
        hits.join("\n")
    );
}

// ============================================================================
// P3 / Trust is pinned, not implied
// ============================================================================

/// PR6, canary C-INSTALL-FINGERPRINT.
///
/// The installer must NOT hardcode the fingerprint. It must derive the pin from
/// the canonical published file and keep the fail-closed gate functions.
#[test]
fn install_trust_setup_derives_pin_not_hardcoded() {
    let text = read_surface("setup.sh");
    for needle in [
        "pinned_fingerprint",
        "fingerprint_matches_pin",
        "require_pinned_signing_key",
        ".well-known/security.txt",
    ] {
        assert!(
            text.contains(needle),
            "C-INSTALL-FINGERPRINT breach (PR6): setup.sh is missing '{needle}'"
        );
    }
    assert!(
        published_fingerprint(&text).is_none(),
        "C-INSTALL-FINGERPRINT breach (PR6): setup.sh hardcodes a fingerprint literal; it must derive the pin from the canonical source"
    );
}

/// PR6, canary C-INSTALL-FINGERPRINT (piped-path threat model, AF-008).
///
/// The pin must derive from a local, in-tree file only. If the derivation ever
/// fetched the fingerprint over the network, a compromised server could serve a
/// matching script and a matching fingerprint, defeating the pin. This asserts
/// the source is a local path and the resolver/reader perform no network fetch.
#[test]
fn install_trust_pin_derivation_reads_local_only() {
    let text = read_surface("setup.sh");

    // The pin source must be a local, in-tree path, never a URL.
    let source_line = text
        .lines()
        .find(|l| l.trim_start().starts_with("SHROUD_SIGNING_FINGERPRINT_SOURCE="))
        .expect("C-INSTALL-FINGERPRINT breach (PR6): setup.sh does not set SHROUD_SIGNING_FINGERPRINT_SOURCE");
    assert!(
        !source_line.contains("://"),
        "C-INSTALL-FINGERPRINT breach (PR6): the pin source is a URL, not a local file: {}",
        source_line.trim()
    );

    // The resolver and reader must not perform any network fetch.
    const FETCH: &[&str] = &["curl", "wget", "http://", "https://", "ftp://", "/dev/tcp"];
    for func in ["script_dir", "pinned_fingerprint"] {
        let body = bash_function_body(&text, func).unwrap_or_else(|| {
            panic!("C-INSTALL-FINGERPRINT breach (PR6): setup.sh has no {func}() function")
        });
        let lower = body.to_lowercase();
        for &tok in FETCH {
            assert!(
                !lower.contains(tok),
                "C-INSTALL-FINGERPRINT breach (PR6): {func}() performs a network fetch ('{tok}'); the pin must derive from a local in-tree file only"
            );
        }
    }
}

/// PR7, canary C-PUBLICATION-AGREEMENT (test half; the CI job is the other).
///
/// The fingerprint must be identical across the installer pin and the three
/// publication surfaces, so a single-surface edit contradicts the others. The
/// check is value-agnostic: it passes with the placeholder token and with the
/// real value, failing only on disagreement or a missing value.
#[test]
fn install_trust_fingerprint_agrees_across_surfaces() {
    const SURFACES: &[&str] = &["README.md", "SECURITY.md", ".well-known/security.txt"];
    let mut seen: Vec<(&str, String)> = Vec::new();
    for &surface in SURFACES {
        let text = read_surface(surface);
        let fp = published_fingerprint(&text).unwrap_or_else(|| {
            panic!("C-PUBLICATION-AGREEMENT breach (PR7): no fingerprint found on {surface}")
        });
        assert!(
            is_fingerprint_token(&fp),
            "C-PUBLICATION-AGREEMENT breach (PR7): {surface} has an invalid fingerprint token: {fp}"
        );
        seen.push((surface, fp));
    }
    let (first_surface, first) = &seen[0];
    for (surface, fp) in &seen[1..] {
        assert_eq!(
            fp, first,
            "C-PUBLICATION-AGREEMENT breach (PR7): {surface} ({fp}) disagrees with {first_surface} ({first})"
        );
    }
}
