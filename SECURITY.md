# Security Policy

> **Last reviewed:** 2026-05-24 (UTC). See [docs/SECURITY.md](docs/SECURITY.md)
> for the full policy, threat model, Supported Versions table, and IPC
> trust boundary.

For Shroud's full security policy, threat model, and vulnerability reporting
instructions, see [docs/SECURITY.md](docs/SECURITY.md).

## Release Signing

Releases are signed with the maintainer's GitHub-verified signing key
(loujr@github.com), the same key GitHub marks Verified on the project's signed
commits and tags. Its fingerprint is a public pin. The canonical copy lives in
[/.well-known/security.txt](.well-known/security.txt); this file and
[README.md](README.md) publish the same value. The installer
([setup.sh](setup.sh)) reads the pin from the canonical file (it is not
hardcoded in the script) and refuses privileged writes if a presented key does
not match it. A disagreement between these surfaces is a signal that one of them
was tampered with.

```
Fingerprint: 4EEFBCAFDB57ECFD00A0CA8A4A2D22286FC38416
```

Verify a release's provenance directly from its signed tag (no separate receipt
is produced; the tag itself is GPG-signed and shows as Verified on GitHub):

```bash
git verify-tag v<version>
```

## Reporting a Vulnerability

**Do not open a public issue for security vulnerabilities.**

Use GitHub's [Security Advisories](../../security/advisories/new) to report
vulnerabilities privately, or contact the maintainer (`loujr`) directly through
GitHub.

### Response Timeline

| Step | Target |
|------|--------|
| Acknowledge receipt | 72 hours |
| Initial assessment | 1 week |
| Fix or mitigation plan | 2 weeks |
| Public disclosure | After fix is available |

## Supported Versions

| Version | Supported          | Notes                                                |
|---------|--------------------|------------------------------------------------------|
| 2.4.x   | :white_check_mark: | Current series (v2.4.1 latest). Receives security fixes. |
| < 2.4   | :x:                | Superseded — please upgrade to 2.4.x.                |

See [docs/SECURITY.md](docs/SECURITY.md#supported-versions) for full details.
