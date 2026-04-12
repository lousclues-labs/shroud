# Contributing to VPNShroud

You want to help. That's appreciated.

Before you write any code, read the [Principles](docs/PRINCIPLES.md). Every contribution should align with them. If your idea contradicts a principle, let's talk about it first. Either the idea needs adjusting, or maybe the principle does.

---

## The Short Version

1. Read the [Principles](docs/PRINCIPLES.md)
2. Fork the repo
3. Make your changes
4. Run `cargo fmt && cargo clippy -- -D warnings && cargo test`
5. Submit a PR

---

## Contributor License

VPNShroud is dual-licensed. By submitting a pull request, you agree to the terms in [CONTRIBUTOR-LICENSE.md](licenses/CONTRIBUTOR-LICENSE.md). The short version: you keep your copyright, you license your contribution under GPL-3.0, and you grant Louis Nelson Jr. (the project maintainer and sole copyright holder) permission to include it in the commercial license too. Your code always stays open source.

---

## The Principles, Summarized

These aren't just nice words. They're the filter for every decision.

| Principle | What It Means For Contributions |
|-----------|--------------------------------|
| **Wrap, Don't Replace** | Don't reinvent NetworkManager. Enhance it. |
| **Fail Loud, Recover Quiet** | Errors must be visible. Recovery must be seamless. |
| **Leave No Trace** | If VPNShroud stops, the system should be clean. |
| **The User Is Not the Enemy** | No telemetry. No analytics. No phoning home. |
| **Complexity Is Debt** | Every dependency needs a damn good reason. |
| **Speak the System's Language** | Use systemd, D-Bus, XDG. Be a native citizen. |
| **State Is Sacred** | The state machine is truth. Don't work around it. |
| **One Binary, One Purpose** | Keep it simple. One executable. |
| **Respect the Disconnect** | Sometimes the user wants to be offline. That's fine. |
| **Built for the Quiet Majority** | Make it work for people who won't file bug reports. |
| **Security Through Clarity** | Auditable. Explainable. No magic. |
| **We Ship, Then Improve** | Working code today beats perfect code never. |

---

## Development Setup

### Dependencies

**Arch:**
```bash
sudo pacman -S networkmanager networkmanager-openvpn networkmanager-wireguard \
    iptables nftables rust
```

**Debian/Ubuntu:**
```bash
sudo apt install network-manager network-manager-openvpn network-manager-wireguard \
    iptables nftables rustc cargo
```

**Fedora:**
```bash
sudo dnf install NetworkManager NetworkManager-openvpn NetworkManager-wireguard \
    iptables nftables rust cargo
```

### Build and Test

```bash
git clone https://github.com/loujr/shroud.git
cd shroud

cargo build           # Debug build
cargo test            # Run tests
cargo clippy          # Lint
cargo fmt             # Format

RUST_LOG=debug cargo run   # Run with debug logging
```

See [Development Setup](docs/DEVELOPMENT.md) for the full guide.

---

## Before You Submit

Every PR must pass:

```bash
cargo fmt             # Format code
cargo clippy -- -D warnings  # No lint warnings
cargo test            # All tests pass
```

Also recommended:

```bash
cargo audit           # Check for vulnerable dependencies
```

These run in CI. Save yourself the round trip.

---

## What Makes a Good Contribution

### Great Contributions

- **Bug fixes** -- especially with tests
- **Documentation** -- typos, clarity, examples
- **Performance** -- with benchmarks showing the improvement
- **Cross-distro fixes** -- Arch, Debian, Fedora, openSUSE
- **Security hardening** -- always welcome

### Discuss First

Open an issue before implementing:

- New CLI commands
- New config options
- New dependencies
- Changes to the state machine
- Architectural changes

Best to make sure it fits before you invest time.

### Out of Scope

These are intentional limits. Don't try to "improve" Shroud by adding:

| Feature | Why Not |
|---------|---------|
| macOS/Windows support | Shroud is Linux-focused. That's the point. |
| GUI application | CLI-first with a tray icon. |
| Built-in VPN protocols | Shroud wraps NM, it doesn't replace it. |
| Telemetry/analytics | No phoning home. Ever. |
| Auto-updates | The user controls their system. |

---

## The PR Process

1. **Fork** the repository
2. **Create a branch** from `main`:
   ```bash
   git checkout -b fix/describe-the-thing
   # or
   git checkout -b feat/new-thing
   ```
3. **Make your changes**
4. **Add tests** if you're adding functionality
5. **Update docs** if behavior changes
6. **Run the checks**:
   ```bash
   cargo fmt && cargo clippy -- -D warnings && cargo test
   ```
7. **Commit** with a clear message:
   ```
   fix: Kill switch cleanup on SIGTERM
   
   The cleanup handler wasn't being called when receiving SIGTERM
   in headless mode. Added signal handler registration.
   
   Fixes #123
   ```
8. **Push** and open a PR

---

## Commit Messages

```
type: Short description (50 chars max)

Longer explanation if needed. Wrap at 72 characters.
Explain what and why, not how.

Fixes #123
```

**Types:**
- `fix` -- bug fixes
- `feat` -- new features
- `docs` -- documentation only
- `refactor` -- code restructuring
- `test` -- adding tests
- `chore` -- build/CI/tooling

---

## Code Style

### The Basics

- Run `cargo fmt`
- Handle all errors explicitly
- No `unwrap()` in production code
- Comment *why*, not *what*

### Error Handling

```rust
// ✅ Good
match connection.activate().await {
    Ok(()) => tracing::info!("Connected"),
    Err(e) => {
        tracing::error!("Connection failed: {}", e);
        return Err(e.into());
    }
}

// ❌ Bad: Silent failure
let _ = connection.activate().await;

// ❌ Bad: Panic
connection.activate().await.unwrap();
```

### Logging

- Use `tracing` macros (`tracing::info!`, `tracing::warn!`, `tracing::error!`, `tracing::debug!`, `tracing::trace!`).
- Prefer `#[instrument(skip(self, ...), fields(...))]` on async handlers to capture context.
- `RUST_LOG=debug cargo run` enables debug logs to stderr; runtime toggle writes to `~/.local/share/shroud/debug.log`.
- Tests: call `tests::common::init()` to initialize tracing subscriber.

```rust
// Example
#[instrument(skip(self), fields(conn = %conn_name))]
pub async fn handle(&mut self, conn_name: &str) {
    info!(%conn_name, "handling connection");
}
```

---

## Testing

### Unit Tests

```bash
cargo test
./scripts/test-unit.sh
```

### Integration Tests

```bash
./scripts/test-integration.sh
```

### Privileged Tests

Kill switch tests need root:

```bash
./scripts/run-privileged-tests.sh
```

### Manual Testing Checklist

Before submitting changes to core functionality:

- [ ] Tray icon appears and works
- [ ] `shroud connect <name>` connects
- [ ] `shroud disconnect` disconnects cleanly
- [ ] `shroud ks on` enables kill switch
- [ ] `shroud ks off` removes all rules
- [ ] `shroud quit` exits cleanly
- [ ] Crash recovery cleans up rules
- [ ] Works on Arch, Debian, Fedora

---

## Questions?

- **Bug?** Open an issue with reproduction steps
- **Feature idea?** Open an issue to discuss first
- **Stuck?** Open an issue. You'll get a response.

Be respectful. Be constructive.

---

## A Note on How Shroud Is Built

Shroud is built with AI. The code is reviewed, tested, and maintained by Lou. If you're contributing, your work goes through the same process as everything else -- CI, tests, review. The tools don't change the standard.

---

*Thanks for helping make Shroud better.*
