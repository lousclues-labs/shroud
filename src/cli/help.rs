// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Help text for CLI commands

/// Print main help message
pub fn print_main_help() {
    println!(
        r#"VPN Shroud - VPN connection manager for Linux

USAGE:
    shroud [OPTIONS]              Start the daemon (tray application)
    shroud [OPTIONS] <COMMAND>    Send a command to the running daemon

OPTIONS:
    -h, --help               Show this help message
    -V, --version            Show version
    -v, --verbose            Increase logging verbosity (-v, -vv, -vvv)
    --log-level <LEVEL>      Set log level (error, warn, info, debug, trace)
    --log-file <PATH>        Log to file instead of stderr
    --json                   Output in JSON format
    -q, --quiet              Suppress output (exit code only)
    --timeout <SECS>         Timeout for daemon communication (default: 5)
    -H, --headless           Run in headless server mode (no tray)
    --desktop                Force desktop mode with tray icon

COMMANDS:
    connect <NAME>       Connect to a VPN connection
    disconnect           Disconnect from current VPN
    reconnect            Reconnect to current VPN
    switch <NAME>        Switch to a different VPN (atomic)
    status               Show current status
    list                 List available VPN connections
    import               Import VPN config (WireGuard/OpenVPN)
    killswitch           Manage kill switch (on/off/toggle/status)
    auto-reconnect       Manage auto-reconnect (on/off/toggle/status)
    autostart            Manage autostart on login (on/off/toggle/status)
    cleanup              Remove old configurations and stale files
    debug                Manage debug logging (on/off/log-path/tail/dump)
    ping                 Check if daemon is running
    refresh              Refresh VPN connection list
    quit                 Stop the daemon gracefully
    restart              Restart the daemon
    reload               Reload configuration without restart
    doctor               Diagnose configuration issues
    verify-killswitch    Verify kill switch rules are working correctly
    update               Build, install, and restart
    version              Show version information
    help <COMMAND>       Show help for a command

EXAMPLES:
    shroud                          Start the tray application
    shroud connect ireland-42       Connect to 'ireland-42'
    shroud status                   Show current connection status
    shroud status --json            Show status in JSON format
    shroud killswitch on            Enable the kill switch
    shroud list                     List available VPN connections
    shroud import ~/vpn.conf         Import a VPN config file
    shroud debug tail               Follow the debug log file
    shroud autostart on             Enable autostart on login
    shroud autostart status         Check autostart status
    shroud --headless               Run in headless server mode
    shroud cleanup                  Remove old systemd service and stale files
    shroud reload                   Reload configuration from disk
    shroud doctor                   Diagnose configuration issues
    shroud update                   Build, install, and restart
    shroud version --check          Check if binary is stale

ALIASES:
    ls                   Alias for 'list'
    ks                   Alias for 'killswitch'
    ar                   Alias for 'auto-reconnect'
    startup              Alias for 'autostart'
    stop, exit           Aliases for 'quit'

For more information, visit: https://github.com/lousclues-labs/shroud"#
    );
}

