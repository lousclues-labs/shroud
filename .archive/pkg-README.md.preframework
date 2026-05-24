# `pkg/` -- source-project packaging contract for `lousclues-pkg`

This directory is the source-project side of the lousclues-pkg contract.
The `lousclues-labs/lousclues-pkg` release pipeline invokes
[`pkg/build.sh`](./build.sh) inside a per-distro container. The script
emits exactly one `.deb` or `.rpm` plus a manifest sidecar into a
caller-provided output directory.

The pattern is adopted from `lousclues-labs/vigil`. Shroud and vigil
keep their `pkg/build.sh` scripts in convergence. Divergences are
deliberate and listed at the bottom of this README.

---

## 1. Contract

Three inputs (environment), two outputs (`$OUTDIR` plus stdout summary),
one exit code.

| Direction | Channel                   | Shape                                                                   |
| --------- | ------------------------- | ----------------------------------------------------------------------- |
| In        | environment variables     | `DISTRO`, `VERSION`, `OUTDIR`                                           |
| Out       | `$OUTDIR/*.{deb,rpm}`     | exactly one artifact per invocation                                     |
| Out       | `$OUTDIR/*.manifest.json` | one manifest sidecar next to the artifact                               |
| Out       | stdout                    | `ARTIFACT=... SHA256=... SIZE=... MANIFEST=...` (one line)              |
| Out       | exit code                 | `0` success, `1` build failure, `2` invalid input                       |

`DISTRO` is one of `noble`, `jammy`, `bookworm`, `el9`, `fedora`. Anything
else exits `2`. `VERSION` is semver with no leading `v` and must match
`[package].version` in `Cargo.toml`. `OUTDIR` must be an absolute path.

Artifact names:

| `DISTRO`   | Output filename                                          |
| ---------- | -------------------------------------------------------- |
| `noble`    | `shroud_<VERSION>_amd64-noble.deb`                       |
| `jammy`    | `shroud_<VERSION>_amd64-jammy.deb`                       |
| `bookworm` | `shroud_<VERSION>_amd64-bookworm.deb`                    |
| `el9`      | `shroud-<VERSION>-1.el9.x86_64.rpm`                      |
| `fedora`   | `shroud-<VERSION>-1.fedora.x86_64.rpm`                   |

---

## 2. Optional knobs

| Variable                  | Default | Effect                                                                                  |
| ------------------------- | ------- | --------------------------------------------------------------------------------------- |
| `SHROUD_SKIP_DEPS`        | `0`     | Skip `apt-get`/`dnf` system-deps install when the caller image is already prepared.     |
| `SHROUD_SKIP_TOOLCHAIN`   | `0`     | Skip `rustup` + `gem install fpm`. Use with warm runners that mounted a toolchain.      |
| `SHROUD_CARGO_TARGET_DIR` | `target/` | Override `CARGO_TARGET_DIR`. CI mounts a cache at the default path.                   |
| `SHROUD_KEEP_STAGE`       | `0`     | Preserve the staged tree after exit for debugging `validate_stage` or fpm input.        |
| `SHROUD_MANIFEST_COMMIT`  | unset   | Override the `git_commit` recorded in the manifest. Must be 40 lowercase hex chars.     |
| `FPM_VERSION`             | `1.16.0` | fpm gem version pinned by the script. Bump only after re-running the reproducibility check. |
| `SOURCE_DATE_EPOCH`       | from `git log -1 --pretty=%ct` | Reproducible-build timestamp. Falls back to a pinned constant when no git. |

`SOURCE_SHA` (generic, exported by lousclues-pkg) is honoured as a
fallback for `SHROUD_MANIFEST_COMMIT`. `CARGO_BUILD_JOBS=N` is passed
straight through to cargo.

---

## 3. Reproducibility

Two passes of the script against the same source tree, same
`SOURCE_DATE_EPOCH`, must produce byte-identical artifacts. The
`pkg-build` workflow asserts this on every push to `main` (PRs skip
pass 2 to keep them fast).

The script handles reproducibility in two layers:

- **deb**: `strip-nondeterminism --timestamp="$SOURCE_DATE_EPOCH"`
  runs as a post-processor. fpm does not honour `SOURCE_DATE_EPOCH`
  inside its tar/gzip/ar invocations. ar entry mtimes, gzip header
  timestamps, and tar entry ordering all vary run-to-run without it.
  `install_deb_deps` pulls `strip-nondeterminism` from the Debian
  reproducible-builds project's apt repo.
- **rpm**: rpmbuild macros do the work. `strip-nondeterminism` has no
  `.rpm` handler and is not packaged for Fedora or EPEL, so running it
  on an rpm would be a no-op. fpm passes three rpmbuild macros instead:
  `use_source_date_epoch_as_buildtime 1`,
  `clamp_mtime_to_source_date_epoch 1`, and a pinned
  `_buildhost reproducible.shroud.local`.

Every staged file's mtime is pinned to `SOURCE_DATE_EPOCH` before fpm
packs the archive, with `find "$STAGE" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +`.
Compile-time stripping (`RUSTFLAGS="-C debuginfo=0 -C strip=symbols"`)
removes the largest source of binary-level variance.

---

## 4. Manifest sidecar

Each artifact gets a `${artifact}.manifest.json` next to it. The
lousclues-pkg sign-and-publish pipeline consumes it to pin attestations
to the source commit.

