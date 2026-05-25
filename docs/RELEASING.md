# Releasing

How we ship new versions.

---

## Pre-Release Checklist

Before tagging a release:

### 1. Update Version

Bump the version in `Cargo.toml`:

```toml
[package]
version = "1.8.7"
```

### 2. Update Changelog

Move items from `[Unreleased]` to the new version in `CHANGELOG.md`:

```markdown
## [1.8.7] - 2026-02-03

### Added
- ...

### Fixed
- ...
```

### 3. Verify Documentation

- README reflects current features
- CLI help matches actual commands
- Config options are documented

### 4. Run All Checks

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
./scripts/audit.sh
```

All must pass. No exceptions.

### 5. Build Release Binary

```bash
cargo build --release
```

Test it manually:

```bash
./target/release/shroud --version
./target/release/shroud doctor
```

---

## Tag and Release

### 1. Create Git Tag

```bash
git add -A
git commit -m "Release v1.8.7"
git tag -s v1.8.7 -m "v1.8.7"
```

Sign the tag. This proves it came from a maintainer.

### 2. Push

```bash
git push
git push --tags
```

### 3. Create GitHub Release

1. Go to Releases on GitHub
2. Click "Draft a new release"
3. Select the tag
4. Title: `v1.8.7`
5. Body: Copy from CHANGELOG.md
6. Attach binaries if desired
7. Publish

---

## Post-Release

### 1. Verify Installation

Test from a clean environment:

```bash
git clone https://github.com/lousclues-labs/shroud.git
cd shroud
./setup.sh
shroud --version
```

### 2. Monitor

Watch the issue tracker for:
- Regressions
- Installation problems
- Unexpected behavior

### 3. Hotfix If Needed

If something's broken, fix it fast:

1. Fix the issue
2. Bump patch version (1.8.7 → 1.8.8)
3. Release again

Don't let users sit on a broken release.

---

## Version Numbering

Shroud follows [Semantic Versioning](https://semver.org/):

| Change | Example | Bump |
|--------|---------|------|
| Breaking change | CLI argument removed | Major (1.x.x → 2.0.0) |
| New feature | New command added | Minor (1.8.x → 1.9.0) |
| Bug fix | Crash fixed | Patch (1.8.7 → 1.8.8) |

For Shroud specifically:

- **Major**: Breaking config changes, removed commands
- **Minor**: New features, new config options
- **Patch**: Bug fixes, documentation, performance

---

## Multi-distro packaging (.deb / .rpm)

Shroud ships `.deb` and `.rpm` artifacts via the
`lousclues-labs/lousclues-pkg` release pipeline. That pipeline is the
consumer; this repository is the **producer**. As of v2.4.0 the
producer side is the shared
[`lousclues-labs/pkg-integration`](https://github.com/lousclues-labs/pkg-integration)
build framework (`pkg-framework`), vendored at v1.2.4. The framework
ships the deb/rpm pipeline once; shroud declares its package surface in
[`pkg/project.sh`](../pkg/project.sh).

The producer-consumer contract is:

- Inputs: `DISTRO` (one of `deb`, `rpm`), `VERSION`, `OUTDIR`
  environment variables.
- Output: exactly one `.deb` or `.rpm` in `$OUTDIR`.
- Side output: an `ARTIFACT=... SHA256=... SIZE=...` line on stdout.
- Exit codes: `0` success, `1` build failure, `2` invalid input.

The matrix of underlying base images (debian:12 / ubuntu:24.04 /
rockylinux:9 / fedora:latest) lives inside the vendored workflow at
[`.github/workflows/pkg-build.yml`](../.github/workflows/pkg-build.yml);
shroud no longer needs to enumerate distros directly.

The framework files are sha256-pinned by `pkg-framework verify`:

- [`pkg/build.sh`](../pkg/build.sh) -- thin entry point. Do not edit;
  drift fails CI.
- [`pkg/lib/framework.sh`](../pkg/lib/framework.sh),
  [`layout-check.sh`](../pkg/lib/layout-check.sh),
  [`input-tests.sh`](../pkg/lib/input-tests.sh),
  [`VERSION`](../pkg/lib/VERSION) -- vendored helpers and version pin.
- `.github/workflows/pkg-build.yml` -- vendored CI workflow template.

Project-specific behavior (description, dependencies, layout checks,
postinst body, fpm flag overrides) lives in
[`pkg/project.sh`](../pkg/project.sh).

When cutting a release:

1. Bump `Cargo.toml` `version` as described above. The framework's
   phase 0 will refuse to build if `VERSION` drifts from `Cargo.toml`.
2. Push the tag. The `release` workflow builds the binary tarball
   **and** the `.deb` / `.rpm` matrix (ubuntu:24.04, debian:12-slim,
   fedora:40) and attaches all of them to the GitHub release; the
   downstream `lousclues-pkg` pipeline then pulls those artifacts and
   publishes them through the apt / dnf repos.
3. If `pkg-build` is red on `main`, do not tag. The producer contract
   must be green for the consumer pipeline to succeed.

To bump the framework version: edit `FRAMEWORK_VERSION` in
`pkg/project.sh`, run `pkg-framework upgrade` to re-vendor the pinned
files, then `pkg-framework verify` to confirm zero drift.

---

## The Philosophy

Ship often. Ship small. Ship working code.

A release with one fix is better than a release with ten that aren't fully tested. Users can update frequently. Big releases are scary.

Working code today beats perfect code never.