/// Print help for a specific command
pub fn print_command_help(command: &str) {
    match command {
        "connect" => println!(
            r#"Connect to a VPN

USAGE:
    shroud connect <NAME>

ARGS:
    <NAME>    Name of the NetworkManager VPN connection

EXAMPLES:
    shroud connect ireland-42
    shroud connect "My VPN"
    shroud -q connect us-east-1    # Quiet mode, exit code only"#
        ),

        "disconnect" => println!(
            r#"Disconnect from current VPN

USAGE:
    shroud disconnect

Disconnects from the currently active VPN connection.
The kill switch (if enabled) will remain active until manually disabled."#
        ),

        "reconnect" => println!(
            r#"Reconnect to current VPN

USAGE:
    shroud reconnect

Disconnects and reconnects to the same VPN server.
Useful when the connection is degraded or stuck."#
        ),

        "switch" => println!(
            r#"Switch to a different VPN

USAGE:
    shroud switch <NAME>

ARGS:
    <NAME>    Name of the VPN connection to switch to

Performs an atomic disconnect + connect operation.
The kill switch (if enabled) remains active during the switch.

EXAMPLES:
    shroud switch us-west-2"#
        ),

        "status" => println!(
            r#"Show current status

USAGE:
    shroud status [OPTIONS]

OPTIONS:
    --json    Output in JSON format

OUTPUT (human-readable):
    State: Connected
    Connection: ireland-42
    Kill switch: enabled
    Auto-reconnect: enabled

OUTPUT (JSON):
    {{"state":"Connected","connection":"ireland-42","kill_switch":true,"auto_reconnect":true}}"#
        ),

        "list" | "ls" => println!(
            r#"List available VPN connections

USAGE:
    shroud list [OPTIONS]

OPTIONS:
    --type <TYPE>   Filter by type: wireguard, openvpn, or all
    --json          Output in JSON format

ALIASES:
    ls

OUTPUT (human-readable):
    NAME                 TYPE        STATUS
    ireland-42            openvpn     available
    mullvad-us1           wireguard   connected

OUTPUT (JSON):
    {{"connections":[{{"name":"ireland-42","vpn_type":"openvpn","status":"available"}}]}}"#
        ),

        "import" => println!(
            r#"Import VPN config files

USAGE:
    shroud import <PATH> [OPTIONS]

ARGS:
    <PATH>    File or directory path to import

OPTIONS:
    -n, --name <NAME>     Custom connection name (single file only)
    -c, --connect         Connect immediately after successful import
    -f, --force           Overwrite existing connection with same name
    -r, --recursive       Recurse into subdirectories when importing directory
    --dry-run             Show what would be imported without importing
    --type <TYPE>         Force config type: wireguard or openvpn
    -q, --quiet           Suppress output except errors
    --json                Output results as JSON

EXAMPLES:
    shroud import ~/mullvad-us1.conf
    shroud import ~/corporate.ovpn --name "Work VPN"
    shroud import ~/vpn.conf --connect
    shroud import ~/vpn-configs/
    shroud import ~/configs/ --dry-run
    shroud import ~/vpn.conf --force
    shroud import ~/configs/ --json"#
        ),

        "killswitch" | "kill-switch" | "ks" => println!(
            r#"Manage the kill switch

USAGE:
    shroud killswitch <ACTION>

ACTIONS:
    on, enable      Enable the kill switch
    off, disable    Disable the kill switch
    toggle          Toggle the kill switch
    status          Show kill switch status (default)

ALIASES:
    ks, kill-switch

The kill switch blocks all non-VPN traffic when enabled, preventing
leaks if the VPN connection drops. It uses iptables rules.

EXAMPLES:
    shroud killswitch on
    shroud ks toggle
    shroud killswitch status --json"#
        ),

        "auto-reconnect" | "autoreconnect" | "ar" => println!(
            r#"Manage auto-reconnect

USAGE:
    shroud auto-reconnect <ACTION>

ACTIONS:
    on, enable      Enable auto-reconnect
    off, disable    Disable auto-reconnect
    toggle          Toggle auto-reconnect
    status          Show auto-reconnect status (default)

ALIASES:
    ar, autoreconnect

When enabled, VPN Shroud will automatically attempt to reconnect if the
VPN connection drops unexpectedly."#
        ),

        "autostart" | "startup" => println!(
            r#"Manage automatic startup on login

USAGE:
    shroud autostart <on|off|toggle|status>

SUBCOMMANDS:
    on        Enable autostart on login
    off       Disable autostart
    toggle    Toggle autostart state
    status    Show current autostart status

EXAMPLES:
    shroud autostart on
    shroud autostart status

NOTES:
    This uses XDG autostart (~/.config/autostart/shroud.desktop).
    The desktop file contains an absolute path to the shroud binary.

If you previously used systemd user services, run 'shroud cleanup'
to remove the old configuration.

ALIAS:
    'startup' is an alias for 'autostart'"#
        ),

        "cleanup" => println!(
            r#"Clean up old configurations and stale files

USAGE:
    shroud cleanup

This command removes:
    - Old systemd user service (deprecated)
    - Stale socket files (if daemon not running)
    - Stale lock files (if daemon not running)

This is safe to run at any time."#
        ),

        "doctor" => println!(
            r#"Diagnose common configuration issues

USAGE:
    shroud doctor

Checks for:
    - Firewall binary paths (iptables, ip6tables, nft)
    - Sudo access configuration
    - Sudoers file presence
    - User group membership

Run this command if the kill switch is not working."#
        ),

        "verify-killswitch" | "verify-ks" => println!(
            r#"Verify kill switch rules are working

USAGE:
    shroud verify-killswitch [OPTIONS]

OPTIONS:
    --json   Output results as JSON
    -v       Show detailed rule output

ALIASES:
    verify-ks

Runs read-only checks to verify the kill switch is correctly configured
and actively protecting against traffic leaks. No rules are modified.

CHECKS:
1. SHROUD_KILLSWITCH chain exists in iptables/nftables
2. OUTPUT chain has jump rule to SHROUD_KILLSWITCH
3. Default policy is DROP (non-VPN traffic blocked)
4. Loopback traffic is allowed
5. VPN tunnel interfaces (tun+/wg+/tap+) are allowed
6. DHCP traffic is allowed
7. IPv6 leak protection is in place
8. DNS leak protection matches configured mode
9. No conflicting/rogue rules exist in OUTPUT
10. State machine agrees kill switch is active

EXAMPLES:
    shroud verify-killswitch
    shroud verify-ks --json
    shroud verify-ks -v"#
        ),

        "debug" => println!(
            r#"Manage debug logging

USAGE:
    shroud debug <ACTION> [OPTIONS]

ACTIONS:
    on, enable      Enable debug logging to file
    off, disable    Disable debug logging
    log-path        Show the debug log file path
    tail            Follow the log file (INFO+ only, filtered)
    tail -v         Follow the log file (all levels including DEBUG)
    dump            Dump current internal state as JSON

The debug log is written to ~/.local/share/shroud/debug.log
with automatic rotation (10MB max, 3 files kept).

'tail' auto-enables debug logging if not already on.
Default output filters out DEBUG-level noise (NM polling,
health checks). Use -v/--verbose for the full firehose.

EXAMPLES:
    shroud debug on
    shroud debug tail            # INFO, WARN, ERROR only
    shroud debug tail -v         # All levels including DEBUG
    shroud debug dump"#
        ),

        "ping" => println!(
            r#"Check if daemon is running

USAGE:
    shroud ping

Returns exit code 0 if the daemon is running, 2 if not.

OUTPUT:
    VPN Shroud daemon is running (PID: 12345, uptime: 2h 15m)"#
        ),

        "refresh" => println!(
            r#"Refresh VPN connection list

USAGE:
    shroud refresh

Requests the daemon to re-scan NetworkManager for available
VPN connections. Useful after importing new .ovpn files."#
        ),

        "quit" | "stop" | "exit" => println!(
            r#"Stop the daemon

USAGE:
    shroud quit

ALIASES:
    stop, exit

Requests the daemon to shut down gracefully. The kill switch
will be disabled before exit."#
        ),

        "restart" => println!(
            r#"Restart the daemon

USAGE:
    shroud restart

Stops the daemon and starts a new instance.
Equivalent to: shroud quit && shroud"#
        ),

        "reload" => println!(
            r#"Reload configuration without restart

USAGE:
    shroud reload

Reloads config from disk and applies it live without restarting
the daemon."#
        ),

        "version" => println!(
            r#"Show version information

USAGE:
    shroud version [--check]

OPTIONS:
    --check    Check if source files are newer than the installed binary

EXAMPLES:
    shroud version
    shroud version --check"#
        ),

        "update" => println!(
            r#"Build, install, and restart

USAGE:
    shroud update

Runs scripts/update.sh which:
  1. Builds a release binary (cargo install --path .)
  2. Copies to ~/.local/bin/shroud
  3. Restarts the daemon

EXAMPLES:
    shroud update"#
        ),

        _ => println!("No help available for '{}'", command),
    }
}
