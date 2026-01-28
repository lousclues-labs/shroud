//! Inter-process communication module.
//!
//! Provides the protocol, server, and client components for communication
//! between the Shroud daemon and CLI clients over a Unix domain socket.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐     Unix Socket      ┌─────────────┐
//! │  CLI Client │ ◄──────────────────► │   Daemon    │
//! │             │   IpcCommand/Response│ (IpcServer) │
//! └─────────────┘                      └─────────────┘
//! ```
//!
//! # Modules
//!
//! - [`protocol`]: Command and response types, socket path
//! - [`server`]: Unix socket server for the daemon
//! - [`client`]: Connection logic for CLI clients

pub mod client;
pub mod protocol;
pub mod server;

pub use protocol::{IpcCommand, IpcResponse};
pub use server::IpcServer;
