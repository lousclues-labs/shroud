//! Daemon utilities
//!
//! Provides utilities for running the Shroud daemon, including
//! instance locking to prevent multiple daemons from running.

pub mod lock;

pub use lock::{acquire_instance_lock, get_lock_file_path, release_instance_lock};
