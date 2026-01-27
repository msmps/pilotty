//! Screen state types.

use serde::{Deserialize, Serialize};

/// Terminal dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

/// Cursor position and visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorState {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
}

/// Complete screen state snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenState {
    pub snapshot_id: u64,
    pub size: TerminalSize,
    pub cursor: CursorState,
    /// Plain text content of the screen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl ScreenState {
    pub fn empty(cols: u16, rows: u16) -> Self {
        Self {
            snapshot_id: 0,
            size: TerminalSize { cols, rows },
            cursor: CursorState {
                row: 0,
                col: 0,
                visible: true,
            },
            text: None,
        }
    }
}
