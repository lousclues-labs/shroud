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

Shroud ships `.deb` and `.rpm` artifacts for the noble, jammy, bookworm,
el9, and fedora distros via the `lousclues-labs/lousclues-pkg` release
pipeline. That pipeline is the consumer; this repository is the
**producer**, and the producer–consumer contract is defined entirely by
[`pkg/build.sh`](../pkg/build.sh).

The contract is:

- Inputs: `DISTRO`, `VERSION`, `OUTDIR` environment variables.
- Output: exactly one `.deb` or `.rpm` in `$OUTDIR`.
- Side output: a `ARTIFACT=… SHA256=… SIZE=…` line on stdout.
- Exit codes: `0` success, `1` build failure, `2` invalid input.

Every change that could affect packaging is validated end-to-end by the
`pkg-build` workflow at
[`.github/workflows/pkg-build.yml`](../.github/workflows/pkg-build.yml).
The workflow runs the script in real per-distro containers, installs
the produced artifact, and verifies the installed layout. The full
design — phases, performance knobs, CI structure, asset layout, local
validation recipes, and a porting checklist for new repos — is
documented in [`pkg/README.md`](../pkg/README.md).

When cutting a release:

1. Bump `Cargo.toml` `version` as described above. `pkg/build.sh`'s
   phase 0 will refuse to build if `VERSION` drifts from `Cargo.toml`.
2. Push the tag. `lousclues-pkg` is what produces the published
   artifacts — this repo does not upload `.deb`/`.rpm` itself.
3. If `pkg-build` is red on `main`, do not tag. The producer contract
   must be green for the consumer pipeline to succeed.

---

## The Philosophy

Ship often. Ship small. Ship working code.

A release with one fix is better than a release with ten that aren't fully tested. Users can update frequently. Big releases are scary.

Working code today beats perfect code never.
