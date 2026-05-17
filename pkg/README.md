# `pkg/` — source-project packaging contract for `lousclues-pkg`

This directory is the **source-project side of the lousclues-pkg contract**.
The `lousclues-labs/lousclues-pkg` release-build workflow invokes
[`pkg/build.sh`](./build.sh) inside a per-distro container, and the script
emits exactly one `.deb` or `.rpm` per invocation into a caller-provided
output directory.

The contract is intentionally minimal:

| Direction | Channel                  | Shape                                                   |
| --------- | ------------------------ | ------------------------------------------------------- |
| In        | environment variables    | `DISTRO`, `VERSION`, `OUTDIR`                           |
| Out       | `$OUTDIR/*.{deb,rpm}`    | exactly one artifact per invocation                     |
| Out       | stdout                   | a machine-readable `ARTIFACT=… SHA256=… SIZE=…` line    |
| Out       | exit code                | `0` success, `1` build failure, `2` invalid input       |

No other shared state. The script owns its own toolchain install, its own
staging directory, and its own cleanup. The wrapping workflow only has to
prepare a clean container, set the three env vars, run the script, and
collect the file.

This document explains how that contract is satisfied, why every knob exists,
and how to port the pattern to other `lousclues-labs` repositories.

---

## 1. The contract surface

[`pkg/build.sh`](./build.sh) reads three required environment variables:

| Variable  | Meaning                                                                                 |
| --------- | --------------------------------------------------------------------------------------- |
| `DISTRO`  | One of `noble`, `jammy`, `bookworm`, `el9`, `fedora`. Anything else exits **2**.        |
| `VERSION` | Semver string with no leading `v`. Must match `[package].version` in `Cargo.toml`.      |
| `OUTDIR`  | Absolute path. Created if missing. Receives exactly one `.deb` or `.rpm`.               |

Optional knobs (default off / unset):

| Variable                  | Effect                                                                                                 |
| ------------------------- | ------------------------------------------------------------------------------------------------------ |
| `SHROUD_SKIP_DEPS=1`      | Skip the `apt-get`/`dnf` system-deps install. Use when the calling image is already prepared.          |
| `SHROUD_SKIP_TOOLCHAIN=1` | Skip the `rustup` + `gem install fpm` step. Use with warm CI runners that mounted a toolchain volume.  |
| `SHROUD_CARGO_TARGET_DIR` | Path to a persistent cargo target dir to reuse across containers (caching).                            |
| `CARGO_BUILD_JOBS`        | Passed straight through to cargo. Defaults to all cores.                                               |
| `SOURCE_DATE_EPOCH`       | Reproducible-build timestamp. Auto-derived from `git log -1 --pretty=%ct` when unset and inside a repo. |

### Artifact naming

| `DISTRO`   | Output filename                                          |
| ---------- | -------------------------------------------------------- |
| `noble`    | `shroud_<VERSION>_amd64-noble.deb`                       |
| `jammy`    | `shroud_<VERSION>_amd64-jammy.deb`                       |
| `bookworm` | `shroud_<VERSION>_amd64-bookworm.deb`                    |
| `el9`      | `shroud-<VERSION>-1.el9.x86_64.rpm`                      |
| `fedora`   | `shroud-<VERSION>-1.fedora.x86_64.rpm`                   |

The script also writes a single trailing stdout line for the wrapping
workflow to grep:

```text
ARTIFACT=/path/to/file.{deb,rpm} SHA256=<hex> SIZE=<bytes>
```

---

## 2. Script anatomy — eight phases

`pkg/build.sh` is structured as eight numbered phases. Each phase is small
enough to read in one sitting, and the failure mode of any phase is loud
and actionable. The phase numbering is in the script source itself.

