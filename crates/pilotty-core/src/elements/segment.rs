//! Segmentation: grouping adjacent cells by visual style.
//!
//! Scans the terminal grid row by row, grouping adjacent cells with identical
//! visual styles into clusters for classification.

use unicode_width::UnicodeWidthStr;

use crate::elements::grid::ScreenGrid;
use crate::elements::style::CellStyle;

/// A cluster of adjacent cells with identical visual style.
///
/// Clusters are the intermediate representation between raw cells and
/// classified elements. Each cluster spans a contiguous horizontal region
/// of a single row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cluster {
    /// Row index (0-based, from top).
    pub row: u16,
    /// Column index (0-based, from left).
    pub col: u16,
    /// Width in terminal cells.
    pub width: u16,
    /// Text content of the cluster.
    pub text: String,
    /// Visual style shared by all cells in this cluster.
    pub style: CellStyle,
}

impl Cluster {
    /// Create a new cluster.
    #[must_use]
    pub fn new(row: u16, col: u16, text: String, style: CellStyle) -> Self {
        // Use unicode-width for proper terminal column alignment.
        // CJK characters are width 2, zero-width chars are width 0.
        let width = text.width().min(u16::MAX as usize) as u16;
        Self {
            row,
            col,
            width,
            text,
            style,
        }
    }

    /// Check if this cluster contains only whitespace.
    #[must_use]
    pub fn is_whitespace_only(&self) -> bool {
        self.text.chars().all(|c| c.is_whitespace())
    }
}

/// Segment a single row into clusters.
fn segment_row<G: ScreenGrid>(grid: &G, row: u16) -> Vec<Cluster> {
    let mut clusters = Vec::new();

    if row >= grid.rows() {
        return clusters;
    }

    let mut current_text = String::new();
    let mut current_style: Option<CellStyle> = None;
    let mut start_col: u16 = 0;

    for col in 0..grid.cols() {
        let Some(cell) = grid.cell(row, col) else {
            continue;
        };

        match current_style {
            Some(ref style) if *style == cell.style => {
                // Same style, extend current cluster
                current_text.push(cell.ch);
            }
            _ => {
                // Style changed or first cell, finalize previous cluster
                if let Some(style) = current_style.take() {
                    if !current_text.is_empty() {
                        clusters.push(Cluster::new(
                            row,
                            start_col,
                            std::mem::take(&mut current_text),
                            style,
                        ));
                    }
                }
                // Start new cluster
                start_col = col;
                current_style = Some(cell.style);
                current_text.push(cell.ch);
            }
        }
    }

    // Don't forget the last cluster
    if let Some(style) = current_style {
        if !current_text.is_empty() {
            clusters.push(Cluster::new(row, start_col, current_text, style));
        }
    }

    clusters
}

/// Segment an entire grid into clusters.
fn segment_grid<G: ScreenGrid>(grid: &G) -> Vec<Cluster> {
    let mut clusters = Vec::new();

    for row in 0..grid.rows() {
        clusters.extend(segment_row(grid, row));
    }

    clusters
}

/// Filter out whitespace-only clusters.
fn filter_whitespace(clusters: Vec<Cluster>) -> Vec<Cluster> {
    clusters
        .into_iter()
        .filter(|c| !c.is_whitespace_only())
        .collect()
}

/// Segment a grid and filter whitespace in one step.
///
/// Convenience function that combines `segment_grid` and `filter_whitespace`.
#[must_use]
pub fn segment<G: ScreenGrid>(grid: &G) -> Vec<Cluster> {
    filter_whitespace(segment_grid(grid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elements::grid::test_support::SimpleGrid;

    #[test]
    fn cluster_creation() {
        let cluster = Cluster::new(5, 10, "Hello".to_string(), CellStyle::default());
        assert_eq!(cluster.row, 5);
        assert_eq!(cluster.col, 10);
        assert_eq!(cluster.width, 5);
        assert_eq!(cluster.text, "Hello");
        assert!(!cluster.is_whitespace_only());
    }

    #[test]
    fn segment_splits_by_style() {
        let mut grid = SimpleGrid::from_text(&["AABBBCC"], 7);
        let bold = CellStyle::new().with_bold(true);
        let inverse = CellStyle::new().with_inverse(true);

        grid.style_range(0, 2, 5, bold);
        grid.style_range(0, 5, 7, inverse);

        let clusters = segment_row(&grid, 0);

        assert_eq!(clusters.len(), 3);
        assert_eq!(clusters[0].text, "AA");
        assert_eq!(clusters[0].col, 0);
        assert_eq!(clusters[1].text, "BBB");
        assert!(clusters[1].style.bold);
        assert_eq!(clusters[2].text, "CC");
        assert!(clusters[2].style.inverse);
    }

    #[test]
    fn segment_filters_whitespace() {
        let mut grid = SimpleGrid::from_text(&["[OK]     [Cancel]"], 20);
        let inverse = CellStyle::new().with_inverse(true);

        grid.style_range(0, 0, 4, inverse);
        grid.style_range(0, 9, 17, inverse);

        let clusters = segment(&grid);

        assert!(clusters.iter().all(|c| !c.is_whitespace_only()));
        let texts: Vec<&str> = clusters.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"[OK]"));
        assert!(texts.contains(&"[Cancel]"));
    }

    // ========================================================================
    // Unicode Width Tests
    // ========================================================================

    #[test]
    fn cluster_width_cjk() {
        // CJK characters should have width 2 each
        let cluster = Cluster::new(0, 0, "你好".to_string(), CellStyle::default());
        assert_eq!(cluster.width, 4); // 2 + 2 = 4
    }

    #[test]
    fn cluster_width_ascii() {
        // ASCII characters should have width 1 each
        let cluster = Cluster::new(0, 0, "Hello".to_string(), CellStyle::default());
        assert_eq!(cluster.width, 5);
    }

    #[test]
    fn cluster_width_mixed() {
        // Mixed ASCII and CJK
        let cluster = Cluster::new(0, 0, "Hi你好".to_string(), CellStyle::default());
        // H=1 + i=1 + 你=2 + 好=2 = 6
        assert_eq!(cluster.width, 6);
    }
}
