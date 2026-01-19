# archtools - OpenVPN System Tray Application

A production-ready system tray application for managing OpenVPN connections with auto-reconnect capabilities, designed for Arch Linux / KDE Plasma (X11).

## Features

- **StatusNotifierItem (SNI) Tray Support**: Native integration with KDE Plasma using `ksni`
- **NetworkManager Integration**: Detects and manages VPNs started externally via NetworkManager
- **Dynamic Icon Colors**:
  - 🟢 Green: Connected
  - 🟡 Yellow/Orange: Connecting or Retrying
  - 🔴 Red: Disconnected
- **Auto-Reconnect**: Automatically reconnects when connection drops unexpectedly (only for app-initiated connections)
- **Intent Tracking**: Distinguishes between user-initiated disconnects and unexpected connection drops
- **Server Switching**: Seamlessly switch between VPN servers
- **Desktop Notifications**: Receive notifications for connection state changes

## Requirements

### Arch Linux

```bash
sudo pacman -S openvpn polkit rust networkmanager
```

### Ubuntu/Debian (for development)

```bash
sudo apt install openvpn policykit-1 libdbus-1-dev pkg-config network-manager
```

## Setup

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

4. Install the binary (optional):
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

## Usage

- **Left-click** on the tray icon to open the menu
- Select a server from the list to connect
- Click **Disconnect** to manually disconnect
- Toggle **Auto-Reconnect** to enable/disable automatic reconnection
- Click **Refresh Servers** to rescan `/etc/openvpn/` for new configurations

## NetworkManager Integration

The tray app now integrates with NetworkManager to provide a unified VPN status view:

- **Detects external VPNs**: If a VPN is connected via NetworkManager (e.g., using `nmcli` or KDE's network applet), the tray will show "Connected (NM)"
- **Disconnect NM VPNs**: The "Disconnect" button works for both app-initiated and NM-initiated connections
- **No auto-reconnect for external VPNs**: Auto-reconnect only triggers for connections started by this app
- **State polling**: NetworkManager state is polled every 5 seconds to detect changes

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
- **NM Sync**: Periodic polling of NetworkManager state for external VPN detection

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

## Technical Stack

- **Rust 2021 Edition** (Rust 1.92.0 or newer recommended)
- **ksni**: Native KDE StatusNotifierItem support
- **tokio**: Async runtime with process, fs, time, and signal features
- **notify-rust**: Desktop notifications via D-Bus
- **nix**: Unix signal handling (SIGTERM for graceful process termination)
- **regex**: Filename parsing for clean server name display
- **nmcli**: NetworkManager CLI for VPN state detection

## License

MIT
