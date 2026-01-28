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
//! - [`elements`]: UI element detection
//!
//! # Element Detection
//!
//! pilotty detects interactive UI elements using a simplified 3-kind model
//! optimized for AI agents:
//!
//! | Kind | Detection | Confidence |
//! |------|-----------|------------|
//! | **Button** | Inverse video, `[OK]`, `<Cancel>` | 1.0 / 0.8 |
//! | **Input** | Cursor position, `____` underscores | 1.0 / 0.6 |
//! | **Toggle** | `[x]`, `[ ]`, `☑`, `☐` | 1.0 |
//!
//! Elements include row/col coordinates for use with the click command.
//! The `content_hash` field enables efficient change detection.

pub mod elements;
pub mod error;
pub mod input;
pub mod protocol;
pub mod snapshot;