```text
0. Input validation
   - Validate DISTRO / VERSION / OUTDIR shape (semver, no leading 'v',
     absolute path, known distro).
   - Confirm VERSION matches Cargo.toml [package].version (drift guard).
   - Resolve REPO_DIR from $BASH_SOURCE so the script is location-agnostic.
   - Export CARGO_NET_RETRY=10, CARGO_INCREMENTAL=0, umask 022.
   - Derive SOURCE_DATE_EPOCH from git when available.

1. Staging directory
   - mktemp -d -t shroud-pkg-XXXXXXXX
   - trap cleanup EXIT INT TERM   (clean up even on CI cancellation)

2. System dependency install (skippable)
   - install_deb_deps  -> apt-get with --no-install-recommends, no language files
   - install_rpm_deps  -> dnf with install_weak_deps=False + tsflags=nodocs,
                          --allowerasing for curl-minimal swap, rubygem-json
                          for fpm's load-time json require

3. Toolchain ensure (skippable, idempotent)
   - rustup --profile minimal --no-modify-path  (skip if cargo already on PATH)
   - gem install --no-document fpm              (skip if fpm already present)

4. Build
   - cargo build --release --locked
     - --locked: fail if Cargo.lock would change (reproducibility)
     - profile.release in Cargo.toml already does lto + strip + opt-level=s

5. Stage assets to $STAGE
   - /usr/bin/shroud                                       (binary)
   - {/lib,/usr/lib}/systemd/system/shroud.service         (unit; path rewritten)
   - /etc/sudoers.d/shroud                                 (mode 0440)
   - /usr/share/polkit-1/actions/com.shroud.killswitch.policy
   - /usr/share/applications/shroud.desktop
   - /usr/share/doc/shroud/{README,LICENSE,CHANGELOG,docs/*.md}

6. fpm package emit (one invocation)
   - fpm_deb -> network-manager + dbus + iptables hard deps,
                nftables + polkit as Recommends, /etc/sudoers.d/shroud
                marked as config file
   - fpm_rpm -> NetworkManager + dbus + iptables, same config-file marker

7. Validation + machine-readable summary
   - Confirm exactly one artifact in $OUTDIR (contract check).
   - Print path, size, sha256.
   - Emit ARTIFACT=... SHA256=... SIZE=... for the wrapping workflow.
```

---

## 3. Performance optimizations — every knob has a reason

The script is designed for cold CI containers: every container starts
empty, every dep has to be re-fetched, every toolchain has to be
re-installed. Each optimization below is targeted at that profile.

| Layer       | Knob                                                     | Why                                                                                                                                          |
| ----------- | -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Input       | All shape + version-drift checks before any install      | Bad invocations fail in milliseconds, before any apt/dnf cycle.                                                                              |
| apt-get     | `-qq --no-install-recommends -o Acquire::Languages=none` | Skips weak deps + the per-package translation index downloads. Several seconds saved on cold images.                                         |
| dnf         | `--setopt=install_weak_deps=False --setopt=tsflags=nodocs` | Skips weak deps and `/usr/share/doc` writes inside the build container.                                                                      |
| dnf         | `--allowerasing`                                         | Lets dnf swap `curl-minimal` for `curl` on Rocky 9 / Fedora bases without erroring.                                                          |
| dnf         | `rubygem-json`                                           | fpm's `package/python.rb` is `require`d unconditionally at load time and needs `json`; RHEL/Fedora's `rubygems` doesn't pull it as a default. |
| rustup      | `--profile minimal --no-modify-path`                     | Installs only the smallest toolchain (rustc + cargo + std), no docs, no rust-analyzer.                                                       |
| rustup      | Skip if `cargo` already on PATH                          | Honour warm runners and local re-runs.                                                                                                       |
| gem         | `--no-document`                                          | Skips rdoc/ri generation, which is the slowest part of `gem install fpm`.                                                                    |
| gem         | Skip if `fpm` already present                            | Same idempotency story as rustup.                                                                                                            |
| cargo       | `CARGO_NET_RETRY=10`                                     | Tolerate transient registry hiccups inside short-lived CI containers.                                                                        |
| cargo       | `CARGO_INCREMENTAL=0`                                    | No incremental cache to reuse across containers; disabling saves disk + skips per-codegen-unit accounting.                                   |
| cargo       | `--release --locked`                                     | Locked: fail if `Cargo.lock` would change (reproducibility); release: smallest, fastest binary.                                              |
| cargo       | `[profile.release] lto + codegen-units=1 + strip`        | Already in `Cargo.toml`; no need to double-strip after build.                                                                                 |
| cargo       | `CARGO_TARGET_DIR` reuse via `SHROUD_CARGO_TARGET_DIR`   | Opt-in cache reuse when the workflow mounts a persistent volume.                                                                              |
| Staging     | `mktemp -d` + `trap cleanup EXIT INT TERM`               | Predictable cleanup even when CI cancels the job mid-run.                                                                                    |
| Staging     | `umask 022`                                              | Predictable file modes in the fpm tree so the package doesn't carry whatever umask the container shipped with.                               |
| Reproducibility | `SOURCE_DATE_EPOCH` from `git log -1 --pretty=%ct` | Stable timestamps inside the artifact when invoked from a git checkout.                                                                      |