```json
{
   "artifact": "shroud_2.2.0_amd64-noble.deb",
   "sha256": "<64 hex chars>",
   "size_bytes": 1234567,
   "version": "2.2.0",
   "distro": "noble",
   "source_date_epoch": 1778976000,
   "git_commit": "<40 hex chars>"
}
```

`git_commit` resolution precedence:

1. `SHROUD_MANIFEST_COMMIT` (explicit override, validated as 40-hex)
2. `SOURCE_SHA` (generic export from lousclues-pkg, same validation)
3. `git rev-parse HEAD` (the common case)
4. `"unknown"` only outside CI. In CI an unresolved commit exits `1`
   because a silent `unknown` would break attestation reproducibility
   downstream.

The workflow validates the manifest with `jq`. The fields it checks:
`.sha256` against the artifact, `.version` against `Cargo.toml`,
`.distro` against the matrix leg, and `.git_commit` against a
40-lowercase-hex regex.

---

## 5. Local invocation

The seven negative cases that Layer 2 of the workflow runs, without
docker or fpm:

```bash
cd <repo>
ACTUAL=$(awk -F\" '/^version *=/ { print $2; exit }' Cargo.toml)

t() { local name=$1 expected=$2; shift 2; local out rc
      out=$("$@" 2>&1) && rc=0 || rc=$?
      printf '  %-30s exit %d (want %d)\n' "$name" "$rc" "$expected"; }

t "missing DISTRO"     1 env -u DISTRO   VERSION="$ACTUAL" OUTDIR=/tmp/out bash pkg/build.sh
t "missing VERSION"    1 env -u VERSION  DISTRO=noble OUTDIR=/tmp/out         bash pkg/build.sh
t "missing OUTDIR"     1 env -u OUTDIR   DISTRO=noble VERSION="$ACTUAL"       bash pkg/build.sh
t "unknown DISTRO"     2 env DISTRO=arch VERSION="$ACTUAL" OUTDIR=/tmp/out    bash pkg/build.sh
t "leading 'v'"        2 env DISTRO=noble VERSION="v$ACTUAL" OUTDIR=/tmp/out  bash pkg/build.sh
t "relative OUTDIR"    2 env DISTRO=noble VERSION="$ACTUAL" OUTDIR=rel/out    bash pkg/build.sh
t "VERSION mismatch"   2 env DISTRO=noble VERSION=99.99.99  OUTDIR=/tmp/out   bash pkg/build.sh
```

For an end-to-end build locally, install podman or docker and run the
script inside the same container the workflow uses:

```bash
podman run --rm -it \
    -v "$PWD:/src:Z" -w /src \
    -e DISTRO=noble -e VERSION="$(awk -F\" '/^version/{print $2;exit}' Cargo.toml)" \
    -e OUTDIR=/src/out \
    ubuntu:24.04 \
    bash pkg/build.sh
```

---

## 6. CI gate

[`.github/workflows/pkg-build.yml`](../.github/workflows/pkg-build.yml)
validates the contract. Three layers, fastest first, plus an aggregate
gate (`pkg-success`) that branch protection can require as a single check.

- **lint** (~30s): `shellcheck -x` and `bash -n` on `pkg/build.sh`.
- **input-tests** (~10s): the eight negative cases the script must
  reject before doing any work (the seven above plus a second
  unknown-distro variant).
- **build** (matrix, ~10-18 min per distro on cold cache): real
  container build on each supported distro, smoke install, manifest
  validation, installed-layout assertions, plus a reproducibility
  re-build on non-PR events. Cargo registry and target dir are
  cached between runs.

Trigger paths are scoped to packaging-relevant directories
(`pkg/**`, `Cargo.toml`, `Cargo.lock`, `assets/**`, `autostart/**`,
the workflow itself) so unrelated `src/**` changes do not burn
container minutes.

The installed-layout step uses an aggregating `FAILS=()` pattern: a
single run surfaces every problem instead of exiting on the first.

---

## 7. Differences from sibling project (vigil)

Shroud's `pkg/build.sh` tracks vigil's shape closely. The list below is
the complete set of intentional divergences. Anything outside this list
is debt and must be reconciled before merge.

1. **Privilege model**: sudoers 0440 + visudo validation in postinst
   (vigil uses file caps + setcap).
2. **Binary count**: shroud (vigil builds vigil + vigild).
3. **Generated artifacts**: none (vigil ships man pages + shell
   completions generated from the binary).
4. **Package-manager hooks**: none (vigil ships `hooks/apt/` and
   `hooks/dnf/`).
5. **Desktop integration**: `.desktop` + polkit policy +
   `update-desktop-database` refresh in postinst (vigil ships none;
   daemon is silent-by-default).
6. **License tag**: `GPL-3.0-or-later` (vigil: `GPL-3.0-only`).
7. **`fix-debian-deps.sh`**: not present (shroud's deb dep graph does
   not trigger the libsystemd0 skew).
8. **Runtime deps**: divergent by construction. shroud declares
   `network-manager`/`NetworkManager`, `dbus`, `iptables` (with
   `nftables` and `polkit` as soft); vigil declares its own surface.

Any divergence outside this list is debt. Fix it or document it here.

---

*Harmonize the shape. Keep the substance. Document the divergence.*
