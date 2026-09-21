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

Commits must carry a `Signed-off-by:` trailer. The `lousclues-pkg` release
gate reads the *tagged commit's* message and refuses to publish without one:

```bash
git log -1 --format=%B "v1.8.7" | grep -qiE '^Signed-off-by:[[:space:]]'
```

Enable the hook once per clone so this is automatic
(see [CONTRIBUTING.md](../CONTRIBUTING.md)):

```bash
git config core.hooksPath scripts/hooks
```

It cannot be fixed after tagging: moving the tag changes the GitHub source
tarball (the commit SHA is embedded in the archive's pax header), which breaks
the AUR `sha256sums` published against it.

### 2. Push

```bash
git push
git push --tags
```

### 3. Create GitHub Release

Pushing the tag triggers [release.yml](../.github/workflows/release.yml), which
builds the binary, attaches the tarball plus its `.sha256`, and publishes the
release. Verify rather than assume:

```bash
gh release view v1.8.7 --json tagName,isDraft,assets
```

---

## Publish to every channel

A release is not finished when the tag is pushed. Shroud ships through **four**
channels and each is a separate step. Versions 2.5.0 through 2.6.0 were tagged
but never reached crates.io, the AUR, or the package repository, because only
the GitHub half of this list was written down.

Verify all four afterwards; the commands are in
[Post-Release](#1-verify-every-channel).

### 1. crates.io

Manual. There is no CI automation for this.

```bash
cargo publish --dry-run          # packages + compiles from the packaged tree
cargo publish                    # needs a crates.io token (cargo login)
```

### 2. AUR (`vpn-shroud`)

Update the in-repo mirror and the AUR repository together. The checksum is of
the **GitHub source tarball for the tag**, so the tag must already be pushed:

```bash
curl -sL -o /tmp/v.tar.gz \
    "https://github.com/lousclues-labs/shroud/archive/v1.8.7.tar.gz"
sha256sum /tmp/v.tar.gz
```

Set `pkgver` and `sha256sums` in [aur/PKGBUILD](../aur/PKGBUILD) together --
bumping one without the other produces a PKGBUILD that cannot build. Regenerate
[aur/.SRCINFO](../aur/.SRCINFO) (`makepkg --printsrcinfo > .SRCINFO` on Arch),
then push both files to `ssh://aur@aur.archlinux.org/vpn-shroud.git` with the
commit message `Update to 1.8.7`.

### 3. lousclues packages (.deb / .rpm)

See [Multi-distro packaging](#multi-distro-packaging-deb--rpm) below for the
build matrix. The publish itself is a two-step, operator-present flow:

```bash
pkg lint shroud 1.8.7 --pre-release        # seven gates; all must pass

cd ~/src/lousclues-pkg                      # NOT the source project
make prepare-artifacts PROJECT=shroud VERSION=1.8.7   # YubiKey touch
pkg release shroud 1.8.7                              # YubiKey touch
```

Add `FORCE=1` to `prepare-artifacts` when re-running against a populated
`dist/` directory. Both steps require a physical YubiKey touch; a missed touch
surfaces as `gpg: signing failed: Timeout`.

### 4. GitHub Release

Covered above -- automated by `release.yml` on tag push.

---

## Post-Release

### 1. Verify every channel

Check each one rather than trusting the publish command's own output:

```bash
# GitHub
gh release view v1.8.7 --json tagName,assets

# crates.io
curl -s https://crates.io/api/v1/crates/vpn-shroud | jq -r .crate.max_version

# AUR
curl -s 'https://aur.archlinux.org/rpc/v5/info?arg[]=vpn-shroud' \
    | jq -r '.results[0].Version'

# lousclues packages -- deb metadata, then the served version
for d in noble jammy bookworm; do
    curl -fsI "https://pkg.lousclues.com/deb/dists/$d/InRelease" >/dev/null \
        && echo "$d InRelease ok"
done
curl -s https://pkg.lousclues.com/deb/dists/noble/main/binary-amd64/Packages \
    | awk '/^Package: vpn-shroud$/{f=1} f&&/^Version:/{print $2; exit}'
```

### 2. Verify Installation

Test from a clean environment:

```bash
git clone https://github.com/lousclues-labs/shroud.git
cd shroud
./setup.sh
shroud --version
```

### 3. Monitor

Watch the issue tracker for:
- Regressions
- Installation problems
- Unexpected behavior

### 4. Hotfix If Needed

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
2. Push the tag. `lousclues-pkg` is what produces the published
   artifacts -- this repo does not upload `.deb` / `.rpm` itself.
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
