//! UI element detection types.
//!
//! This module provides types for detecting and classifying terminal UI elements.
//! It uses a heuristic pipeline that segments the terminal buffer by visual
//! style, then classifies segments into semantic kinds.
//!
//! # Element Kinds
//!
//! We use a simplified 3-kind model instead of many roles:
//! - **Button**: Clickable elements (bracketed text, inverse video)
//! - **Input**: Text entry fields (cursor position, underscore runs)
//! - **Toggle**: Checkbox/radio elements with on/off state
//!
//! # Detection Rules (priority order)
//!
//! 1. Cursor position → Input (confidence: 1.0, focused: true)
//! 2. Checkbox pattern `[x]`/`[ ]`/`☑`/`☐` → Toggle (confidence: 1.0)
//! 3. Inverse video → Button (confidence: 1.0, focused: true)
//! 4. Bracket pattern `[OK]`/`<Cancel>` → Button (confidence: 0.8)
//! 5. Underscore field `____` → Input (confidence: 0.6)
//!
//! Non-interactive elements (links, progress bars, status text) are filtered out.
//! They remain in `snapshot.text` for agents to read, not as elements.

pub mod classify;
pub mod grid;
pub mod segment;
pub mod style;

use serde::{Deserialize, Serialize};

/// Kind of interactive element.
///
/// Simplified from 11 roles to 3 kinds based on what agents actually need:
/// - What kind is it? (button/input/toggle)
/// - Is it focused?
/// - What's the toggle state? (for toggles only)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    /// Clickable element (buttons, menu items, tabs).
    /// Detected via: inverse video, bracket patterns `[OK]`, `<Cancel>`.
    Button,
    /// Text entry field.
    /// Detected via: cursor position, underscore runs `____`.
    Input,
    /// Checkbox or radio button with on/off state.
    /// Detected via: `[x]`, `[ ]`, `☑`, `☐` patterns.
    Toggle,
}

/// A detected interactive UI element.
///
/// # Coordinates
///
/// All coordinates are 0-based (row, col) to match cursor API.
/// Height is always 1 in v1 (single-row elements only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    /// Kind of interactive element.
    pub kind: ElementKind,

    /// Row index (0-based, from top).
    pub row: u16,

    /// Column index (0-based, from left).
    pub col: u16,

    /// Width in terminal cells.
    pub width: u16,

    /// Text content of the element.
    pub text: String,

    /// Detection confidence (0.0-1.0).
    /// - 1.0: High confidence (cursor, inverse video, checkbox pattern)
    /// - 0.8: Medium confidence (bracket pattern)
    /// - 0.6: Low confidence (underscore run)
    pub confidence: f32,

    /// Whether this element currently has focus.
    /// Orthogonal to kind, applies to any element type.
    #[serde(default, skip_serializing_if = "is_false")]
    pub focused: bool,

    /// Checked state for Toggle kind (None for non-toggles).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
}

/// Helper for serde skip_serializing_if.
fn is_false(b: &bool) -> bool {
    !*b
}

impl Element {
    /// Create a new element.
    #[must_use]
    pub fn new(
        kind: ElementKind,
        row: u16,
        col: u16,
        width: u16,
        text: String,
        confidence: f32,
    ) -> Self {
        Self {
            kind,
            row,
            col,
            width,
            text,
            confidence,
            focused: false,
            checked: None,
        }
    }

    /// Set checked state (for toggles).
    #[must_use]
    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// Set focused state.
    #[must_use]
    pub fn with_focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_kind_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&ElementKind::Button).unwrap(),
            "\"button\""
        );
        assert_eq!(
            serde_json::to_string(&ElementKind::Toggle).unwrap(),
            "\"toggle\""
        );
    }

    #[test]
    fn element_serialization_omits_optional_fields() {
        let elem = Element::new(ElementKind::Button, 0, 0, 4, "OK".to_string(), 0.8);
        let json = serde_json::to_string(&elem).unwrap();

        // Buttons shouldn't have checked, unfocused elements shouldn't have focused
        assert!(!json.contains("checked"));
        assert!(!json.contains("focused"));
    }

    #[test]
    fn element_serialization_includes_set_fields() {
        let elem = Element::new(ElementKind::Toggle, 0, 0, 3, "[x]".to_string(), 1.0)
            .with_checked(true)
            .with_focused(true);
        let json = serde_json::to_string(&elem).unwrap();

        assert!(json.contains("\"checked\":true"));
        assert!(json.contains("\"focused\":true"));
    }
}
