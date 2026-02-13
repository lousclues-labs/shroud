# Development Setup

So you want to hack on Shroud. Excellent.

This guide gets you from zero to a working development environment. No fluff.

---

## Prerequisites

You need:
- **Rust 1.75+** — We use some newer features
- **NetworkManager** — With OpenVPN and/or WireGuard plugins
- **iptables or nftables** — For kill switch testing
- **A Linux system** — WSL might work, but we don't test it

### Arch Linux

```bash
sudo pacman -S networkmanager networkmanager-openvpn networkmanager-wireguard \
    openvpn wireguard-tools iptables nftables rust
```

### Debian / Ubuntu

```bash
sudo apt install network-manager network-manager-openvpn network-manager-openvpn-gnome \
    openvpn wireguard-tools iptables nftables rustc cargo
```

### Fedora

```bash
sudo dnf install NetworkManager NetworkManager-openvpn NetworkManager-wireguard \
    openvpn wireguard-tools iptables nftables rust cargo
```

---

## Getting the Code

```bash
git clone https://github.com/loujr/shroud.git
cd shroud
```

---

## Building

```bash
# Debug build (fast compile, slow runtime)
cargo build

# Release build (slow compile, fast runtime)
cargo build --release

# Run directly without installing
cargo run

# Run with arguments
cargo run -- status
cargo run -- --headless
```

---

## Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture

# Run only unit tests (fast)
./scripts/test-unit.sh

# Run integration tests (needs NM)
./scripts/test-integration.sh

# Run everything
./scripts/test-all.sh
```

### Test Categories

| Script | What It Tests | Requirements |
|--------|---------------|--------------|
| `test-unit.sh` | Pure logic, no side effects | None |
| `test-integration.sh` | NM interactions | NetworkManager running |
| `test-e2e.sh` | Full user scenarios | Running daemon |
| `test-all.sh` | Everything | All of the above |

### Privileged Tests

Kill switch tests need root:

```bash
./scripts/run-privileged-tests.sh
```

---

## Code Quality

Before you commit:

```bash
# Format code
cargo fmt

# Lint (no warnings allowed)
cargo clippy -- -D warnings

# Check for security issues
cargo audit
# or
./scripts/audit.sh
```

All of these run in CI. Save yourself the round trip.

---

## The Development Loop

When you're actively hacking:

```bash
# Make changes, then:
cargo build && cargo test && cargo clippy -- -D warnings

# If it passes, you're probably good
```

For testing with a running daemon:

```bash
# Terminal 1: Run the daemon
RUST_LOG=debug cargo run

# Terminal 2: Send commands
cargo run -- status
cargo run -- connect my-vpn
cargo run -- ks toggle
```

### Quick Rebuild and Restart

If you have Shroud installed:

```bash
shroud update
```

This builds, copies the binary, and restarts the daemon in one command.

---

## Project Structure

```
src/
├── main.rs              # Entry point, VpnSupervisor
├── cli/                 # Command-line interface
│   ├── args.rs          # Argument parsing
│   ├── handlers.rs      # Command handlers
│   └── help.rs          # Help text generation
├── config/              # Configuration management
├── daemon/              # Lock file, single instance
├── dbus/                # D-Bus/NetworkManager monitoring
├── headless/            # Headless/server mode
├── health/              # Connection health checks
├── import/              # VPN config import
├── ipc/                 # Inter-process communication
├── killswitch/          # Firewall rules
├── logging.rs           # Structured logging
├── nm/                  # NetworkManager client
├── state/               # State machine
├── supervisor/          # Main event loop
└── tray/                # System tray integration
```

### Key Files

| File | What It Does |
|------|--------------|
| `src/main.rs` | Entry point, mode detection |
| `src/supervisor/event_loop.rs` | The main event loop |
| `src/state/machine.rs` | State transitions |
| `src/killswitch/firewall.rs` | iptables rule generation |
| `src/nm/client.rs` | nmcli wrapper |
| `src/ipc/protocol.rs` | Command/response types |

---

## Debugging

### Verbose Logging

```bash
# Info level
RUST_LOG=info cargo run

# Debug level (recommended for development)
RUST_LOG=debug cargo run

# Trace level (very verbose)
RUST_LOG=trace cargo run

# Target specific modules
RUST_LOG=shroud::killswitch=debug cargo run
```

### Debug Dump

Get the internal state as JSON:

```bash
shroud debug dump
```

### Common Debug Commands

```bash
# Check daemon status
shroud ping

# View active NM connections
nmcli connection show --active

# View iptables rules
sudo iptables -L -n -v

# View shroud's chain specifically
sudo iptables -L SHROUD_KILLSWITCH -n -v
```

---

## Adding a New CLI Command

1. Add the command variant in `src/ipc/protocol.rs`:
   ```rust
   pub enum IpcCommand {
       // ...
       MyNewCommand { arg: String },
   }
   ```

2. Add the handler in `src/cli/handlers.rs`

3. Add argument parsing in `src/cli/args.rs`

4. Add help text in `src/cli/help.rs`

5. Handle the command in `src/supervisor/handlers.rs`

---

## Adding a Config Option

1. Add the field in `src/config/settings.rs`:
   ```rust
   pub struct Config {
       // ...
       pub my_option: bool,
   }
   ```

2. Add a default in `Default` impl

3. Update the config version if needed

4. Document in `docs/CONFIGURATION.md`

---

## The Philosophy

Development should feel like the tool itself: simple, clear, no surprises.

If something is confusing, that's a bug in our documentation or our code. File an issue or send a PR.

Read [PRINCIPLES.md](PRINCIPLES.md) before making significant changes. Every contribution should align with those values.

---

## Getting Help

- **Architecture questions**: Read [ARCHITECTURE.md](ARCHITECTURE.md)
- **Design philosophy**: Read [PRINCIPLES.md](PRINCIPLES.md)
- **Stuck on something**: Open a GitHub issue

We're friendly. We're here to help.
