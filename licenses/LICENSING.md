# Licensing Guide

*Last updated: February 2026*

This document provides a comprehensive overview of how licensing applies to every file and file type in the VPN Shroud project. It serves as the canonical reference for contributors, users, and anyone evaluating VPN Shroud's licensing for compatibility or compliance.

## Copyright Holder

All original work in this repository is copyright **Louis Nelson Jr.** unless otherwise noted.

See the [NOTICE](../NOTICE) file for the full identity mapping (Louis Nelson Jr. ↔ loujr ↔ lousclues).

## License Summary

| License | SPDX Identifier | Applies To | Full Text |
|---------|-----------------|------------|-----------|
| GNU GPL v3.0 or later | `GPL-3.0-or-later` | Source code, scripts, build files, tests, configs | [LICENSE](../LICENSE) |
| Commercial License | `LicenseRef-Commercial` | Same files as GPL (alternative license) | [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md) |
| CC BY 4.0 | `CC-BY-4.0` | Documentation | [LICENSE-DOCS.md](LICENSE-DOCS.md) |

Source code files are dual-licensed: users may choose either the GPL-3.0-or-later **or** the Commercial License. The SPDX identifier for dual-licensed files is:

```
GPL-3.0-or-later OR LicenseRef-Commercial
```

## LicenseRef-Commercial Definition

