# archtools - OpenVPN System Tray Application

A production-ready system tray application for managing OpenVPN connections with auto-reconnect capabilities, designed for Arch Linux / KDE Plasma (X11).

## Features

- **StatusNotifierItem (SNI) Tray Support**: Native integration with KDE Plasma using `ksni`
- **Dynamic Icon Colors**:
  - 🟢 Green: Connected
  - 🟡 Yellow/Orange: Connecting or Retrying
  - 🔴 Red: Disconnected
- **Auto-Reconnect**: Automatically reconnects when connection drops unexpectedly
- **Intent Tracking**: Distinguishes between user-initiated disconnects and unexpected connection drops
- **Server Switching**: Seamlessly switch between VPN servers
- **Desktop Notifications**: Receive notifications for connection state changes

## Requirements

### Arch Linux

```bash
sudo pacman -S openvpn polkit rust
```

### Ubuntu/Debian (for development)

```bash
sudo apt install openvpn policykit-1 libdbus-1-dev pkg-config
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

4. Run the application:
   ```bash
   ./target/release/openvpn-tray
   ```

## Usage

- **Left-click** on the tray icon to open the menu
- Select a server from the list to connect
- Click **Disconnect** to manually disconnect
- Toggle **Auto-Reconnect** to enable/disable automatic reconnection
- Click **Refresh Servers** to rescan `/etc/openvpn/` for new configurations

## Configuration

The application scans for `.ovpn` files in `/etc/openvpn/` and expects authentication credentials in `/etc/openvpn/auth.txt`.

### Auto-Reconnect Behavior

- **Enabled by default**: The app will automatically attempt to reconnect if the VPN connection drops unexpectedly
- **Retry Logic**: Up to 5 reconnection attempts with 5-second delays between attempts
- **Intent Tracking**: If you manually click "Disconnect", auto-reconnect is disabled until you connect again

## Architecture

The application uses an actor-based architecture to ensure the UI never freezes during reconnection attempts:

- **VpnActor**: Handles process supervision and connection management in an async context
- **VpnTray**: Manages the system tray UI using the `ksni` crate
- **Shared State**: Thread-safe state management using `tokio::sync::RwLock`

## Technical Stack

- **Rust 2021 Edition** (Rust 1.92.0 or newer recommended)
- **ksni**: Native KDE StatusNotifierItem support
- **tokio**: Async runtime with process, fs, time, and signal features
- **notify-rust**: Desktop notifications via D-Bus
- **nix**: Unix signal handling (SIGTERM for graceful process termination)
- **regex**: Filename parsing for clean server name display

## License

MIT
