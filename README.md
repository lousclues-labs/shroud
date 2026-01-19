# archtools - OpenVPN System Tray Application

A production-ready system tray application for managing OpenVPN connections with auto-reconnect capabilities, designed for Arch Linux / KDE Plasma (X11).

## Features

- **StatusNotifierItem (SNI) Tray Support**: Native integration with KDE Plasma using `ksni`
- **Left-click Menu**: Opens menu on left-click (native KDE behavior)
- **NetworkManager Integration**: Detects and manages VPNs started externally via NetworkManager
- **Dynamic Icon Colors**:
  - 🟢 Green: Connected
  - 🟡 Yellow/Orange: Connecting or Retrying
  - 🔴 Red: Disconnected
- **Auto-Reconnect**: Automatically reconnects when connection drops unexpectedly (only for app-initiated connections)
- **Intent Tracking**: Distinguishes between user-initiated disconnects and unexpected connection drops
- **Server Switching**: Seamlessly switch between VPN servers
- **Desktop Notifications**: Receive notifications for connection state changes
- **Structured Logging**: Full logging support with `RUST_LOG` environment variable

## Requirements

### Arch Linux

```bash
sudo pacman -S openvpn polkit rust networkmanager
```

### Ubuntu/Debian (for development)

```bash
sudo apt install openvpn policykit-1 libdbus-1-dev pkg-config network-manager
```

## Quick Start with setup.sh (Recommended)

The easiest way to install and manage the application on Arch Linux is using the provided setup script:

```bash
# Clone the repository
git clone https://github.com/loujr/archtools.git
cd archtools

# Run the setup script
./setup.sh
```

The setup script is **idempotent** and will:

1. Verify all dependencies (rust/cargo, openvpn, nmcli, polkit)
2. Pull latest changes from git (use `--skip-pull` to skip)
3. Build the release binary
4. Install binary to `~/.local/bin/openvpn-tray`
5. Install systemd user service to `~/.config/systemd/user/openvpn-tray.service`
6. Reload systemd and enable/restart the service

### Updating After Git Pull

Simply run the setup script again to rebuild and restart the service:

```bash
git pull
./setup.sh
```

