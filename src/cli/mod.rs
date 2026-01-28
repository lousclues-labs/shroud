//! CLI module.
//!
//! Provides command-line argument parsing and command handlers.

pub mod args;
pub mod handlers;
pub mod help;

pub use args::Args;
pub use handlers::run_client_mode;
