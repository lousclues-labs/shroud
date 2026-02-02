# Releasing Shroud

## Pre-release Checklist

1. Update version in Cargo.toml
2. Update CHANGELOG.md with release notes
3. Ensure README.md and docs are current
4. Run formatting and checks:
   - cargo fmt --all
   - cargo clippy --all-targets --all-features -- -D warnings
   - cargo test --all
5. Run security audit:
   - ./scripts/audit.sh
   - or shroud audit
6. Build release binary:
   - cargo build --release

## Tag and Release

1. Create a git tag:
   - git tag -s vX.Y.Z -m "vX.Y.Z"
2. Push commits and tags:
   - git push
   - git push --tags
3. Create a GitHub Release with:
   - Tag vX.Y.Z
   - Release notes from CHANGELOG.md
   - Attached artifacts if desired

## Post-release

1. Verify installation from a clean environment
2. Monitor issue tracker for regressions
3. Schedule follow-up fixes if needed