---

## 4. The `pkg-build` CI workflow

[`.github/workflows/pkg-build.yml`](../.github/workflows/pkg-build.yml)
validates this contract on every change that could affect packaging.
It has three layers, fastest first, and an aggregate gate for branch
protection.

### Trigger gating

Runs on `pull_request` and `push` to `main` only when one of these paths
changes — `src/**` is *not* a trigger because `src/` is already validated
by [`ci.yml`](../.github/workflows/ci.yml) and we don't need to burn
container minutes on every code change:

```yaml
paths:
  - 'pkg/**'
  - 'Cargo.toml'
  - 'Cargo.lock'
  - 'assets/**'
  - 'autostart/**'
  - '.github/workflows/pkg-build.yml'
```

Also runs on `workflow_dispatch`.

### Layer 1 — `lint` (~30s)

- `shellcheck -x pkg/build.sh`
- `bash -n pkg/build.sh`

### Layer 2 — `input-tests` (~10s)

Seven negative cases the script must reject before doing any work:

1. Missing `DISTRO`
2. Missing `VERSION`
3. Missing `OUTDIR`
4. Unknown `DISTRO` value → exit 2
5. `VERSION` with leading `v` → exit 2
6. Relative `OUTDIR` → exit 2
7. `VERSION` mismatch vs `Cargo.toml` → exit 2

These mirror the local test set in section 6 below.

### Layer 3 — `build` (matrix, ~10–18 min per distro on cold cache)

Matrix over the five supported distros, each in its own container:

| `distro`   | `image`              | `ext` |
| ---------- | -------------------- | ----- |
| `noble`    | `ubuntu:24.04`       | `deb` |
| `jammy`    | `ubuntu:22.04`       | `deb` |
| `bookworm` | `debian:12`          | `deb` |
| `el9`      | `rockylinux:9`       | `rpm` |
| `fedora`   | `fedora:latest`      | `rpm` |

`fail-fast: false` so all five legs always finish — partial signal is
better than first-failure abort.

Each leg:

1. **Bootstrap checkout deps** (`shell: sh`, POSIX-only, since `bash` may not
   exist yet on the most minimal images): install `ca-certificates`, `git`,
   `curl`, `bash`. RPM side adds `--allowerasing` to handle the
   `curl-minimal → curl` swap.
2. **`actions/checkout@v4`** populates the working tree.
3. **Resolve version** from `Cargo.toml` and stash in `$GITHUB_OUTPUT`.
4. **`bash pkg/build.sh`** end-to-end, with `DISTRO`, `VERSION`, `OUTDIR`
   set — this exercises the same path `lousclues-pkg` will exercise in
   production.
5. **Verify exactly one artifact** in `out/`.
6. **Inspect contents** with `dpkg-deb --info/--contents` or `rpm -qip/-qlp`.
7. **Smoke install** with `dpkg -i --force-depends` or `rpm -i --nodeps`.
   For deb distros, `rm -f /etc/dpkg/dpkg.cfg.d/excludes` first — Ubuntu
   and Debian slim base images ship an exclude that strips
   `/usr/share/doc/*` at install time, which would silently drop our
   packaged docs and make the layout-verify step fail spuriously.
   Real end-user systems do not ship this exclude.
