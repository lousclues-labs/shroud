# Dependency License Audit Framework

*Last updated: February 2026*

## Purpose

Shroud is dual-licensed under GPL-3.0-or-later and a Commercial License. Every dependency must be compatible with **both** licenses. This document provides a framework for evaluating dependencies and maintaining license compliance.

## The Dual-License Compatibility Challenge

| License Type | GPL-3.0 Compatible? | Commercial License Compatible? | Can We Use It? |
|-------------|---------------------|-------------------------------|----------------|
| **MIT** | Yes | Yes | **Yes** |
| **Apache-2.0** | Yes (with GPL-3.0+) | Yes | **Yes** |
| **BSD-2-Clause** | Yes | Yes | **Yes** |
| **BSD-3-Clause** | Yes | Yes | **Yes** |
| **ISC** | Yes | Yes | **Yes** |
| **Unlicense** | Yes | Yes | **Yes** |
| **Zlib** | Yes | Yes | **Yes** |
| **CC0-1.0** | Yes | Yes | **Yes** |
| **MIT OR Apache-2.0** | Yes | Yes | **Yes** (choose Apache-2.0 or MIT) |
| **LGPL-2.1+** | Yes | Conditional — dynamic linking only | **Caution** |
| **LGPL-3.0+** | Yes | Conditional — dynamic linking only | **Caution** |
| **MPL-2.0** | Yes (file-level copyleft) | Conditional — MPL files must remain open | **Caution** |
| **GPL-2.0-only** | No (incompatible with GPL-3.0+) | No | **No** |
| **GPL-2.0-or-later** | Yes | No | **No** (blocks commercial) |
| **GPL-3.0-only** | Yes | No | **No** (blocks commercial) |
| **GPL-3.0-or-later** | Yes | No | **No** (blocks commercial) |
| **AGPL-3.0** | Compatible but viral | No | **No** |
| **SSPL** | No | No | **No** |
| **BSL (Business Source)** | No | Depends on terms | **No** (generally) |
| **Proprietary** | No | Depends on terms | **Probably No** |

### Key Rules

1. **Permissive licenses (MIT, Apache-2.0, BSD, ISC, etc.) are always safe.** They impose minimal obligations and are compatible with both GPL and commercial licensing.

2. **Copyleft licenses (GPL, LGPL, AGPL) block commercial licensing.** If a dependency is GPL-licensed, we cannot redistribute it under the commercial license. The dependency's copyleft would extend to Shroud.

3. **Weak copyleft (LGPL, MPL) requires care.** These can work with commercial licensing if the dependency is dynamically linked (LGPL) or if the copyleft is file-scoped (MPL). Since Rust links statically by default, LGPL dependencies are generally **not compatible** with the commercial license in a Rust project without careful structuring.

4. **Dual-licensed dependencies** — When a dependency offers a choice (e.g., "MIT OR Apache-2.0"), we choose the most permissive option for each use case.

---

## Current Dependency Audit

All current direct dependencies have been audited and are compliant:

| Dependency | License | Compatible? |
|-----------|---------|-------------|
| tokio | MIT | Yes |
| async-trait | MIT OR Apache-2.0 | Yes |
| ksni | MIT | Yes |
| tracing | MIT | Yes |
| tracing-subscriber | MIT | Yes |
| notify-rust | MIT OR Apache-2.0 | Yes |
| zbus | MIT | Yes |
| futures-lite | Apache-2.0 OR MIT | Yes |
| ctrlc | MIT OR Apache-2.0 | Yes |
| serde | MIT OR Apache-2.0 | Yes |
| toml | MIT OR Apache-2.0 | Yes |
| serde_json | MIT OR Apache-2.0 | Yes |
| dirs | MIT OR Apache-2.0 | Yes |
| walkdir | Unlicense OR MIT | Yes |
| libc | MIT OR Apache-2.0 | Yes |
| thiserror | MIT OR Apache-2.0 | Yes |
| ureq | MIT OR Apache-2.0 | Yes |
| rand | MIT OR Apache-2.0 | Yes |
| scopeguard | MIT OR Apache-2.0 | Yes |

**Status: All clear.** All direct dependencies use permissive licenses compatible with both GPL-3.0 and commercial licensing.

---

## Audit Process for New Dependencies

Before adding a new dependency to `Cargo.toml`, follow this checklist:

### 1. Identify the License

```bash
# Check a specific crate's license
cargo license -d | grep <crate-name>

# Or check on crates.io
# https://crates.io/crates/<crate-name>
```

### 2. Evaluate Compatibility

Use the compatibility table above. If the license is:

- **Green (MIT, Apache-2.0, BSD, etc.):** Proceed without concern.
- **Yellow (LGPL, MPL):** Stop and evaluate. In most cases, LGPL is incompatible with Shroud's commercial license due to Rust's static linking. Consult this document's compatibility table and consider alternatives.
- **Red (GPL, AGPL, SSPL, proprietary):** Do not add. Find an alternative.

### 3. Check Transitive Dependencies

A dependency may be permissively licensed, but its own dependencies might not be:

```bash
# Full dependency tree with licenses
cargo license --all-deps

# Or use cargo-about for detailed reporting
cargo about generate -o html > license-report.html
```

### 4. Document the Decision

If the dependency is approved, add it to [THIRD-PARTY-LICENSES](THIRD-PARTY-LICENSES) using this format:

```
  <crate-name> (https://crates.io/crates/<crate-name>) — <SPDX-ID>
```

### 5. Periodic Re-Audit

Run a full dependency audit periodically (recommended: before each minor/major release):

```bash
# Check all dependency licenses
cargo license

# Security audit (also good practice)
cargo audit

# Detailed license report
cargo about generate
```

---

## THIRD-PARTY-LICENSES Entry Template

When adding a new dependency, add an entry to [THIRD-PARTY-LICENSES](THIRD-PARTY-LICENSES) in this format:

```
  <crate-name> (https://crates.io/crates/<crate-name>) — <SPDX-License-Identifier>
```

For dependencies with notable license conditions:

```
  <crate-name> (https://crates.io/crates/<crate-name>) — <SPDX-License-Identifier>
    Note: <any relevant notes about license conditions>
```

---

## Tools

The following tools are useful for license auditing in Rust projects:

| Tool | Purpose | Install |
|------|---------|---------|
| `cargo-license` | List licenses of all dependencies | `cargo install cargo-license` |
| `cargo-about` | Generate detailed license reports | `cargo install cargo-about` |
| `cargo-deny` | Policy-based license checking (CI-ready) | `cargo install cargo-deny` |
| `cargo-audit` | Security vulnerability audit | `cargo install cargo-audit` |

### cargo-deny Configuration

For automated CI enforcement, consider adding a `deny.toml`:

```toml
[licenses]
unlicensed = "deny"
allow = [
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unlicense",
    "Zlib",
    "CC0-1.0",
]
deny = [
    "GPL-2.0",
    "GPL-3.0",
    "AGPL-3.0",
    "SSPL-1.0",
]
copyleft = "deny"
```

---

## Questions

For questions about dependency licensing, open an issue in the [Shroud repository](https://github.com/loujr/shroud) or contact Louis Nelson Jr. ([@loujr](https://github.com/loujr)).
