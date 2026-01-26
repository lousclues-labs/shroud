//! CLI command system for Shroud
//!
//! Provides both client and server functionality for CLI control:
//! - Client mode: Send commands to running daemon via Unix socket
//! - Server mode: Listen for CLI commands in the daemon
//!
//! Communication uses a simple JSON-based protocol over Unix domain sockets.

pub mod args;
pub mod client;
pub mod commands;
pub mod error;
pub mod help;
pub mod server;

pub use args::{parse_args, Args, DebugAction, ParsedCommand, ToggleAction};
pub use commands::CliCommand;
pub use server::CliServer;
