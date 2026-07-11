//! Core types and logic for pilotty.
//!
//! This crate provides shared data structures and algorithms for AI-driven
//! terminal automation. It's used by both the CLI/daemon and MCP server.
//!
//! # Modules
//!
//! - [`error`]: API error types with actionable suggestions for AI consumers
//! - [`input`]: Terminal input encoding (keys, mouse, modifiers)
//! - [`protocol`]: JSON-line request/response protocol
//! - [`snapshot`]: Screen state capture and change detection

pub mod error;
pub mod input;
pub mod protocol;
pub mod snapshot;
