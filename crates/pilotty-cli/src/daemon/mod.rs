//! Daemon process for managing PTY sessions.

pub mod client;
pub mod paths;
pub mod pty;
pub mod server;
pub mod session;
pub mod terminal;

// Public API - used by main.rs
pub use client::DaemonClient;
pub use server::DaemonServer;
