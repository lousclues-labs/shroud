# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Shroud, please report it privately.

Preferred: GitHub Security Advisories
- Open a new draft advisory and include steps to reproduce.

If advisories are unavailable, open a private issue or contact the maintainers directly.

Please include:
- A clear description of the issue
- Impact and potential exploitability
- Steps to reproduce (proof of concept if possible)
- Affected versions

We aim to acknowledge reports within 72 hours and provide a remediation plan as soon as possible.

## Supported Versions

Security fixes are provided for the latest released version. Older versions are not guaranteed to receive updates.

## Dependency Audits

Shroud uses cargo-audit to check dependencies against the RustSec Advisory Database.

Run locally:

```bash
./scripts/audit.sh

# Or via the CLI
shroud audit
```

If a vulnerability cannot be fixed immediately:
- Document the risk and mitigation
- Create a tracking issue
- Prioritize a patch in the next release
