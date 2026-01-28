//! CLI module.
//!
//! Provides command-line argument parsing and command handlers.

pub mod args;
pub mod handlers;
pub mod help;

pub use args::{Args, DebugAction, ParsedCommand, ToggleAction};
pub use handlers::run_client_mode;
