//! Screen state and region detection types.

use serde::{Deserialize, Serialize};

use crate::error::ApiError;

/// A unique identifier for an interactive region.
///
/// RefIds are formatted as `@e<number>` (e.g., `@e1`, `@e2`).
/// Use `RefId::new()` to create and `as_str()` to access the inner value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RefId(String);

impl RefId {
    /// Create a new RefId.
    ///
    /// The id should be in the format `@e<number>` (e.g., `@e1`, `@e42`).
    /// Empty strings are allowed for unassigned refs before `assign_refs` is called.
    ///
    /// # Panics
    ///
    /// Panics if the format is invalid. All internal callers use `format!("@e{}", n)`
    /// which is always valid, so this catches bugs in calling code immediately.
    /// For untrusted input, use [`RefId::try_new`] instead.
    pub fn new(id: impl Into<String>) -> Self {
        let s = id.into();
        assert!(
            s.is_empty() || Self::is_valid_format(&s),
            "RefId must be empty or in format @e<number>, got: {s}"
        );
        Self(s)
    }

    /// Create a new RefId with validation, returning an error on invalid format.
    pub fn try_new(id: impl Into<String>) -> Result<Self, ApiError> {
        let s = id.into();
        if s.is_empty() || Self::is_valid_format(&s) {
            return Ok(Self(s));
        }

        Err(ApiError::invalid_input_with_suggestion(
            format!("Invalid ref id '{}'", s),
            "Use a ref from snapshot output, formatted as @e<number> (e.g., @e1).",
        ))
    }

    /// Check if a string is a valid RefId format (@e followed by digits).
    fn is_valid_format(s: &str) -> bool {
        if let Some(rest) = s.strip_prefix("@e") {
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
        } else {
            false
        }
    }

    /// Get the inner string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::RefId;

    #[test]
    fn test_ref_id_try_new_accepts_valid() {
        let ref_id = RefId::try_new("@e42").expect("valid ref id");
        assert_eq!(ref_id.as_str(), "@e42");
    }

    #[test]
    fn test_ref_id_try_new_rejects_invalid() {
        let err = RefId::try_new("bad-ref").expect_err("invalid ref id");
        assert!(err.message.contains("Invalid ref id"));
        assert!(err
            .suggestion
            .expect("suggestion should be present")
            .contains("@e"));
    }
}

impl std::fmt::Display for RefId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

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

/// A rectangular region on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// Type of interactive region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionType {
    Button,
    TextInput,
    MenuItem,
    Checkbox,
    RadioButton,
    Link,
    ScrollableArea,
    Unknown,
}

/// An interactive region detected on the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub ref_id: RefId,
    pub bounds: Rect,
    pub region_type: RegionType,
    pub text: String,
    pub focused: bool,
}

/// Complete screen state snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenState {
    pub snapshot_id: u64,
    pub size: TerminalSize,
    pub cursor: CursorState,
    pub regions: Vec<Region>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_region: Option<RefId>,
    /// Plain text content of the screen (for text format).
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
            regions: Vec::new(),
            active_region: None,
            text: None,
        }
    }
}
