//! Protocol types for CLI-daemon communication.

use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::snapshot::ScreenState;

/// Default timeout for snapshot await_change/settle operations (30 seconds).
fn default_snapshot_timeout() -> u64 {
    30000
}

/// A request from CLI to daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub command: Command,
}

/// Commands the daemon can execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    /// Spawn a new PTY session.
    Spawn {
        command: Vec<String>,
        session_name: Option<String>,
        /// Working directory for the spawned process.
        ///
        /// If provided, the command runs in this directory. The path must be
        /// an existing directory. If not provided, the process inherits the
        /// daemon's working directory.
        cwd: Option<String>,
    },
    /// Kill a session.
    Kill { session: Option<String> },
    /// Get a snapshot of the terminal screen.
    ///
    /// Optionally block until the screen changes from a baseline hash and/or
    /// stabilizes for a specified duration.
    Snapshot {
        session: Option<String>,
        format: Option<SnapshotFormat>,
        /// If set, block until content_hash differs from this value.
        #[serde(default)]
        await_change: Option<u64>,
        /// Wait for screen to be stable for this many ms before returning.
        #[serde(default)]
        settle_ms: u64,
        /// Timeout in ms for await_change/settle operations.
        #[serde(default = "default_snapshot_timeout")]
        timeout_ms: u64,
    },
    /// Type text at cursor.
    Type {
        text: String,
        session: Option<String>,
    },
    /// Send a key, key combo, or key sequence.
    ///
    /// For sequences (space-separated keys like "Ctrl+X m"), `delay_ms` specifies
    /// the delay between each key. Defaults to 0 (no delay). Maximum is 10000ms.
    Key {
        key: String,
        /// Delay between keys in a sequence (milliseconds). Defaults to 0, max 10000.
        #[serde(default)]
        delay_ms: u32,
        session: Option<String>,
    },
    /// Click at a specific row/column coordinate.
    Click {
        row: u16,
        col: u16,
        session: Option<String>,
    },
    /// Scroll the terminal.
    Scroll {
        direction: ScrollDirection,
        amount: u32,
        session: Option<String>,
    },
    /// List all active sessions.
    ListSessions,
    /// Resize the terminal.
    Resize {
        cols: u16,
        rows: u16,
        session: Option<String>,
    },
    /// Wait for text to appear.
    WaitFor {
        pattern: String,
        timeout_ms: Option<u64>,
        regex: Option<bool>,
        session: Option<String>,
    },
    /// Shutdown the daemon gracefully.
    Shutdown,
}

/// Snapshot output format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotFormat {
    /// Full JSON with all metadata including text and elements.
    #[default]
    Full,
    /// Compact format: omits text and elements, just metadata.
    Compact,
    /// Plain text only (no JSON structure).
    Text,
}

/// Scroll direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
}

/// A response from daemon to CLI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ResponseData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

impl Response {
    pub fn success(id: impl Into<String>, data: ResponseData) -> Self {
        Self {
            id: id.into(),
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(id: impl Into<String>, error: ApiError) -> Self {
        Self {
            id: id.into(),
            success: false,
            data: None,
            error: Some(error),
        }
    }
}

/// Response payload variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseData {
    /// Full screen state snapshot.
    ScreenState(ScreenState),
    /// Text-format snapshot.
    Snapshot {
        format: SnapshotFormat,
        content: String,
    },
    /// Session created response.
    SessionCreated { session_id: String, message: String },
    /// List of active sessions.
    Sessions { sessions: Vec<SessionInfo> },
    /// Wait-for result with match info.
    WaitForResult {
        found: bool,
        matched_text: Option<String>,
        elapsed_ms: u64,
    },
    /// Generic success message.
    Ok { message: String },
}

/// Information about an active session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: Option<String>,
    pub command: Vec<String>,
    pub created_at: String,
}