Or to skip the git pull (if you've already pulled):

```bash
./setup.sh --skip-pull
```

## Manual Setup

If you prefer manual installation:

1. Place your `.ovpn` configuration files in `/etc/openvpn/`

2. Create an authentication file:
   ```bash
   sudo nano /etc/openvpn/auth.txt
   # Add username on first line
   # Add password on second line
   sudo chmod 600 /etc/openvpn/auth.txt
   ```

3. Build the application:
   ```bash
   cargo build --release
   ```

4. Install the binary:
   ```bash
   mkdir -p ~/.local/bin
   cp target/release/openvpn-tray ~/.local/bin/
   ```

5. Run the application:
   ```bash
   ./target/release/openvpn-tray
   # Or if installed:
   ~/.local/bin/openvpn-tray
   ```

## Systemd User Service (Auto-Start)

To have the tray app start automatically with your graphical session:

1. Install the service file:
   ```bash
   mkdir -p ~/.config/systemd/user
   cp systemd/openvpn-tray.service ~/.config/systemd/user/
   ```

2. Reload systemd and enable the service:
   ```bash
   systemctl --user daemon-reload
   systemctl --user enable --now openvpn-tray.service
   ```

3. Check service status:
   ```bash
   systemctl --user status openvpn-tray.service
   ```

4. After rebuilding the application, restart the service:
   ```bash
   systemctl --user restart openvpn-tray.service
   ```

5. View logs:
   ```bash
   journalctl --user -u openvpn-tray.service -f
   ```

### Service File Location

- Binary: `~/.local/bin/openvpn-tray`
- Service file: `~/.config/systemd/user/openvpn-tray.service`

### Logging

The service defaults to `RUST_LOG=info`. For debug logging:

```bash
# Stop the service first
systemctl --user stop openvpn-tray.service

# Run with debug logging
RUST_LOG=debug ~/.local/bin/openvpn-tray
```

Or edit the service file to change the `Environment=RUST_LOG=info` line.

## Usage

- **Left-click** on the tray icon to open the menu
- **Right-click** also opens the menu (standard behavior)
- Select a server from the list to connect
- Click **Disconnect** to manually disconnect
- Toggle **Auto-Reconnect** to enable/disable automatic reconnection
- Click **Refresh Servers** to rescan `/etc/openvpn/` for new configurations

## NetworkManager Integration

The tray app integrates with NetworkManager to provide a unified VPN status view:

- **Detects external VPNs**: If a VPN is connected via NetworkManager (e.g., using `nmcli` or KDE's network applet), the tray will show "Connected (NM)"
- **Disconnect NM VPNs**: The "Disconnect" button works for both app-initiated and NM-initiated connections
- **No auto-reconnect for external VPNs**: Auto-reconnect only triggers for connections started by this app
- **State polling**: NetworkManager state is polled every 5 seconds to detect changes (with timeout to prevent freezes)

### How it works

The app uses `nmcli -t -f NAME,TYPE,STATE con show --active` to query active VPN connections. When a VPN is detected:
- If the app shows "Disconnected" but NM shows a VPN active, the tray updates to show "Connected (NM)"
- If the app shows "Connected (NM)" but NM shows no VPN, the tray updates to "Disconnected"

## Configuration

The application scans for `.ovpn` files in `/etc/openvpn/` and expects authentication credentials in `/etc/openvpn/auth.txt`.

### Auto-Reconnect Behavior

- **Enabled by default**: The app will automatically attempt to reconnect if the VPN connection drops unexpectedly
- **App-initiated only**: Auto-reconnect only applies to connections started by this app, not external NM connections
- **Retry Logic**: Up to 5 reconnection attempts with 5-second delays between attempts
- **Intent Tracking**: If you manually click "Disconnect", auto-reconnect is disabled until you connect again

## Architecture

The application uses an actor-based architecture to ensure the UI never freezes during reconnection attempts:

- **VpnActor**: Handles process supervision and connection management in an async context
- **VpnTray**: Manages the system tray UI using the `ksni` crate
- **Shared State**: Thread-safe state management using `tokio::sync::RwLock`
- **NM Sync**: Periodic polling of NetworkManager state for external VPN detection (with 10s timeout)

### State Reconciliation

The app maintains internal state that is reconciled with NetworkManager:

| Internal State | NM State | Action |
|---------------|----------|--------|
| Disconnected | VPN Active | Update to Connected (External) |
| Connected (External) | No VPN | Update to Disconnected |
| Connected (App) | VPN Active | No change (trust internal state) |
| Connecting/Retrying | Any | No change (transitional state) |

## Manual Testing

### Test Cases

1. **VPN already active before app starts**
   - Start VPN via `nmcli con up <vpn-name>`
   - Start the tray app
   - Expected: Tray shows "Connected (NM)" with green icon

2. **App starts VPN and NM reflects active**
   - Start tray app with no VPN active
   - Select a server from the menu
   - Expected: Tray shows "Connecting" then "Connected" with green icon

3. **VPN drops unexpectedly (auto-reconnect triggers)**
   - Connect via the app
   - Kill the VPN process: `sudo pkill openvpn`
   - Expected: Tray shows "Retrying" and attempts reconnection

4. **User manually disconnects (no auto-reconnect)**
   - Connect via the app
   - Click "Disconnect" in the menu
   - Expected: Tray shows "Disconnected", no reconnection attempts

5. **Left-click opens menu**
   - Left-click on the tray icon
   - Expected: Menu opens (same as right-click)

6. **NM state change detection**
   - Connect via NM: `nmcli con up <vpn>`
   - Expected: Tray updates to "Connected (NM)" within 5 seconds

## Technical Stack

- **Rust 2021 Edition** (Rust 1.92.0 or newer recommended)
- **ksni 0.3**: Native KDE StatusNotifierItem support with left-click menu
- **tokio**: Async runtime with process, fs, time, and signal features
- **notify-rust**: Desktop notifications via D-Bus
- **nix**: Unix signal handling (SIGTERM for graceful process termination)
- **regex**: Filename parsing for clean server name display
- **nmcli**: NetworkManager CLI for VPN state detection
- **env_logger**: Structured logging with RUST_LOG support

## License

MIT
