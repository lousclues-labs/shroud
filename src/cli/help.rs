//! Help text for CLI commands

/// Print main help message
pub fn print_main_help() {
    println!(
        r#"Shroud - VPN connection manager for Linux

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

COMMANDS:
    connect <NAME>       Connect to a VPN connection
    disconnect           Disconnect from current VPN
    reconnect            Reconnect to current VPN
    switch <NAME>        Switch to a different VPN (atomic)
    status               Show current status
    list                 List available VPN connections
    killswitch           Manage kill switch (on/off/toggle/status)
    auto-reconnect       Manage auto-reconnect (on/off/toggle/status)
    debug                Manage debug logging (on/off/log-path/tail/dump)
    ping                 Check if daemon is running
    refresh              Refresh VPN connection list
    quit                 Stop the daemon gracefully
    restart              Restart the daemon
    help <COMMAND>       Show help for a command

EXAMPLES:
    shroud                          Start the tray application
    shroud connect ireland-42       Connect to 'ireland-42'
    shroud status                   Show current connection status
    shroud status --json            Show status in JSON format
    shroud killswitch on            Enable the kill switch
    shroud list                     List available VPN connections
    shroud debug tail               Follow the debug log file

ALIASES:
    ls                   Alias for 'list'
    ks                   Alias for 'killswitch'
    ar                   Alias for 'auto-reconnect'
    stop, exit           Aliases for 'quit'

For more information, visit: https://github.com/loujr/shroud"#
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
    --json    Output in JSON format

ALIASES:
    ls

OUTPUT (human-readable):
    Available VPN connections:
        ireland-42
        ireland-15
      * us-east-1 (current)

OUTPUT (JSON):
    {{"connections":["ireland-42","ireland-15","us-east-1"],"current":"us-east-1"}}"#
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
leaks if the VPN connection drops. It uses nftables rules.

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

When enabled, Shroud will automatically attempt to reconnect if the
VPN connection drops unexpectedly."#
        ),

        "debug" => println!(
            r#"Manage debug logging

USAGE:
    shroud debug <ACTION>

ACTIONS:
    on, enable      Enable debug logging to file
    off, disable    Disable debug logging
    log-path        Show the debug log file path
    tail            Follow the debug log file (like tail -f)
    dump            Dump current internal state as JSON

The debug log is written to ~/.local/share/shroud/debug.log
with automatic rotation (10MB max, 3 files kept).

EXAMPLES:
    shroud debug on
    shroud debug tail
    shroud debug dump --json"#
        ),

        "ping" => println!(
            r#"Check if daemon is running

USAGE:
    shroud ping

Returns exit code 0 if the daemon is running, 2 if not.

OUTPUT:
    Shroud daemon is running (PID: 12345, uptime: 2h 15m)"#
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

        _ => println!("No help available for '{}'", command),
    }
}
