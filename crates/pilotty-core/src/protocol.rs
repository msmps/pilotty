//! Protocol types for CLI-daemon communication.

use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::snapshot::ScreenState;

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
    },
    /// Kill a session.
    Kill { session: Option<String> },
    /// Get a snapshot of the terminal screen.
    Snapshot {
        session: Option<String>,
        format: Option<SnapshotFormat>,
    },
    /// Type text at cursor.
    Type {
        text: String,
        session: Option<String>,
    },
    /// Send a key or key combo.
    Key {
        key: String,
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
    /// Full JSON with all metadata.
    #[default]
    Full,
    /// Compact format with inline refs.
    Compact,
    /// Plain text only.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
