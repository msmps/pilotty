//! Terminal emulator using vt100 for ANSI parsing.
//!
//! Wraps vt100::Parser to provide an in-memory terminal screen buffer
//! that can parse ANSI escape sequences from PTY output.

use crate::daemon::pty::TermSize;

/// Terminal emulator that parses ANSI escape sequences.
///
/// Wraps vt100::Parser to maintain an in-memory representation
/// of the terminal screen state.
pub struct TerminalEmulator {
    parser: vt100::Parser,
}

impl TerminalEmulator {
    /// Create a new terminal emulator with the given size.
    pub fn new(size: TermSize) -> Self {
        // vt100::Parser::new(rows, cols, scrollback_len)
        let parser = vt100::Parser::new(size.rows, size.cols, 0);
        Self { parser }
    }

    /// Feed bytes from PTY output into the terminal emulator.
    ///
    /// Parses ANSI escape sequences and updates the screen state.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Get the terminal size (cols, rows).
    ///
    /// Used in tests to verify resize operations.
    #[allow(dead_code)]
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.screen().size();
        (cols, rows)
    }

    /// Get the cursor position (row, col) - 0-indexed.
    pub fn cursor_position(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    /// Check if the cursor is visible.
    ///
    /// Returns false when cursor has been hidden via DECTCEM (ESC[?25l).
    pub fn cursor_visible(&self) -> bool {
        !self.parser.screen().hide_cursor()
    }

    /// Get a cell at the given position.
    ///
    /// Returns None if position is out of bounds.
    ///
    /// Used in tests to verify ANSI attribute parsing (colors, bold, underline).
    #[allow(dead_code)]
    pub fn cell(&self, row: u16, col: u16) -> Option<&vt100::Cell> {
        self.parser.screen().cell(row, col)
    }

    /// Get the plain text content of the screen.
    pub fn get_text(&self) -> String {
        self.parser.screen().contents()
    }

    /// Get the text content of a single row.
    ///
    /// Returns `None` if the row is out of bounds.
    ///
    /// Used in tests to verify line content after cursor movement and newlines.
    #[allow(dead_code)]
    pub fn get_line(&self, row: u16) -> Option<String> {
        let (_, cols) = self.parser.screen().size();
        // rows(start_col, width) returns iterator over all rows with column subset
        // We want full row (start_col=0, width=cols), then skip to the row we want
        self.parser.screen().rows(0, cols).nth(row as usize)
    }

    /// Resize the terminal.
    pub fn resize(&mut self, size: TermSize) {
        self.parser.screen_mut().set_size(size.rows, size.cols);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feed_plain_text() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        term.feed(b"Hello World");

        let text = term.get_text();
        assert!(
            text.contains("Hello World"),
            "Expected 'Hello World' in: {text}"
        );

        // Cursor should be at position after "Hello World"
        let (row, col) = term.cursor_position();
        assert_eq!(row, 0);
        assert_eq!(col, 11);
    }

    #[test]
    fn test_feed_ansi_colors() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // Feed: "Hello" then red color then "Red" then reset
        term.feed(b"Hello\x1b[31mRed\x1b[0m");

        // Plain text should have both words
        let text = term.get_text();
        assert!(text.contains("Hello"), "Expected 'Hello' in: {text}");
        assert!(text.contains("Red"), "Expected 'Red' in: {text}");

        // Check the "R" in "Red" has red foreground (color index 1)
        let cell = term.cell(0, 5).expect("Cell should exist at (0, 5)");
        assert_eq!(cell.contents(), "R");
        // Red is typically color index 1
        assert_eq!(cell.fgcolor(), vt100::Color::Idx(1));

        // Check "H" in "Hello" has default foreground
        let cell = term.cell(0, 0).expect("Cell should exist at (0, 0)");
        assert_eq!(cell.contents(), "H");
        assert_eq!(cell.fgcolor(), vt100::Color::Default);
    }

    #[test]
    fn test_cursor_movement() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // Move cursor to row 5, col 10 (1-indexed in ANSI, 0-indexed in our API)
        // \x1b[6;11H means move to row 6, col 11 (1-indexed)
        term.feed(b"\x1b[6;11H");

        let (row, col) = term.cursor_position();
        assert_eq!(row, 5, "Expected row 5 (0-indexed)");
        assert_eq!(col, 10, "Expected col 10 (0-indexed)");
    }

    #[test]
    fn test_newline_moves_cursor() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // Use \r\n for proper newline (carriage return + line feed)
        term.feed(b"Line 1\r\nLine 2");

        let (row, col) = term.cursor_position();
        assert_eq!(row, 1, "After newline, should be on row 1");
        assert_eq!(col, 6, "After 'Line 2', col should be 6");

        // Check both lines are present
        let line0 = term.get_line(0).expect("row 0 should exist");
        let line1 = term.get_line(1).expect("row 1 should exist");
        assert!(
            line0.contains("Line 1"),
            "Row 0 should have 'Line 1': {line0}"
        );
        assert!(
            line1.contains("Line 2"),
            "Row 1 should have 'Line 2': {line1}"
        );
    }

    #[test]
    fn test_get_line() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        term.feed(b"First line\x1b[2;1HSecond line");

        let line0 = term.get_line(0).expect("row 0 should exist");
        let line1 = term.get_line(1).expect("row 1 should exist");

        assert!(line0.starts_with("First line"), "Line 0: {line0}");
        assert!(line1.starts_with("Second line"), "Line 1: {line1}");

        // Out of bounds returns None
        assert!(term.get_line(100).is_none(), "row 100 should not exist");
    }

    #[test]
    fn test_resize() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        assert_eq!(term.size(), (80, 24));

        term.resize(TermSize {
            cols: 120,
            rows: 40,
        });

        assert_eq!(term.size(), (120, 40));
    }

    #[test]
    fn test_bold_attribute() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // \x1b[1m = bold on, \x1b[0m = reset
        term.feed(b"normal\x1b[1mBOLD\x1b[0m");

        // "n" at col 0 should NOT be bold
        let normal_cell = term.cell(0, 0).expect("Cell should exist");
        assert_eq!(normal_cell.contents(), "n");
        assert!(!normal_cell.bold(), "Regular text should not be bold");

        // "B" at col 6 should be bold
        let bold_cell = term.cell(0, 6).expect("Cell should exist");
        assert_eq!(bold_cell.contents(), "B");
        assert!(bold_cell.bold(), "Text after \\x1b[1m should be bold");
    }

    #[test]
    fn test_underline_attribute() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // \x1b[4m = underline on, \x1b[0m = reset
        term.feed(b"normal\x1b[4mUNDER\x1b[0m");

        // "n" at col 0 should NOT be underlined
        let normal_cell = term.cell(0, 0).expect("Cell should exist");
        assert!(
            !normal_cell.underline(),
            "Regular text should not be underlined"
        );

        // "U" at col 6 should be underlined
        let under_cell = term.cell(0, 6).expect("Cell should exist");
        assert_eq!(under_cell.contents(), "U");
        assert!(
            under_cell.underline(),
            "Text after \\x1b[4m should be underlined"
        );
    }

    #[test]
    fn test_background_color_attribute() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // \x1b[44m = blue background (color index 4), \x1b[0m = reset
        term.feed(b"normal\x1b[44mBLUEBG\x1b[0m");

        // "n" at col 0 should have default background
        let normal_cell = term.cell(0, 0).expect("Cell should exist");
        assert_eq!(
            normal_cell.bgcolor(),
            vt100::Color::Default,
            "Regular text should have default bg"
        );

        // "B" at col 6 should have blue background (index 4)
        let blue_cell = term.cell(0, 6).expect("Cell should exist");
        assert_eq!(blue_cell.contents(), "B");
        assert_eq!(
            blue_cell.bgcolor(),
            vt100::Color::Idx(4),
            "Text with \\x1b[44m should have blue background"
        );
    }

    #[test]
    fn test_combined_attributes() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // Bold + underline + red fg + blue bg: \x1b[1;4;31;44m
        term.feed(b"\x1b[1;4;31;44mSTYLED\x1b[0m");

        let cell = term.cell(0, 0).expect("Cell should exist");
        assert_eq!(cell.contents(), "S");
        assert!(cell.bold(), "Should be bold");
        assert!(cell.underline(), "Should be underlined");
        assert_eq!(
            cell.fgcolor(),
            vt100::Color::Idx(1),
            "Should have red foreground"
        );
        assert_eq!(
            cell.bgcolor(),
            vt100::Color::Idx(4),
            "Should have blue background"
        );
    }

    #[test]
    fn test_cursor_carriage_return() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // Write some text, then carriage return (should reset column to 0, same row)
        term.feed(b"Hello World");
        assert_eq!(term.cursor_position(), (0, 11));

        term.feed(b"\r");
        let (row, col) = term.cursor_position();
        assert_eq!(row, 0, "\\r should stay on same row");
        assert_eq!(col, 0, "\\r should reset column to 0");
    }

    #[test]
    fn test_cursor_line_feed() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // Write text, then line feed only (moves down, column unchanged)
        term.feed(b"Hello");
        assert_eq!(term.cursor_position(), (0, 5));

        term.feed(b"\n");
        let (row, col) = term.cursor_position();
        assert_eq!(row, 1, "\\n should move to next row");
        assert_eq!(col, 5, "\\n alone should NOT reset column");
    }

    #[test]
    fn test_cursor_home_no_args() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // Move cursor somewhere first
        term.feed(b"\x1b[10;20H"); // row 10, col 20 (1-indexed)
        assert_eq!(term.cursor_position(), (9, 19)); // 0-indexed

        // \x1b[H with no args should go to home (0, 0)
        term.feed(b"\x1b[H");
        let (row, col) = term.cursor_position();
        assert_eq!(row, 0, "\\x1b[H should go to row 0");
        assert_eq!(col, 0, "\\x1b[H should go to col 0");
    }

    #[test]
    fn test_cursor_position_comprehensive() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // Start at (0, 0)
        assert_eq!(
            term.cursor_position(),
            (0, 0),
            "Initial position should be (0, 0)"
        );

        // Type moves cursor forward
        term.feed(b"ABC");
        assert_eq!(term.cursor_position(), (0, 3), "After typing 'ABC'");

        // \r resets column
        term.feed(b"\r");
        assert_eq!(term.cursor_position(), (0, 0), "After \\r");

        // \n moves down (column stays at 0 since we just did \r)
        term.feed(b"\n");
        assert_eq!(term.cursor_position(), (1, 0), "After \\n");

        // \r\n combo (common in terminals)
        term.feed(b"XYZ\r\n");
        assert_eq!(term.cursor_position(), (2, 0), "After 'XYZ\\r\\n'");

        // Absolute positioning with \x1b[<row>;<col>H
        term.feed(b"\x1b[5;10H");
        assert_eq!(
            term.cursor_position(),
            (4, 9),
            "After \\x1b[5;10H (1-indexed to 0-indexed)"
        );

        // Home with \x1b[H
        term.feed(b"\x1b[H");
        assert_eq!(term.cursor_position(), (0, 0), "After \\x1b[H (home)");

        // Single-arg form: \x1b[<row>H (column defaults to 1)
        term.feed(b"\x1b[3H");
        assert_eq!(term.cursor_position(), (2, 0), "After \\x1b[3H (row only)");
    }

    #[test]
    fn test_resize_preserves_content() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // Write some content
        term.feed(b"Hello World\r\nLine 2\r\nLine 3");

        // Verify initial content
        assert!(term
            .get_line(0)
            .expect("row 0 should exist")
            .contains("Hello World"));
        assert!(term
            .get_line(1)
            .expect("row 1 should exist")
            .contains("Line 2"));
        assert!(term
            .get_line(2)
            .expect("row 2 should exist")
            .contains("Line 3"));

        // Resize to larger
        term.resize(TermSize {
            cols: 120,
            rows: 40,
        });
        assert_eq!(term.size(), (120, 40));

        // Content should still be there
        let line0 = term.get_line(0).expect("row 0 should exist after resize");
        assert!(
            line0.contains("Hello World"),
            "Content should survive resize: {line0}",
        );

        // Resize to smaller (content may wrap but shouldn't crash)
        term.resize(TermSize { cols: 40, rows: 10 });
        assert_eq!(term.size(), (40, 10));

        // Should not panic and get_text should work
        let text = term.get_text();
        assert!(
            text.contains("Hello"),
            "Some content should remain after shrink"
        );
    }

    #[test]
    fn test_resize_updates_cursor_bounds() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // Move cursor to row 20, col 70
        term.feed(b"\x1b[21;71H");
        assert_eq!(term.cursor_position(), (20, 70));

        // Resize to smaller - cursor should be clamped to new bounds
        term.resize(TermSize { cols: 40, rows: 10 });

        let (row, col) = term.cursor_position();
        // vt100 clamps cursor to valid range
        assert!(row < 10, "Cursor row should be within new bounds: {row}");
        assert!(col < 40, "Cursor col should be within new bounds: {col}");
    }

    #[test]
    fn test_cursor_visibility() {
        let mut term = TerminalEmulator::new(TermSize { cols: 80, rows: 24 });

        // Cursor visible by default
        assert!(term.cursor_visible(), "Cursor should be visible by default");

        // Hide cursor with DECTCEM: ESC[?25l
        term.feed(b"\x1b[?25l");
        assert!(
            !term.cursor_visible(),
            "Cursor should be hidden after ESC[?25l"
        );

        // Show cursor with DECTCEM: ESC[?25h
        term.feed(b"\x1b[?25h");
        assert!(
            term.cursor_visible(),
            "Cursor should be visible after ESC[?25h"
        );
    }
}
