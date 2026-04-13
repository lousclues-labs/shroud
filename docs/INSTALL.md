# Installation

Getting VPN Shroud running on your system. No PhD required.

---

## The Easy Way

```bash
git clone https://github.com/lousclues-labs/shroud.git
cd shroud
./setup.sh
```

That's genuinely it. The setup script:
- Detects your distro
- Installs dependencies
- Builds the binary
- Copies it to `~/.local/bin`
- Sets up desktop entries
- Configures shell completions
- Offers to install the sudoers rule for kill switch

---

## Requirements

You need these on your system:

| Requirement | Why |
|-------------|-----|
| **Linux** | We're not a cross-platform Swiss Army knife |
| **NetworkManager** | We wrap it, not replace it |
| **OpenVPN and/or WireGuard plugins** | For your VPN connections |
| **iptables or nftables** | For the kill switch |
| **Rust 1.75+** | For building from source |

That's the whole list.

---

## Dependencies by Distro

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

For WireGuard on older releases:
```bash
sudo apt install network-manager-wireguard
```

### Fedora

```bash
sudo dnf install NetworkManager NetworkManager-openvpn NetworkManager-wireguard \
    openvpn wireguard-tools iptables nftables rust cargo
```

### openSUSE

```bash
sudo zypper install NetworkManager NetworkManager-openvpn NetworkManager-wireguard \
    openvpn wireguard-tools iptables nftables rust cargo
```

---

## Setup Script Options

The setup script does more than just install:

```bash
# Full installation (recommended)
./setup.sh

# Server/headless installation (no GUI stuff)
./setup.sh --headless

# Just install the sudoers rule
./setup.sh --install-sudoers

# Uninstall everything
./setup.sh uninstall

# Uninstall without asking questions
./setup.sh --force uninstall
```

---

## Manual Installation

If you prefer doing it yourself:

```bash
# Clone
git clone https://github.com/lousclues-labs/shroud.git
cd shroud

# Build
cargo build --release

# Install binary
mkdir -p ~/.local/bin
cp target/release/shroud ~/.local/bin/
chmod +x ~/.local/bin/shroud

# Add to PATH if needed
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc

# Verify
shroud --version
```

### Desktop Entry (Optional)

If you want VPN Shroud in your app launcher:

```bash
mkdir -p ~/.local/share/applications
cp autostart/shroud.desktop ~/.local/share/applications/
```

### Autostart on Login (Optional)

```bash
shroud autostart on
```

Or manually:
```bash
mkdir -p ~/.config/autostart
cp autostart/shroud.desktop ~/.config/autostart/
```

---

## Kill Switch Setup

The kill switch needs root to modify iptables. We use a sudoers rule so you don't have to type your password every time.

### Automatic

```bash
./setup.sh --install-sudoers
```

### Manual

**Arch/Fedora/RHEL (wheel group):**
```bash
echo '%wheel ALL=(ALL) NOPASSWD: /usr/sbin/iptables, /usr/sbin/ip6tables, /usr/sbin/iptables-legacy, /usr/sbin/ip6tables-legacy, /usr/sbin/nft, /usr/bin/iptables, /usr/bin/ip6tables, /usr/bin/iptables-legacy, /usr/bin/ip6tables-legacy, /usr/bin/nft' | sudo tee /etc/sudoers.d/shroud
sudo chmod 440 /etc/sudoers.d/shroud
```

**Debian/Ubuntu (sudo group):**
```bash
echo '%sudo ALL=(ALL) NOPASSWD: /usr/sbin/iptables, /usr/sbin/ip6tables, /usr/sbin/iptables-legacy, /usr/sbin/ip6tables-legacy, /usr/sbin/nft, /usr/bin/iptables, /usr/bin/ip6tables, /usr/bin/iptables-legacy, /usr/bin/ip6tables-legacy, /usr/bin/nft' | sudo tee /etc/sudoers.d/shroud
sudo chmod 440 /etc/sudoers.d/shroud
```

### Verify

```bash
shroud doctor
```

If it says the kill switch is ready, you're good.

### Remove

```bash
sudo rm /etc/sudoers.d/shroud
```

---

## Importing VPN Configs

Bring your own configs:

```bash
# Single file
shroud import ~/mullvad-us1.conf

# With custom name
shroud import ~/vpn.ovpn --name "Work VPN"

# Whole directory
shroud import ~/vpn-configs/

# Preview without importing
shroud import ~/configs/ --dry-run
```

We support:
- **WireGuard** -- `.conf` files with `[Interface]` and `[Peer]` sections
- **OpenVPN** -- `.ovpn` files

Or use nmcli directly:
```bash
nmcli connection import type openvpn file /path/to/config.ovpn
nmcli connection import type wireguard file /path/to/config.conf
```

---

## Verify Installation

```bash
# Version
shroud --version

# Diagnostics
shroud doctor

# Start it up
shroud
```

If the tray icon appears and `shroud status` works, you're set.

---

## Uninstalling

```bash
# Full uninstall
./setup.sh uninstall
```

This removes:
- Binary from `~/.local/bin/`
- Desktop entries
- Autostart entries
- Shell completions
- Sudoers rule (if installed)

It asks before removing config and logs.

Force mode skips the questions:
```bash
./setup.sh --force uninstall
```

---

## Troubleshooting

### "Command not found"

Make sure `~/.local/bin` is in your PATH:
```bash
echo $PATH | grep -q "$HOME/.local/bin" || echo 'Add ~/.local/bin to PATH'
```

### Build fails

Make sure you have Rust 1.75 or newer:
```bash
rustc --version
```

Update if needed:
```bash
rustup update
```

### Dependencies missing

The setup script tries to detect your distro and install dependencies. If it fails, install them manually using the commands above.

---

## Next Steps

1. **Import a VPN**: `shroud import ~/your-vpn.conf`
2. **Connect**: `shroud connect your-vpn`
3. **Enable kill switch**: `shroud ks on`
4. **Enable autostart**: `shroud autostart on`

You're protected.
