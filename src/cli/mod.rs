//! CLI module.
//!
//! Provides command-line argument parsing and command handlers.

pub mod args;
pub mod handlers;
pub mod help;
pub mod import;
pub mod install;
#[allow(dead_code)]
pub mod output;
pub mod validation;

#[allow(unused_imports)]
pub use args::{parse_args, Args, DebugAction, ParsedCommand, ToggleAction};
pub use handlers::run_client_mode;
#[allow(unused_imports)]
pub use validation::{ValidationError, ValidationResult};
