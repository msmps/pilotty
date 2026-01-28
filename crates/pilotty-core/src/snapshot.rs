//! Screen state capture and change detection.
//!
//! This module provides types for capturing terminal screen state, including
//! text content, cursor position, and detected UI elements.
//!
//! # Snapshot Formats
//!
//! The daemon supports two snapshot formats:
//!
//! | Format | Content | Use Case |
//! |--------|---------|----------|
//! | **Full** | text + elements + hash | Complete state for new screens |
//! | **Compact** | metadata only | Quick status checks |
//!
//! # Change Detection
//!
//! The `content_hash` field provides efficient change detection. Agents can
//! compare hashes across snapshots without parsing the full element list:
//!
//! ```ignore
//! if new_snapshot.content_hash != old_snapshot.content_hash {
//!     // Screen changed, re-analyze elements
//! }
//! ```

use serde::{Deserialize, Serialize};

use crate::elements::Element;

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenState {
    pub snapshot_id: u64,
    pub size: TerminalSize,
    pub cursor: CursorState,
    /// Plain text content of the screen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Detected interactive UI elements.
    ///
    /// Elements are detected using visual style segmentation and pattern
    /// classification. Each element includes its position (row, col) for
    /// interaction via the click command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elements: Option<Vec<Element>>,
    /// Hash of screen content for change detection.
    ///
    /// Computed from the screen text using a fast non-cryptographic hash.
    /// Present when `elements` is requested (`with_elements=true`).
    /// Agents can compare hashes across snapshots to detect screen changes
    /// without parsing the full element list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<u64>,
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
            elements: None,
            content_hash: None,
        }
    }
}

/// Compute a content hash from screen text.
///
/// Uses FNV-1a, a fast non-cryptographic hash suitable for change detection.
#[must_use]
pub fn compute_content_hash(text: &str) -> u64 {
    // FNV-1a parameters for 64-bit
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;

    let mut hash = FNV_OFFSET;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_deterministic() {
        let text = "Hello, World!";
        let hash1 = compute_content_hash(text);
        let hash2 = compute_content_hash(text);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn content_hash_differs_for_different_text() {
        let hash1 = compute_content_hash("Hello");
        let hash2 = compute_content_hash("World");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn content_hash_empty_string() {
        // Empty string should return the FNV-1a offset basis
        let hash = compute_content_hash("");
        assert_eq!(hash, 0xcbf29ce484222325);
    }

    #[test]
    fn content_hash_single_char_difference() {
        // Even a single character difference should produce different hashes
        let hash1 = compute_content_hash("test");
        let hash2 = compute_content_hash("tess");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn content_hash_unicode() {
        // Unicode text should hash consistently
        let text = "日本語テスト 🚀";
        let hash1 = compute_content_hash(text);
        let hash2 = compute_content_hash(text);
        assert_eq!(hash1, hash2);
        // Should differ from ASCII
        assert_ne!(hash1, compute_content_hash("ascii"));
    }
}