8. **Verify installed layout**: binary executable, sudoers mode 0440,
   systemd unit installed under `/lib/systemd/system` (deb) or
   `/usr/lib/systemd/system` (rpm) with the `/usr/local/bin/shroud` path
   rewritten to `/usr/bin/shroud`, docs present, polkit policy present,
   and `/usr/bin/shroud --help` exits 0.
9. **Upload artifact** via `actions/upload-artifact@v4` for download from
   the run summary (14-day retention).

### Aggregate gate

`pkg-success` depends on all three layers. Branch protection can require
this single check.

### Why `defaults.run.shell: bash` on the build job

GitHub Actions defaults to `/bin/sh` inside `container:` jobs. The `build`
job's `run:` blocks use `[[ ... ]]` conditionals which are bashisms, so
the job sets `defaults.run.shell: bash`. The two bootstrap steps run
*before* `bash` is guaranteed to exist in the most minimal images (you
can't `apt-get install bash` from a step that itself needs bash), so they
explicitly use `shell: sh` and stay POSIX-compatible.

---

## 5. Asset layout inside the produced package

| Path                                                       | Source                                  | Notes                                                                                          |
| ---------------------------------------------------------- | --------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `/usr/bin/shroud`                                          | `target/release/shroud`                 | Mode 0755.                                                                                     |
| `/lib/systemd/system/shroud.service` (deb)                 | `assets/shroud.service`                 | `/usr/local/bin/shroud` rewritten to `/usr/bin/shroud` at stage time.                          |
| `/usr/lib/systemd/system/shroud.service` (rpm)             | `assets/shroud.service`                 | Same rewrite.                                                                                  |
| `/etc/sudoers.d/shroud`                                    | `assets/sudoers.d/shroud`               | Mode 0440 (sudo refuses to load otherwise). Marked as fpm config file so upgrades don't clobber. |
| `/usr/share/polkit-1/actions/com.shroud.killswitch.policy` | `assets/com.shroud.killswitch.policy`   | Legacy polkit path; installed for desktops that prefer it.                                     |
| `/usr/share/applications/shroud.desktop`                   | `autostart/shroud.desktop`              | App launcher + autostart source.                                                               |
| `/usr/share/doc/shroud/README.md`                          | `README.md`                             |                                                                                                |
| `/usr/share/doc/shroud/LICENSE`                            | `LICENSE`                               |                                                                                                |
| `/usr/share/doc/shroud/CHANGELOG.md`                       | `CHANGELOG.md`                          |                                                                                                |
| `/usr/share/doc/shroud/docs/*.md`                          | `docs/*.md`                             |                                                                                                |
| `/usr/share/doc/shroud/shroud-headless.conf.example`       | `assets/shroud-headless.conf.example`   | Example config; never auto-loaded.                                                             |

Runtime dependencies declared to fpm:

| Layer  | deb                                                   | rpm                                                |
| ------ | ----------------------------------------------------- | -------------------------------------------------- |
| Hard   | `network-manager`, `dbus`, `iptables`                 | `NetworkManager`, `dbus`, `iptables`               |
| Soft   | `nftables` (Recommends), `polkit` (Recommends)        | n/a — rpm boolean deps not used                    |
| Soft   | `network-manager-openvpn` (Suggests)                  | n/a                                                |

---

## 6. Local validation — testing without docker

If you don't have docker / podman / fpm locally, you can still validate
the script's input-handling surface (everything before the build/stage/fpm
phases). The seven tests below match what Layer 2 of the workflow runs.

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

For a real end-to-end build locally, install podman or docker and run the
script inside the same container the workflow uses, e.g.:

```bash
podman run --rm -it \
    -v "$PWD:/src:Z" -w /src \
    -e DISTRO=noble -e VERSION="$(awk -F\" '/^version/{print $2;exit}' Cargo.toml)" \
    -e OUTDIR=/src/out \
    ubuntu:24.04 \
    bash pkg/build.sh
```

