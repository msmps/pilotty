//! Visual style types for element detection segmentation.
//!
//! These types represent cell styling independent of the vt100 crate,
//! allowing the core element detection types to remain vt100-agnostic.

use serde::{Deserialize, Serialize};

/// Terminal color representation.
///
/// Maps to standard terminal color modes:
/// - Default: terminal's default foreground/background
/// - Indexed: 256-color palette (0-255)
/// - Rgb: 24-bit true color
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Color {
    /// Terminal default color.
    #[default]
    Default,
    /// 256-color palette index (0-255).
    Indexed { index: u8 },
    /// 24-bit RGB color.
    Rgb { r: u8, g: u8, b: u8 },
}

impl Color {
    /// Create an indexed color.
    #[must_use]
    pub fn indexed(index: u8) -> Self {
        Self::Indexed { index }
    }

    /// Create an RGB color.
    #[must_use]
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::Rgb { r, g, b }
    }
}

/// Visual style attributes for a terminal cell.
///
/// Used for segmentation: adjacent cells with identical styles are grouped
/// into clusters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct CellStyle {
    /// Bold text attribute.
    pub bold: bool,
    /// Underlined text attribute.
    pub underline: bool,
    /// Inverse video (swapped fg/bg).
    pub inverse: bool,
    /// Foreground color.
    pub fg_color: Color,
    /// Background color.
    pub bg_color: Color,
}

impl CellStyle {
    /// Create a new cell style with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set bold attribute.
    #[must_use]
    pub fn with_bold(mut self, bold: bool) -> Self {
        self.bold = bold;
        self
    }

    /// Set underline attribute.
    #[must_use]
    pub fn with_underline(mut self, underline: bool) -> Self {
        self.underline = underline;
        self
    }

    /// Set inverse attribute.
    #[must_use]
    pub fn with_inverse(mut self, inverse: bool) -> Self {
        self.inverse = inverse;
        self
    }

    /// Set foreground color.
    #[must_use]
    pub fn with_fg(mut self, color: Color) -> Self {
        self.fg_color = color;
        self
    }

    /// Set background color.
    #[must_use]
    pub fn with_bg(mut self, color: Color) -> Self {
        self.bg_color = color;
        self
    }

    /// Check if this style uses inverse video.
    ///
    /// Inverse video is a strong signal for selected menu items and tabs.
    #[must_use]
    pub fn is_inverse(&self) -> bool {
        self.inverse
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_style_default() {
        let style = CellStyle::default();
        assert!(!style.bold);
        assert!(!style.inverse);
        assert_eq!(style.fg_color, Color::Default);
    }

    #[test]
    fn is_inverse_helper() {
        assert!(!CellStyle::new().is_inverse());
        assert!(CellStyle::new().with_inverse(true).is_inverse());
    }
}