Per the [SPDX specification](https://spdx.github.io/spdx-spec/v2.3/other-licensing-information-detected/), custom license identifiers use the `LicenseRef-` prefix. The commercial license referenced by `LicenseRef-Commercial` is defined in [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md).

```
LicenseRef-Commercial:
  name: VPN Shroud Commercial License
  url: https://github.com/lousclues-labs/shroud/blob/main/licenses/LICENSE-COMMERCIAL.md
  contact: https://lousclues.com
```

---

## File-Type License Coverage Map

### Source Code

| File Pattern | License | SPDX Header Required | Notes |
|-------------|---------|---------------------|-------|
| `src/**/*.rs` | GPL-3.0-or-later OR LicenseRef-Commercial | Yes | All Rust source files |
| `tests/**/*.rs` | GPL-3.0-or-later OR LicenseRef-Commercial | Yes | All test files |

### Scripts

| File Pattern | License | SPDX Header Required | Notes |
|-------------|---------|---------------------|-------|
| `setup.sh` | GPL-3.0-or-later OR LicenseRef-Commercial | Yes | Installation script |
| `scripts/*.sh` | GPL-3.0-or-later OR LicenseRef-Commercial | Yes | Development/CI scripts |

### Build & Configuration Files

| File Pattern | License | SPDX Header Required | Notes |
|-------------|---------|---------------------|-------|
| `Cargo.toml` | GPL-3.0-or-later OR LicenseRef-Commercial | No (metadata file) | `license` field covers this |
| `Cargo.lock` | GPL-3.0-or-later OR LicenseRef-Commercial | No (generated) | Auto-generated |
| `.github/workflows/*.yml` | GPL-3.0-or-later OR LicenseRef-Commercial | Recommended | CI/CD pipelines |
| `codecov.yml` | GPL-3.0-or-later OR LicenseRef-Commercial | No | CI configuration |
| `autostart/*.desktop` | GPL-3.0-or-later OR LicenseRef-Commercial | No | XDG autostart entry |

### Documentation

| File Pattern | License | SPDX Header Required | Notes |
|-------------|---------|---------------------|-------|
| `docs/*.md` | CC-BY-4.0 | No | User and developer documentation |
| `README.md` | CC-BY-4.0 | No | Project README |
| `CONTRIBUTING.md` | CC-BY-4.0 | No | Contribution guide |
| `CHANGELOG.md` | CC-BY-4.0 | No | Release history |
| `tests/README.md` | CC-BY-4.0 | No | Test documentation |

### Legal & Administrative Documents

| File Pattern | License | Notes |
|-------------|---------|-------|
| `LICENSE` | N/A (is the license) | GPL-3.0 full text |
| `LICENSE-COMMERCIAL.md` | N/A (is the license) | Commercial license terms |
| `LICENSE-DOCS.md` | N/A (is the license) | Documentation license terms |
| `LICENSING.md` | N/A (this file) | License coverage guide |
| `CONTRIBUTOR-LICENSE.md` | N/A (is the CLA) | Contributor license agreement |
| `TRADEMARKS.md` | N/A (is the policy) | Trademark usage policy |
| `GOVERNANCE.md` | N/A (is the policy) | Succession and governance |
| `NOTICE` | N/A (is the notice) | Attribution and identity |
| `THIRD-PARTY-LICENSES` | N/A (is attribution) | Third-party license list |

### Assets

| File Pattern | License | Notes |
|-------------|---------|-------|
| `assets/*.policy` | GPL-3.0-or-later OR LicenseRef-Commercial | PolicyKit policy files |
| `assets/*.conf.example` | GPL-3.0-or-later OR LicenseRef-Commercial | Configuration examples |
| `assets/*.service` | GPL-3.0-or-later OR LicenseRef-Commercial | systemd service files |
| `assets/sudoers.d/*` | GPL-3.0-or-later OR LicenseRef-Commercial | sudoers configuration |
| Future logos/icons | Proprietary (Louis Nelson Jr.) | Not covered by GPL or CC BY 4.0 |

### Generated & Temporary Files

| File Pattern | License | Notes |
|-------------|---------|-------|
| `target/` | N/A | Build artifacts (not distributed) |
| `coverage/` | N/A | Generated reports (not distributed) |
| `systemd/` | GPL-3.0-or-later OR LicenseRef-Commercial | Generated systemd units |

---

## Canonical SPDX Header Formats

### Rust Source Files (`.rs`)

```rust
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
```

### Shell Scripts (`.sh`)

```bash
#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
```

Place the SPDX header immediately after the shebang line.

### TOML Configuration Files

```toml
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
```

> **Note:** For `Cargo.toml`, the `license` field in `[package]` serves as the SPDX declaration. An additional comment header is optional but not required.

### YAML Configuration Files (`.yml`, `.yaml`)

```yaml
# SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
# Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>
```

### Markdown Documentation (`.md` in `docs/`)

Markdown documentation files are covered by CC-BY-4.0 as declared in [LICENSE-DOCS.md](LICENSE-DOCS.md). No per-file header is required — the repository-level license declaration covers all documentation files.

If a per-file header is desired (e.g., for standalone distribution), use an HTML comment:

```markdown
<!-- SPDX-License-Identifier: CC-BY-4.0 -->
<!-- Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com> -->
```

---

## Header Verification

To verify that all source files have correct SPDX headers, use the following approach:

### Quick Check (grep)

```bash
# Find .rs files missing SPDX headers
find src/ tests/ -name '*.rs' -exec grep -L 'SPDX-License-Identifier' {} \;

# Find .sh files missing SPDX headers
find . -name '*.sh' -not -path './target/*' -exec grep -L 'SPDX-License-Identifier' {} \;

# Verify copyright line format
grep -rn 'Copyright (C)' src/ tests/ scripts/ setup.sh | grep -v 'Louis Nelson Jr.'
```

### Automated Check (CI)

A CI step can enforce header compliance:

```bash
#!/bin/bash
# check-headers.sh — Verify SPDX headers in source files
set -euo pipefail

ERRORS=0

# Check Rust files
while IFS= read -r file; do
    if ! head -1 "$file" | grep -q 'SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial'; then
        echo "MISSING SPDX header: $file"
        ERRORS=$((ERRORS + 1))
    fi
    if ! head -2 "$file" | grep -q 'Copyright (C) .* Louis Nelson Jr.'; then
        echo "MISSING/INCORRECT copyright: $file"
        ERRORS=$((ERRORS + 1))
    fi
done < <(find src/ tests/ -name '*.rs')

# Check shell scripts
while IFS= read -r file; do
    if ! head -3 "$file" | grep -q 'SPDX-License-Identifier'; then
        echo "MISSING SPDX header: $file"
        ERRORS=$((ERRORS + 1))
    fi
done < <(find . -name '*.sh' -not -path './target/*')

if [ "$ERRORS" -gt 0 ]; then
    echo "Found $ERRORS header issues."
    exit 1
fi

echo "All headers OK."
```

---

## Dependency License Compatibility

All third-party dependencies must be compatible with **both** the GPL-3.0-or-later license and the Commercial License. See [DEPENDENCY-AUDIT.md](DEPENDENCY-AUDIT.md) for the full compatibility framework and audit process.

---

## Questions

For licensing questions, see the [README](../README.md) or contact Louis Nelson Jr.:

- **GitHub:** [@loujr](https://github.com/loujr)
- **Website:** [lousclues.com](https://lousclues.com)