---

## 7. Iterative bug log — what the first runs caught

This pattern was hardened by running the workflow against real containers
and fixing what surfaced. Documented here so the next repo doesn't
relitigate the same ground.

### Run 1 — three independent failures across all five distros

| Symptom                                                       | Root cause                                                                                                                                  | Fix                                                                                       |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `[[: not found` on every leg                                  | GitHub Actions defaults to `/bin/sh` inside `container:` jobs; `[[ ... ]]` is a bashism.                                                    | `defaults.run.shell: bash` on the build job. Bootstrap steps kept on `shell: sh`.         |
| `curl-minimal ... conflicts with curl` on `el9`               | Rocky 9 base image ships `curl-minimal`; our `curl` install collides.                                                                       | `dnf install --allowerasing ...` in both the workflow bootstrap and `install_rpm_deps`.   |
| `fpm` crashed: `cannot load such file -- json (LoadError)`    | fpm 1.17.0 `require`s its python package handler at load time, which `require`s `json`; RHEL/Fedora's `rubygems` does not bundle it.        | Add `rubygem-json` to `install_rpm_deps`.                                                 |

### Run 2 — slim base image quirk

| Symptom                                            | Root cause                                                                                                                                                                | Fix                                                                                  |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `missing: README under /usr/share/doc/shroud` (deb) | Ubuntu/Debian slim images ship `/etc/dpkg/dpkg.cfg.d/excludes` which strips `/usr/share/doc/*` at install time, silently dropping our packaged docs during the smoke install. | `rm -f /etc/dpkg/dpkg.cfg.d/excludes` before `dpkg -i` in the smoke step.            |

Real end-user systems do not ship the dpkg exclude file, so the produced
`.deb` is correct — only the CI verification needed adjusting.

### Run 3 — green

---

## 8. Porting this pattern to another repo

For a fresh `lousclues-labs/<project>` that should also be packaged by
lousclues-pkg:

1. **Copy `pkg/build.sh` and this README.**
2. **Edit the asset stage step (`stage_assets()` in `pkg/build.sh`):**
   - Replace `/usr/bin/shroud` with your binary name.
   - Replace each `install -Dm…` line with the assets your project ships.
   - Drop the systemd unit / sudoers / polkit lines if your project doesn't need them.
3. **Edit the fpm depends / recommends / suggests lists** in `fpm_deb`
   and `fpm_rpm` to match your runtime dependencies. Use `--depends`
   for hard requirements only.
4. **Edit artifact naming** in `fpm_deb`/`fpm_rpm` if your project's
   distro-pkg name differs from its binary name.
5. **Copy `.github/workflows/pkg-build.yml`.** The matrix and three-layer
   structure are project-agnostic. Update:
   - The `paths:` triggers to match what changes packaging in your repo.
   - The "Verify installed layout" step to test paths your project
     installs.
   - The `--help` smoke command if your project's binary uses different
     flag syntax.
6. **Confirm `Cargo.toml` `[profile.release]` does the right thing.**
   This repo's profile is `lto = true, codegen-units = 1, strip = true,
   opt-level = "s"` — the script does *not* re-strip, so if your profile
   doesn't strip, add `strip target/release/<bin>` between phase 4 and
   phase 5.
7. **Push and let the workflow tell you what's wrong.** Real container
   quirks (like the three Run 1 failures above) usually only show up
   when you actually run the build. Plan for at least one iteration.

---

## 9. References

- [`pkg/build.sh`](./build.sh) — the script itself.
- [`.github/workflows/pkg-build.yml`](../.github/workflows/pkg-build.yml) — the CI workflow.
- [`aur/PKGBUILD`](../aur/PKGBUILD) — canonical reference for the asset install layout.
- [`docs/RELEASING.md`](../docs/RELEASING.md) — release workflow that consumes packages emitted by this contract.
- `lousclues-labs/lousclues-pkg` — the upstream packaging pipeline that invokes `pkg/build.sh` in production.
