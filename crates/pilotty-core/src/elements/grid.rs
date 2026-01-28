//! Screen grid abstraction for element detection segmentation.
//!
//! Defines the `ScreenGrid` trait for uniform access to terminal screen content.

use crate::elements::style::CellStyle;

/// A single terminal cell with its character and visual style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCell {
    /// The character in this cell (space for empty cells).
    pub ch: char,
    /// Visual style attributes.
    pub style: CellStyle,
}

impl ScreenCell {
    /// Create a new screen cell.
    #[must_use]
    pub fn new(ch: char, style: CellStyle) -> Self {
        Self { ch, style }
    }
}

/// Trait for accessing terminal screen content.
///
/// This abstraction allows element detection to work with any terminal backend.
/// Uses 0-based coordinates matching the cursor API convention.
pub trait ScreenGrid {
    /// Number of rows in the grid.
    fn rows(&self) -> u16;

    /// Number of columns in the grid.
    fn cols(&self) -> u16;

    /// Get cell at the given position. Returns `None` if out of bounds.
    fn cell(&self, row: u16, col: u16) -> Option<ScreenCell>;
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A simple in-memory grid for testing.
    #[derive(Debug, Clone)]
    pub struct SimpleGrid {
        cells: Vec<ScreenCell>,
        rows: u16,
        cols: u16,
    }

    impl SimpleGrid {
        /// Create a new grid filled with empty cells.
        #[must_use]
        pub fn new(rows: u16, cols: u16) -> Self {
            let cell_count = rows as usize * cols as usize;
            Self {
                cells: vec![ScreenCell::new(' ', CellStyle::default()); cell_count],
                rows,
                cols,
            }
        }

        /// Create a grid from text lines.
        #[must_use]
        pub fn from_text(lines: &[&str], cols: u16) -> Self {
            let rows = lines.len() as u16;
            let mut grid = Self::new(rows, cols);

            for (row_idx, line) in lines.iter().enumerate() {
                for (col_idx, ch) in line.chars().enumerate() {
                    if col_idx < cols as usize {
                        if let Some(idx) = grid.index(row_idx as u16, col_idx as u16) {
                            grid.cells[idx] = ScreenCell::new(ch, CellStyle::default());
                        }
                    }
                }
            }

            grid
        }

        /// Apply a style to a range of cells in a row.
        pub fn style_range(&mut self, row: u16, start_col: u16, end_col: u16, style: CellStyle) {
            for col in start_col..end_col {
                if let Some(idx) = self.index(row, col) {
                    self.cells[idx].style = style;
                }
            }
        }

        fn index(&self, row: u16, col: u16) -> Option<usize> {
            if row < self.rows && col < self.cols {
                Some(row as usize * self.cols as usize + col as usize)
            } else {
                None
            }
        }
    }

    impl ScreenGrid for SimpleGrid {
        fn rows(&self) -> u16 {
            self.rows
        }

        fn cols(&self) -> u16 {
            self.cols
        }

        fn cell(&self, row: u16, col: u16) -> Option<ScreenCell> {
            self.index(row, col).map(|i| self.cells[i].clone())
        }
    }
}

// Re-export for tests in other modules
#[cfg(test)]
pub(crate) use test_support::SimpleGrid;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_cell_creation() {
        let cell = ScreenCell::new('A', CellStyle::default());
        assert_eq!(cell.ch, 'A');
    }

    #[test]
    fn simple_grid_from_text() {
        let grid = SimpleGrid::from_text(&["Hello", "World"], 10);
        assert_eq!(grid.rows(), 2);
        assert_eq!(grid.cols(), 10);
        assert_eq!(grid.cell(0, 0).unwrap().ch, 'H');
        assert_eq!(grid.cell(1, 0).unwrap().ch, 'W');
    }

    #[test]
    fn simple_grid_style_range() {
        let mut grid = SimpleGrid::from_text(&["[OK]"], 10);
        let inverse = CellStyle::new().with_inverse(true);

        grid.style_range(0, 0, 4, inverse);

        assert!(grid.cell(0, 0).unwrap().style.inverse);
        assert!(grid.cell(0, 3).unwrap().style.inverse);
        assert!(!grid.cell(0, 4).unwrap().style.inverse);
    }
}
