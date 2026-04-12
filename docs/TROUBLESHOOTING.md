# Troubleshooting

Something broke. Let's get you back online.

---

## Quick Fixes

### "I have no internet"

If VPNShroud crashed with the kill switch enabled, your traffic is blocked. That's the kill switch doing its job, just a bit too enthusiastically.

```bash
# Try this first
shroud ks off

# If VPNShroud isn't responding
sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH
sudo iptables -F SHROUD_KILLSWITCH
sudo iptables -X SHROUD_KILLSWITCH
```

You're back.

### "The daemon won't start"

Something's holding the lock or socket:

```bash
# Check for stale processes
pgrep -f shroud

# Kill them if needed
pkill -f shroud

# Remove stale socket
rm -f "${XDG_RUNTIME_DIR}/shroud.sock"

# Try again
shroud
```

### "The tray icon is missing"

Your desktop environment needs StatusNotifierItem (SNI) support.

- **KDE Plasma**: Should work out of the box
- **GNOME**: Install [AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/)
- **Others**: Check if SNI/AppIndicator is supported

Verify your environment:
```bash
echo $XDG_RUNTIME_DIR
echo $DBUS_SESSION_BUS_ADDRESS
```

Both should have values.

---

## The Doctor Is In

When in doubt, run diagnostics:

```bash
shroud doctor
```

This checks:
- NetworkManager connectivity
- iptables/nftables availability
- Sudoers configuration
- Config file validity
- Socket and lock state

---

## Debug Mode

When you need to see what's actually happening:

```bash
# Enable debug logging to file
shroud debug on

# Watch the logs live
shroud debug tail

# Find the log file
shroud debug log-path

# Dump internal state
shroud debug dump
```

Or run with verbose output directly:

```bash
RUST_LOG=debug shroud
```

---

## Common Issues

### VPN Won't Connect

**Check if NetworkManager knows about the connection:**
```bash
nmcli connection show | grep vpn
```

**Test the connection directly:**
```bash
nmcli connection up "your-vpn-name"
```

If nmcli fails, the problem is upstream. Check your VPN config or NetworkManager logs:
```bash
journalctl -u NetworkManager -f
```

### Kill Switch Won't Enable

**Run diagnostics:**
```bash
shroud doctor
```

**Check if iptables is available:**
```bash
iptables --version
```

**Check if the sudoers rule is installed:**
```bash
sudo -n iptables -L -n
```

If that asks for a password, install the rule:
```bash
./setup.sh --install-sudoers
```

**Check for missing kernel modules:**
```bash
sudo modprobe ip_tables ip6_tables nf_tables
```

### VPN Keeps Disconnecting

1. Check if auto-reconnect is enabled:
   ```bash
   shroud status --json | grep auto_reconnect
   ```

2. Check health check settings in `~/.config/shroud/config.toml`:
   ```toml
   health_check_interval_secs = 30
   max_reconnect_attempts = 10
   ```

3. The VPN server might be unstable. Try a different server.

### Connection List Is Empty

VPNShroud only sees VPN connections that NetworkManager knows about.

```bash
# Check what NM sees
nmcli connection show | grep vpn

# Import a config
shroud import ~/your-vpn.ovpn

# Refresh the list
shroud refresh
```

### Config File Corrupted

VPNShroud will back up corrupted configs and create a fresh one. But if you need to manually reset:

```bash
# Backup and remove
mv ~/.config/shroud/config.toml ~/.config/shroud/config.toml.bak

# Start fresh
shroud
```

### Tray Menu Clicks Do Nothing

This usually means the IPC connection is broken.

```bash
# Restart the daemon
shroud restart

# If that doesn't work
shroud quit
shroud
```

### "Connection already active" Spam

This happens when VPNShroud's state diverges from NetworkManager. Usually after connecting/disconnecting via nm-applet or GNOME Settings.

Fixed in v1.8.4. If you're seeing this, update:
```bash
git pull
shroud update
```

---

## Nuclear Options

### Full Reset

```bash
# Stop everything
shroud quit
pkill -f shroud

# Remove all state
rm -f "${XDG_RUNTIME_DIR}/shroud.sock"
rm -f ~/.local/state/shroud/shroud.lock

# Remove config (optional -- you'll lose settings)
rm -f ~/.config/shroud/config.toml

# Clean any stale firewall rules
sudo iptables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
sudo iptables -F SHROUD_KILLSWITCH 2>/dev/null || true
sudo iptables -X SHROUD_KILLSWITCH 2>/dev/null || true
sudo ip6tables -D OUTPUT -j SHROUD_KILLSWITCH 2>/dev/null || true
sudo ip6tables -F SHROUD_KILLSWITCH 2>/dev/null || true
sudo ip6tables -X SHROUD_KILLSWITCH 2>/dev/null || true

# Start fresh
shroud
```

### Check Everything

```bash
# Version
shroud --version

# Daemon status
shroud ping

# Connection status
shroud status

# Kill switch status
shroud ks status

# VPN list
shroud list

# Diagnostics
shroud doctor

# Debug dump
shroud debug dump
```

---

## Getting Help

If you're stuck:

1. **Run diagnostics**: `shroud doctor`
2. **Check logs**: `shroud debug on && shroud debug tail`
3. **Search issues**: [GitHub Issues](https://github.com/loujr/shroud/issues)
4. **Open a new issue** with:
   - `shroud --version` output
   - `shroud doctor` output
   - Relevant log snippets

Every issue gets read. If it's fixable, it gets fixed.

---

## The Philosophy

Troubleshooting docs should get you back online, not teach you the entire codebase.

If you're reading this, something went wrong. Let's fix it and move on.
