//! Region detection algorithms for finding interactive elements on screen.
//!
//! Detects boxes, highlighted regions, buttons, checkboxes, etc.

use crate::snapshot::{Rect, RefId, Region, RegionType};

/// Safely convert usize to u16, saturating at u16::MAX.
///
/// This prevents overflow when dealing with extremely large terminal dimensions.
/// In practice, terminals larger than 65535 columns/rows are extremely rare,
/// but this ensures correct behavior in all cases.
#[inline]
fn saturating_u16(val: usize) -> u16 {
    val.min(u16::MAX as usize) as u16
}

/// Box-drawing corner characters (top-left).
const BOX_TOP_LEFT: &[char] = &[
    '┌', '╔', '╭', '┏', '╒', '╓', '+', // Standard corners
];

/// VT100 line-drawing mode characters.
///
/// When a terminal is in line-drawing mode (entered via ESC(0), exited via ESC(B)),
/// ASCII characters are mapped to box-drawing glyphs:
/// - 'l' = top-left corner (┌)
/// - 'k' = top-right corner (┐)
/// - 'm' = bottom-left corner (└)
/// - 'j' = bottom-right corner (┘)
/// - 'q' = horizontal line (─)
/// - 'x' = vertical line (│)
/// - 't' = left tee (├)
/// - 'u' = right tee (┤)
/// - 'n' = cross (+)
/// - 'w' = top tee (┬)
/// - 'v' = bottom tee (┴)
///
/// These are commonly used by programs like `dialog`, `whiptail`, and ncurses apps.
const VT100_LINE_DRAWING: &[char] = &[
    'l', 'k', 'm', 'j', // Corners
    'q', 'x', // Lines
    't', 'u', 'n', 'w', 'v', // Tees and cross
];

/// Box-drawing corner characters (top-right).
const BOX_TOP_RIGHT: &[char] = &[
    '┐', '╗', '╮', '┓', '╕', '╖', '+', // Standard corners
];

/// Box-drawing corner characters (bottom-left).
const BOX_BOTTOM_LEFT: &[char] = &[
    '└', '╚', '╰', '┗', '╘', '╙', '+', // Standard corners
];

/// Box-drawing corner characters (bottom-right).
const BOX_BOTTOM_RIGHT: &[char] = &[
    '┘', '╝', '╯', '┛', '╛', '╜', '+', // Standard corners
];

/// Box-drawing horizontal line characters.
const BOX_HORIZONTAL: &[char] = &[
    '─', '═', '━', '╌', '╍', '┄', '┅', '┈', '┉', '-', // Horizontal lines
];

/// Box-drawing vertical line characters.
const BOX_VERTICAL: &[char] = &[
    '│', '║', '┃', '╎', '╏', '┆', '┇', '┊', '┋', '|', // Vertical lines
];

/// Check if a character is a top-left corner.
fn is_top_left_corner(c: char) -> bool {
    BOX_TOP_LEFT.contains(&c)
}

/// Check if a character is a top-right corner.
fn is_top_right_corner(c: char) -> bool {
    BOX_TOP_RIGHT.contains(&c)
}

/// Check if a character is a bottom-left corner.
fn is_bottom_left_corner(c: char) -> bool {
    BOX_BOTTOM_LEFT.contains(&c)
}

/// Check if a character is a bottom-right corner.
fn is_bottom_right_corner(c: char) -> bool {
    BOX_BOTTOM_RIGHT.contains(&c)
}

/// Check if a character is a horizontal line.
fn is_horizontal_line(c: char) -> bool {
    BOX_HORIZONTAL.contains(&c) || is_top_left_corner(c) || is_top_right_corner(c)
}

/// Check if a character is a vertical line.
fn is_vertical_line(c: char) -> bool {
    BOX_VERTICAL.contains(&c) || is_top_left_corner(c) || is_bottom_left_corner(c)
}

/// Check if a character is a box-drawing character (Unicode or VT100 line-drawing).
fn is_box_drawing_char(c: char) -> bool {
    // Unicode box-drawing block: U+2500 to U+257F
    let is_unicode_box = ('\u{2500}'..='\u{257F}').contains(&c);

    // VT100 line-drawing characters
    let is_vt100 = VT100_LINE_DRAWING.contains(&c);

    // Our explicit lists (for + and - which aren't in unicode block)
    let is_explicit = BOX_TOP_LEFT.contains(&c)
        || BOX_TOP_RIGHT.contains(&c)
        || BOX_BOTTOM_LEFT.contains(&c)
        || BOX_BOTTOM_RIGHT.contains(&c)
        || BOX_HORIZONTAL.contains(&c)
        || BOX_VERTICAL.contains(&c);

    is_unicode_box || is_vt100 || is_explicit
}

/// Check if text is primarily box-drawing characters (chrome/decoration).
///
/// Returns true if more than half of the non-whitespace characters are box-drawing.
/// Used to filter out highlighted regions that are just UI chrome.
fn is_box_drawing_text(text: &str) -> bool {
    let mut box_count = 0;
    let mut other_count = 0;

    for c in text.chars() {
        if c.is_whitespace() {
            continue;
        }
        if is_box_drawing_char(c) {
            box_count += 1;
        } else {
            other_count += 1;
        }
    }

    // If there's nothing but whitespace, not box-drawing
    if box_count == 0 && other_count == 0 {
        return false;
    }

    // More than half are box-drawing characters
    box_count > other_count
}

/// Terminal color (standard 8/16 colors or default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    /// Bright/bold variant of the color
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    /// 256-color palette index
    Indexed(u8),
    /// RGB color
    Rgb(u8, u8, u8),
}

/// Cell attributes for styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellAttrs {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl CellAttrs {
    /// Check if this cell has a non-default background (highlighted).
    pub fn has_background(&self) -> bool {
        self.bg != Color::Default || self.inverse
    }

    /// Check if this cell is inverse video.
    pub fn is_inverse(&self) -> bool {
        self.inverse
    }
}

/// A single cell on the screen with character and attributes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cell {
    pub char: char,
    pub attrs: CellAttrs,
}

impl Cell {
    pub fn new(c: char) -> Self {
        Self {
            char: c,
            attrs: CellAttrs::default(),
        }
    }

    pub fn with_attrs(c: char, attrs: CellAttrs) -> Self {
        Self { char: c, attrs }
    }
}

/// A simple screen representation for region detection.
///
/// Supports both simple text (for box detection) and attributed cells (for highlighting).
/// Uses pre-converted char arrays for O(1) character access instead of O(n) UTF-8 indexing.
pub struct Screen {
    /// Pre-converted character rows for O(1) access in char_at().
    char_rows: Vec<Vec<char>>,
    /// Optional attributed cells for color/highlight detection.
    cells: Option<Vec<Vec<Cell>>>,
    width: usize,
    height: usize,
}

impl Screen {
    /// Create a screen from lines of text (no cell attributes).
    pub fn from_lines(lines: &[&str]) -> Self {
        let height = lines.len();
        let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);

        // Convert to Vec<char> for O(1) access (no padding, char_at handles bounds)
        let char_rows: Vec<Vec<char>> = lines.iter().map(|l| l.chars().collect()).collect();

        Self {
            char_rows,
            cells: None,
            width,
            height,
        }
    }

    /// Create a screen from a single string with newlines.
    pub fn from_text(text: &str) -> Self {
        let lines: Vec<&str> = text.lines().collect();
        Self::from_lines(&lines)
    }

    /// Create a screen from attributed cells.
    pub fn from_cells(cells: Vec<Vec<Cell>>) -> Self {
        let height = cells.len();
        let width = cells.iter().map(|row| row.len()).max().unwrap_or(0);

        // Build char rows from cells for O(1) access
        let char_rows: Vec<Vec<char>> = cells
            .iter()
            .map(|row| {
                let mut chars: Vec<char> = row.iter().map(|c| c.char).collect();
                chars.resize(width, ' ');
                chars
            })
            .collect();

        Self {
            char_rows,
            cells: Some(cells),
            width,
            height,
        }
    }

    /// Get the screen dimensions.
    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Get character at (row, col). Returns ' ' if out of bounds.
    ///
    /// This is O(1) thanks to pre-converted char arrays.
    pub fn char_at(&self, row: usize, col: usize) -> char {
        if row >= self.height || col >= self.width {
            return ' ';
        }
        // Rows may be shorter than width (no wasteful space padding)
        self.char_rows[row].get(col).copied().unwrap_or(' ')
    }

    /// Get cell at (row, col). Returns default cell if out of bounds or no cells.
    pub fn cell_at(&self, row: usize, col: usize) -> Cell {
        if let Some(ref cells) = self.cells {
            if row < cells.len() && col < cells[row].len() {
                return cells[row][col]; // Cell is Copy, no clone needed
            }
        }
        // Fallback to char-only cell
        Cell::new(self.char_at(row, col))
    }

    /// Get cell attributes at (row, col).
    pub fn attrs_at(&self, row: usize, col: usize) -> CellAttrs {
        self.cell_at(row, col).attrs
    }

    /// Get text content within a rectangle (excluding borders).
    pub fn text_in_rect(&self, x: usize, y: usize, width: usize, height: usize) -> String {
        let mut result = String::new();

        // Extract interior content (skip border)
        let inner_x = x + 1;
        let inner_y = y + 1;
        let inner_width = width.saturating_sub(2);
        let inner_height = height.saturating_sub(2);

        for row in inner_y..(inner_y + inner_height) {
            if row >= self.height {
                break;
            }
            let end_col = (inner_x + inner_width).min(self.char_rows[row].len());
            let start_col = inner_x.min(end_col);
            let line: String = self.char_rows[row][start_col..end_col].iter().collect();
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line.trim_end());
        }

        result.trim().to_string()
    }
}

/// Detect bordered boxes on the screen.
///
/// Finds rectangular regions bounded by box-drawing characters.
/// Returns regions with bounds and contained text.
pub fn detect_boxes(screen: &Screen, ref_counter: &mut u32) -> Vec<Region> {
    let mut regions = Vec::new();

    // Scan for top-left corners
    for row in 0..screen.height {
        for col in 0..screen.width {
            let c = screen.char_at(row, col);

            if is_top_left_corner(c) {
                // Try to find a complete box starting from this corner
                if let Some(region) = try_find_box(screen, row, col, ref_counter) {
                    regions.push(region);
                }
            }
        }
    }

    regions
}

/// Try to find a complete box starting from a top-left corner.
fn try_find_box(
    screen: &Screen,
    start_row: usize,
    start_col: usize,
    ref_counter: &mut u32,
) -> Option<Region> {
    // Find top-right corner (scan right along top edge)
    let mut end_col = start_col + 1;
    while end_col < screen.width {
        let c = screen.char_at(start_row, end_col);
        if is_top_right_corner(c) {
            break;
        }
        if !is_horizontal_line(c) {
            return None; // Not a continuous horizontal line
        }
        end_col += 1;
    }

    if end_col >= screen.width {
        return None; // Didn't find top-right corner
    }

    let width = end_col - start_col + 1;
    if width < 3 {
        return None; // Box too narrow
    }

    // Find bottom-left corner (scan down along left edge)
    let mut end_row = start_row + 1;
    while end_row < screen.height {
        let c = screen.char_at(end_row, start_col);
        if is_bottom_left_corner(c) {
            break;
        }
        if !is_vertical_line(c) {
            return None; // Not a continuous vertical line
        }
        end_row += 1;
    }

    if end_row >= screen.height {
        return None; // Didn't find bottom-left corner
    }

    let height = end_row - start_row + 1;
    if height < 3 {
        return None; // Box too short
    }

    // Verify bottom-right corner
    let bottom_right = screen.char_at(end_row, end_col);
    if !is_bottom_right_corner(bottom_right) {
        return None;
    }

    // Verify bottom edge (horizontal line)
    for col in (start_col + 1)..end_col {
        let c = screen.char_at(end_row, col);
        if !is_horizontal_line(c) && !is_bottom_left_corner(c) && !is_bottom_right_corner(c) {
            return None;
        }
    }

    // Verify right edge (vertical line)
    for row in (start_row + 1)..end_row {
        let c = screen.char_at(row, end_col);
        if !is_vertical_line(c) && !is_top_right_corner(c) && !is_bottom_right_corner(c) {
            return None;
        }
    }

    // Found a valid box!
    *ref_counter += 1;
    let ref_id = RefId::new(format!("@e{}", ref_counter));

    let text = screen.text_in_rect(start_col, start_row, width, height);

    Some(Region {
        ref_id,
        bounds: Rect {
            x: saturating_u16(start_col),
            y: saturating_u16(start_row),
            width: saturating_u16(width),
            height: saturating_u16(height),
        },
        region_type: RegionType::Unknown, // Will be refined by pattern detection
        text,
        focused: false,
    })
}

/// Detect highlighted regions (inverse video or colored background).
///
/// Finds contiguous runs of cells with non-default background colors.
/// These typically represent menu bars, selected items, or buttons.
pub fn detect_highlighted_regions(screen: &Screen, ref_counter: &mut u32) -> Vec<Region> {
    let mut regions = Vec::new();
    let (width, height) = screen.size();

    // Track which cells have been assigned to a region
    let mut visited = vec![vec![false; width]; height];

    for row in 0..height {
        for col in 0..width {
            if visited[row][col] {
                continue;
            }

            let attrs = screen.attrs_at(row, col);
            if !attrs.has_background() {
                continue;
            }

            // Found start of highlighted region - flood fill to find extent
            if let Some(region) = flood_fill_highlight(screen, row, col, &mut visited, ref_counter)
            {
                regions.push(region);
            }
        }
    }

    regions
}

/// Flood fill to find a contiguous highlighted region.
fn flood_fill_highlight(
    screen: &Screen,
    start_row: usize,
    start_col: usize,
    visited: &mut [Vec<bool>],
    ref_counter: &mut u32,
) -> Option<Region> {
    let (width, height) = screen.size();
    let start_attrs = screen.attrs_at(start_row, start_col);

    // Find the horizontal extent of this highlight on the starting row
    let mut min_col = start_col;
    let mut max_col = start_col;

    // Scan left
    while min_col > 0 {
        let attrs = screen.attrs_at(start_row, min_col - 1);
        if !highlights_match(&start_attrs, &attrs) {
            break;
        }
        min_col -= 1;
    }

    // Scan right
    while max_col + 1 < width {
        let attrs = screen.attrs_at(start_row, max_col + 1);
        if !highlights_match(&start_attrs, &attrs) {
            break;
        }
        max_col += 1;
    }

    // Now scan vertically to see if highlight extends to other rows
    let mut min_row = start_row;
    let mut max_row = start_row;

    // Scan up
    while min_row > 0 {
        // Check if entire horizontal range is highlighted on this row
        let mut row_matches = true;
        for col in min_col..=max_col {
            let attrs = screen.attrs_at(min_row - 1, col);
            if !highlights_match(&start_attrs, &attrs) {
                row_matches = false;
                break;
            }
        }
        if !row_matches {
            break;
        }
        min_row -= 1;
    }

    // Scan down
    while max_row + 1 < height {
        let mut row_matches = true;
        for col in min_col..=max_col {
            let attrs = screen.attrs_at(max_row + 1, col);
            if !highlights_match(&start_attrs, &attrs) {
                row_matches = false;
                break;
            }
        }
        if !row_matches {
            break;
        }
        max_row += 1;
    }

    // Mark all cells in this region as visited
    for row in visited.iter_mut().take(max_row + 1).skip(min_row) {
        for cell in row.iter_mut().take(max_col + 1).skip(min_col) {
            *cell = true;
        }
    }

    let region_width = max_col - min_col + 1;
    let region_height = max_row - min_row + 1;

    // Extract text from the region
    let mut text = String::new();
    for row in min_row..=max_row {
        if row > min_row {
            text.push('\n');
        }
        for col in min_col..=max_col {
            text.push(screen.char_at(row, col));
        }
    }
    let text = text.trim().to_string();

    // Skip empty or whitespace-only regions
    if text.is_empty() {
        return None;
    }

    // Skip regions that are primarily box-drawing characters (UI chrome)
    if is_box_drawing_text(&text) {
        return None;
    }

    *ref_counter += 1;
    let ref_id = RefId::new(format!("@e{}", ref_counter));

    // Determine region type based on context
    let region_type = if start_attrs.is_inverse() {
        // Inverse video often indicates selected menu items
        RegionType::MenuItem
    } else {
        RegionType::Unknown
    };

    Some(Region {
        ref_id,
        bounds: Rect {
            x: saturating_u16(min_col),
            y: saturating_u16(min_row),
            width: saturating_u16(region_width),
            height: saturating_u16(region_height),
        },
        region_type,
        text,
        focused: start_attrs.is_inverse(), // Inverse often means focused/selected
    })
}

/// Check if two cell attributes represent the same type of highlight.
fn highlights_match(a: &CellAttrs, b: &CellAttrs) -> bool {
    // Both must have a background highlight
    if !b.has_background() {
        return false;
    }

    // Same background color (or both inverse)
    if a.inverse && b.inverse {
        return true;
    }

    a.bg == b.bg && a.inverse == b.inverse
}

/// Detect UI patterns like buttons, checkboxes, and menu shortcuts.
///
/// Patterns detected:
/// - Buttons: `[ OK ]`, `[ Cancel ]`, `< Yes >`, `< No >`
/// - Checkboxes: `[x]`, `[ ]`, `[*]`, `[X]`
/// - Radio buttons: `(*)`, `( )`, `(o)`, `(O)`
/// - Menu shortcuts: `(F)ile`, `(E)dit` (letter in parens)
pub fn detect_patterns(screen: &Screen, ref_counter: &mut u32) -> Vec<Region> {
    let mut regions = Vec::new();
    let (width, height) = screen.size();

    for row in 0..height {
        let line: String = (0..width).map(|col| screen.char_at(row, col)).collect();

        // Detect buttons: [ text ] or < text >
        regions.extend(find_buttons(&line, row, ref_counter));

        // Detect checkboxes: [x], [ ], [*], [X]
        regions.extend(find_checkboxes(&line, row, ref_counter));

        // Detect radio buttons: (*), ( ), (o), (O)
        regions.extend(find_radio_buttons(&line, row, ref_counter));

        // Detect menu shortcuts: (F)ile, (E)dit
        regions.extend(find_menu_shortcuts(&line, row, ref_counter));
    }

    regions
}

/// Find button patterns: [ text ] or < text >
fn find_buttons(line: &str, row: usize, ref_counter: &mut u32) -> Vec<Region> {
    let mut regions = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();

    let mut i = 0;
    while i < len {
        // Look for [ or <
        if chars[i] == '[' || chars[i] == '<' {
            let open = chars[i];
            let close = if open == '[' { ']' } else { '>' };

            // Must be followed by space: "[ " or "< "
            if i + 2 < len && chars[i + 1] == ' ' {
                // Find closing bracket with space before it: " ]" or " >"
                if let Some(end) = find_button_end(&chars, i + 2, close) {
                    let button_text: String = chars[i..=end].iter().collect();
                    let inner_text: String = chars[i + 2..end - 1].iter().collect();
                    let inner_trimmed = inner_text.trim();

                    // Must have actual content (not just spaces)
                    if !inner_trimmed.is_empty() && inner_trimmed.len() <= 20 {
                        *ref_counter += 1;
                        regions.push(Region {
                            ref_id: RefId::new(format!("@e{}", ref_counter)),
                            bounds: Rect {
                                x: saturating_u16(i),
                                y: saturating_u16(row),
                                width: saturating_u16(end - i + 1),
                                height: 1,
                            },
                            region_type: RegionType::Button,
                            text: button_text,
                            focused: false,
                        });
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }

    regions
}

/// Find the end of a button pattern (space followed by closing bracket).
fn find_button_end(chars: &[char], start: usize, close: char) -> Option<usize> {
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == ' ' && chars[i + 1] == close {
            return Some(i + 1);
        }
        // Don't cross into another bracket
        if chars[i] == '[' || chars[i] == '<' || chars[i] == ']' || chars[i] == '>' {
            return None;
        }
        i += 1;
    }
    None
}

/// Find checkbox patterns: [x], [ ], [*], [X]
fn find_checkboxes(line: &str, row: usize, ref_counter: &mut u32) -> Vec<Region> {
    let mut regions = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();

    let mut i = 0;
    while i + 2 < len {
        if chars[i] == '[' && chars[i + 2] == ']' {
            let middle = chars[i + 1];
            // Checkbox markers: space (unchecked), x/X (checked), * (checked)
            if middle == ' ' || middle == 'x' || middle == 'X' || middle == '*' {
                let text: String = chars[i..i + 3].iter().collect();
                *ref_counter += 1;
                regions.push(Region {
                    ref_id: RefId::new(format!("@e{}", ref_counter)),
                    bounds: Rect {
                        x: saturating_u16(i),
                        y: saturating_u16(row),
                        width: 3,
                        height: 1,
                    },
                    region_type: RegionType::Checkbox,
                    text,
                    focused: false,
                });
                i += 3;
                continue;
            }
        }
        i += 1;
    }

    regions
}

/// Find radio button patterns: (*), ( ), (o), (O)
fn find_radio_buttons(line: &str, row: usize, ref_counter: &mut u32) -> Vec<Region> {
    let mut regions = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();

    let mut i = 0;
    while i + 2 < len {
        if chars[i] == '(' && chars[i + 2] == ')' {
            let middle = chars[i + 1];
            // Radio markers: space (unselected), */o/O/• (selected)
            if middle == ' ' || middle == '*' || middle == 'o' || middle == 'O' || middle == '•' {
                let text: String = chars[i..i + 3].iter().collect();
                *ref_counter += 1;
                regions.push(Region {
                    ref_id: RefId::new(format!("@e{}", ref_counter)),
                    bounds: Rect {
                        x: saturating_u16(i),
                        y: saturating_u16(row),
                        width: 3,
                        height: 1,
                    },
                    region_type: RegionType::RadioButton,
                    text,
                    focused: false,
                });
                i += 3;
                continue;
            }
        }
        i += 1;
    }

    regions
}

/// Find menu shortcut patterns: (F)ile, (E)dit - single letter in parens followed by text
fn find_menu_shortcuts(line: &str, row: usize, ref_counter: &mut u32) -> Vec<Region> {
    let mut regions = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();

    let mut i = 0;
    while i + 2 < len {
        // Look for (X) where X is a letter
        if chars[i] == '(' && chars[i + 2] == ')' && chars[i + 1].is_ascii_alphabetic() {
            let shortcut_char = chars[i + 1];

            // Check if followed by more letters (part of a word)
            if i + 3 < len && chars[i + 3].is_ascii_alphabetic() {
                // Find the end of the word
                let mut end = i + 3;
                while end < len && chars[end].is_ascii_alphabetic() {
                    end += 1;
                }

                let full_text: String = chars[i..end].iter().collect();

                // This looks like a menu shortcut: (F)ile, (E)dit, etc.
                *ref_counter += 1;
                regions.push(Region {
                    ref_id: RefId::new(format!("@e{}", ref_counter)),
                    bounds: Rect {
                        x: saturating_u16(i),
                        y: saturating_u16(row),
                        width: saturating_u16(end - i),
                        height: 1,
                    },
                    region_type: RegionType::MenuItem,
                    text: full_text,
                    focused: false,
                });
                i = end;
                continue;
            } else {
                // Standalone (X) - could be a radio button, skip (handled by find_radio_buttons)
                // unless it's clearly a shortcut indicator
                if shortcut_char.is_ascii_uppercase() {
                    // Might be a standalone shortcut like "(Q)uit" where Q is the only letter shown
                    // For now, skip and let radio button detection handle it if applicable
                }
            }
        }
        i += 1;
    }

    regions
}

/// Find underlined text patterns (often menu shortcuts).
/// Underline is indicated by cell attribute, not character.
pub fn detect_underlined_shortcuts(screen: &Screen, ref_counter: &mut u32) -> Vec<Region> {
    let mut regions = Vec::new();
    let (width, height) = screen.size();

    for row in 0..height {
        let mut col = 0;
        while col < width {
            let cell = screen.cell_at(row, col);

            // Look for underlined letter
            if cell.attrs.underline && cell.char.is_ascii_alphabetic() {
                // Found underlined letter - this is likely a shortcut
                // Find the surrounding word
                let (word_start, word_end) = find_word_bounds(screen, row, col);

                let text: String = (word_start..=word_end)
                    .map(|c| screen.char_at(row, c))
                    .collect();

                if !text.trim().is_empty() {
                    *ref_counter += 1;
                    regions.push(Region {
                        ref_id: RefId::new(format!("@e{}", ref_counter)),
                        bounds: Rect {
                            x: saturating_u16(word_start),
                            y: saturating_u16(row),
                            width: saturating_u16(word_end - word_start + 1),
                            height: 1,
                        },
                        region_type: RegionType::MenuItem,
                        text,
                        focused: false,
                    });
                }

                col = word_end + 1;
                continue;
            }
            col += 1;
        }
    }

    regions
}

/// Deduplicate overlapping regions, keeping the more specific one.
///
/// When multiple detectors find the same UI element (e.g., a button inside a box),
/// this removes duplicates by keeping the region with:
/// 1. A more specific region_type (not Unknown)
/// 2. A smaller bounding box (more precise)
///
/// Two regions are considered duplicates if they overlap significantly
/// (intersection area > 50% of the smaller region).
pub fn deduplicate_regions(regions: Vec<Region>) -> Vec<Region> {
    if regions.len() <= 1 {
        return regions;
    }

    let mut result: Vec<Region> = Vec::new();

    for region in regions {
        let dominated = result.iter().any(|existing| {
            if regions_overlap_significantly(&region.bounds, &existing.bounds) {
                // Check if existing region is "better"
                is_better_region(existing, &region)
            } else {
                false
            }
        });

        if dominated {
            continue;
        }

        // Remove any existing regions that this new one dominates
        result.retain(|existing| {
            if regions_overlap_significantly(&region.bounds, &existing.bounds) {
                !is_better_region(&region, existing)
            } else {
                true
            }
        });

        result.push(region);
    }

    result
}

/// Check if two rectangles overlap significantly (>50% of smaller area).
fn regions_overlap_significantly(a: &Rect, b: &Rect) -> bool {
    // Calculate intersection
    let x1 = a.x.max(b.x) as u32;
    let y1 = a.y.max(b.y) as u32;
    let x2 = (a.x as u32 + a.width as u32).min(b.x as u32 + b.width as u32);
    let y2 = (a.y as u32 + a.height as u32).min(b.y as u32 + b.height as u32);

    if x2 <= x1 || y2 <= y1 {
        return false; // No intersection
    }

    let intersection_area = (x2 - x1) * (y2 - y1);
    let area_a = a.width as u32 * a.height as u32;
    let area_b = b.width as u32 * b.height as u32;
    let smaller_area = area_a.min(area_b);

    if smaller_area == 0 {
        return false;
    }

    // Overlap is significant if intersection > 50% of smaller region
    intersection_area * 2 > smaller_area
}

/// Check if region `a` is "better" than region `b`.
///
/// A region is better if it has:
/// 1. A more specific type (not Unknown)
/// 2. Equal specificity but smaller area (more precise)
fn is_better_region(a: &Region, b: &Region) -> bool {
    let a_specific = a.region_type != RegionType::Unknown;
    let b_specific = b.region_type != RegionType::Unknown;

    if a_specific && !b_specific {
        return true; // a is more specific
    }
    if !a_specific && b_specific {
        return false; // b is more specific
    }

    // Both same specificity - prefer smaller (more precise) bounds
    let area_a = a.bounds.width as u32 * a.bounds.height as u32;
    let area_b = b.bounds.width as u32 * b.bounds.height as u32;
    area_a < area_b
}

/// Find word boundaries around a given position.
fn find_word_bounds(screen: &Screen, row: usize, col: usize) -> (usize, usize) {
    let (width, _) = screen.size();

    // Scan left for word start
    let mut start = col;
    while start > 0 {
        let c = screen.char_at(row, start - 1);
        if !c.is_ascii_alphanumeric() && c != '_' {
            break;
        }
        start -= 1;
    }

    // Scan right for word end
    let mut end = col;
    while end + 1 < width {
        let c = screen.char_at(row, end + 1);
        if !c.is_ascii_alphanumeric() && c != '_' {
            break;
        }
        end += 1;
    }

    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_simple_box() {
        let screen = Screen::from_text(
            "┌───────┐\n\
             │ Hello │\n\
             └───────┘",
        );

        let mut counter = 0;
        let regions = detect_boxes(&screen, &mut counter);

        assert_eq!(regions.len(), 1, "Should detect one box");

        let region = &regions[0];
        assert_eq!(region.bounds.x, 0);
        assert_eq!(region.bounds.y, 0);
        assert_eq!(region.bounds.width, 9);
        assert_eq!(region.bounds.height, 3);
        assert_eq!(region.text, "Hello");
        assert_eq!(region.ref_id.as_str(), "@e1");
    }

    #[test]
    fn test_detect_dialog_box() {
        let screen = Screen::from_text(
            "  ╔════════════════╗\n\
             │ ║  Save changes? ║\n\
             │ ╚════════════════╝\n\
             │                   ",
        );

        let mut counter = 0;
        let regions = detect_boxes(&screen, &mut counter);

        assert_eq!(regions.len(), 1, "Should detect dialog box");

        let region = &regions[0];
        assert_eq!(region.bounds.x, 2);
        assert_eq!(region.bounds.y, 0);
        assert!(region.text.contains("Save changes?"));
    }

    #[test]
    fn test_detect_nested_boxes() {
        let screen = Screen::from_text(
            "┌─────────────┐\n\
             │ ┌─────────┐ │\n\
             │ │  Inner  │ │\n\
             │ └─────────┘ │\n\
             └─────────────┘",
        );

        let mut counter = 0;
        let regions = detect_boxes(&screen, &mut counter);

        // Should detect both outer and inner boxes
        assert_eq!(regions.len(), 2, "Should detect two boxes");

        // Outer box
        let outer = regions.iter().find(|r| r.bounds.x == 0).unwrap();
        assert_eq!(outer.bounds.width, 15);

        // Inner box
        let inner = regions.iter().find(|r| r.bounds.x == 2).unwrap();
        assert!(inner.text.contains("Inner"));
    }

    #[test]
    fn test_regions_overlap_near_u16_max() {
        let a = Rect {
            x: u16::MAX - 1,
            y: 0,
            width: 2,
            height: 2,
        };
        let b = Rect {
            x: u16::MAX - 1,
            y: 0,
            width: 2,
            height: 2,
        };

        assert!(regions_overlap_significantly(&a, &b));
    }

    #[test]
    fn test_detect_plus_corner_box() {
        // Some TUIs use + for corners
        let screen = Screen::from_text(
            "+-------+\n\
             | ASCII |\n\
             +-------+",
        );

        let mut counter = 0;
        let regions = detect_boxes(&screen, &mut counter);

        assert_eq!(regions.len(), 1, "Should detect ASCII box");
        assert!(regions[0].text.contains("ASCII"));
    }

    #[test]
    fn test_no_box_incomplete() {
        let screen = Screen::from_text(
            "┌───────\n\
             │ Broken\n\
             └───────",
        );

        let mut counter = 0;
        let regions = detect_boxes(&screen, &mut counter);

        assert_eq!(regions.len(), 0, "Should not detect incomplete box");
    }

    #[test]
    fn test_multiline_content() {
        let screen = Screen::from_text(
            "┌──────────────┐\n\
             │ Line 1       │\n\
             │ Line 2       │\n\
             │ Line 3       │\n\
             └──────────────┘",
        );

        let mut counter = 0;
        let regions = detect_boxes(&screen, &mut counter);

        assert_eq!(regions.len(), 1);
        let text = &regions[0].text;
        assert!(text.contains("Line 1"), "Text: {}", text);
        assert!(text.contains("Line 2"), "Text: {}", text);
        assert!(text.contains("Line 3"), "Text: {}", text);
    }

    // Highlighted region tests

    fn make_highlighted_cell(c: char, inverse: bool) -> Cell {
        Cell::with_attrs(
            c,
            CellAttrs {
                inverse,
                ..Default::default()
            },
        )
    }

    fn make_bg_cell(c: char, bg: Color) -> Cell {
        Cell::with_attrs(
            c,
            CellAttrs {
                bg,
                ..Default::default()
            },
        )
    }

    #[test]
    fn test_detect_inverse_menu_item() {
        // Simulate a menu bar with one selected (inverse) item
        // "File  Edit  View"
        //        ^^^^  <- inverse
        let mut row: Vec<Cell> = vec![];

        // "File  "
        for c in "File  ".chars() {
            row.push(Cell::new(c));
        }

        // "Edit" in inverse video (selected)
        for c in "Edit".chars() {
            row.push(make_highlighted_cell(c, true));
        }

        // "  View"
        for c in "  View".chars() {
            row.push(Cell::new(c));
        }

        let screen = Screen::from_cells(vec![row]);

        let mut counter = 0;
        let regions = detect_highlighted_regions(&screen, &mut counter);

        assert_eq!(regions.len(), 1, "Should detect one highlighted region");

        let region = &regions[0];
        assert_eq!(region.text, "Edit");
        assert_eq!(region.bounds.x, 6); // starts at column 6
        assert_eq!(region.bounds.width, 4);
        assert!(region.focused, "Inverse region should be marked as focused");
        assert_eq!(region.region_type, RegionType::MenuItem);
    }

    #[test]
    fn test_detect_colored_background_button() {
        // Simulate a button with blue background: "[ OK ]"
        let mut row: Vec<Cell> = vec![];

        // Some leading space
        row.push(Cell::new(' '));
        row.push(Cell::new(' '));

        // Button with blue background
        for c in "[ OK ]".chars() {
            row.push(make_bg_cell(c, Color::Blue));
        }

        // Trailing space
        row.push(Cell::new(' '));

        let screen = Screen::from_cells(vec![row]);

        let mut counter = 0;
        let regions = detect_highlighted_regions(&screen, &mut counter);

        assert_eq!(regions.len(), 1, "Should detect button");

        let region = &regions[0];
        assert_eq!(region.text, "[ OK ]");
        assert_eq!(region.bounds.x, 2);
    }

    #[test]
    fn test_detect_multiple_highlights() {
        // Two menu items, one selected
        let mut row: Vec<Cell> = vec![];

        // First item (not selected, but has background)
        for c in "File".chars() {
            row.push(make_bg_cell(c, Color::Blue));
        }

        row.push(Cell::new(' '));

        // Second item (inverse, selected)
        for c in "Edit".chars() {
            row.push(make_highlighted_cell(c, true));
        }

        row.push(Cell::new(' '));

        // Third item (same background as first)
        for c in "View".chars() {
            row.push(make_bg_cell(c, Color::Blue));
        }

        let screen = Screen::from_cells(vec![row]);

        let mut counter = 0;
        let regions = detect_highlighted_regions(&screen, &mut counter);

        assert_eq!(regions.len(), 3, "Should detect three highlighted regions");

        // Find the "Edit" region (inverse)
        let edit_region = regions.iter().find(|r| r.text == "Edit").unwrap();
        assert!(edit_region.focused);
        assert_eq!(edit_region.region_type, RegionType::MenuItem);
    }

    #[test]
    fn test_multirow_highlight() {
        // A 2-row highlighted region (like a selected list item)
        let cells = vec![
            // Row 0: "  Item 1  " with middle part highlighted
            vec![
                Cell::new(' '),
                Cell::new(' '),
                make_highlighted_cell('I', true),
                make_highlighted_cell('t', true),
                make_highlighted_cell('e', true),
                make_highlighted_cell('m', true),
                make_highlighted_cell(' ', true),
                make_highlighted_cell('1', true),
                Cell::new(' '),
                Cell::new(' '),
            ],
            // Row 1: continuation of highlight
            vec![
                Cell::new(' '),
                Cell::new(' '),
                make_highlighted_cell('D', true),
                make_highlighted_cell('e', true),
                make_highlighted_cell('s', true),
                make_highlighted_cell('c', true),
                make_highlighted_cell('r', true),
                make_highlighted_cell('p', true),
                Cell::new(' '),
                Cell::new(' '),
            ],
        ];

        let screen = Screen::from_cells(cells);

        let mut counter = 0;
        let regions = detect_highlighted_regions(&screen, &mut counter);

        assert_eq!(regions.len(), 1, "Should detect one multi-row region");

        let region = &regions[0];
        assert_eq!(region.bounds.height, 2, "Should span 2 rows");
        assert!(region.text.contains("Item 1"));
    }

    #[test]
    fn test_no_highlight_default_attrs() {
        // Screen with no highlighting
        let screen = Screen::from_text("Just plain text");

        let mut counter = 0;
        let regions = detect_highlighted_regions(&screen, &mut counter);

        assert_eq!(regions.len(), 0, "Should not detect any highlights");
    }

    // Pattern detection tests

    #[test]
    fn test_detect_square_bracket_button() {
        let screen = Screen::from_text("  [ OK ]  [ Cancel ]  ");

        let mut counter = 0;
        let regions = detect_patterns(&screen, &mut counter);

        let buttons: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::Button)
            .collect();

        assert_eq!(buttons.len(), 2, "Should detect two buttons");

        let ok_btn = buttons.iter().find(|r| r.text.contains("OK")).unwrap();
        assert_eq!(ok_btn.text, "[ OK ]");
        assert_eq!(ok_btn.bounds.x, 2);
        assert_eq!(ok_btn.bounds.width, 6);

        let cancel_btn = buttons.iter().find(|r| r.text.contains("Cancel")).unwrap();
        assert_eq!(cancel_btn.text, "[ Cancel ]");
    }

    #[test]
    fn test_detect_angle_bracket_button() {
        let screen = Screen::from_text("< Yes >  < No >");

        let mut counter = 0;
        let regions = detect_patterns(&screen, &mut counter);

        let buttons: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::Button)
            .collect();

        assert_eq!(buttons.len(), 2, "Should detect two angle bracket buttons");

        assert!(buttons.iter().any(|r| r.text == "< Yes >"));
        assert!(buttons.iter().any(|r| r.text == "< No >"));
    }

    #[test]
    fn test_detect_checkboxes() {
        let screen = Screen::from_text("[x] Option A  [ ] Option B  [*] Option C  [X] Option D");

        let mut counter = 0;
        let regions = detect_patterns(&screen, &mut counter);

        let checkboxes: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::Checkbox)
            .collect();

        assert_eq!(checkboxes.len(), 4, "Should detect four checkboxes");

        // Verify each checkbox type was found
        assert!(checkboxes.iter().any(|r| r.text == "[x]"));
        assert!(checkboxes.iter().any(|r| r.text == "[ ]"));
        assert!(checkboxes.iter().any(|r| r.text == "[*]"));
        assert!(checkboxes.iter().any(|r| r.text == "[X]"));

        // Verify checkbox bounds
        let first = checkboxes.iter().find(|r| r.text == "[x]").unwrap();
        assert_eq!(first.bounds.x, 0);
        assert_eq!(first.bounds.width, 3);
        assert_eq!(first.bounds.height, 1);
    }

    #[test]
    fn test_detect_radio_buttons() {
        let screen = Screen::from_text("(*) Selected  ( ) Unselected  (o) Alt selected");

        let mut counter = 0;
        let regions = detect_patterns(&screen, &mut counter);

        let radios: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::RadioButton)
            .collect();

        assert_eq!(radios.len(), 3, "Should detect three radio buttons");

        assert!(radios.iter().any(|r| r.text == "(*)"));
        assert!(radios.iter().any(|r| r.text == "( )"));
        assert!(radios.iter().any(|r| r.text == "(o)"));
    }

    #[test]
    fn test_detect_menu_shortcuts() {
        let screen = Screen::from_text("(F)ile  (E)dit  (V)iew  (H)elp");

        let mut counter = 0;
        let regions = detect_patterns(&screen, &mut counter);

        let shortcuts: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::MenuItem)
            .collect();

        assert_eq!(shortcuts.len(), 4, "Should detect four menu shortcuts");

        assert!(shortcuts.iter().any(|r| r.text == "(F)ile"));
        assert!(shortcuts.iter().any(|r| r.text == "(E)dit"));
        assert!(shortcuts.iter().any(|r| r.text == "(V)iew"));
        assert!(shortcuts.iter().any(|r| r.text == "(H)elp"));

        // Verify bounds include the whole word
        let file = shortcuts.iter().find(|r| r.text == "(F)ile").unwrap();
        assert_eq!(file.bounds.width, 6); // "(F)ile" is 6 chars
    }

    #[test]
    fn test_detect_underlined_shortcuts() {
        // Create cells with underlined letters for shortcuts
        fn make_underlined_cell(c: char) -> Cell {
            Cell::with_attrs(
                c,
                CellAttrs {
                    underline: true,
                    ..Default::default()
                },
            )
        }

        // "File  Edit" where F and E are underlined
        let mut row: Vec<Cell> = vec![];

        // "File" with underlined F
        row.push(make_underlined_cell('F'));
        for c in "ile".chars() {
            row.push(Cell::new(c));
        }

        // Spaces
        row.push(Cell::new(' '));
        row.push(Cell::new(' '));

        // "Edit" with underlined E
        row.push(make_underlined_cell('E'));
        for c in "dit".chars() {
            row.push(Cell::new(c));
        }

        let screen = Screen::from_cells(vec![row]);

        let mut counter = 0;
        let regions = detect_underlined_shortcuts(&screen, &mut counter);

        assert_eq!(regions.len(), 2, "Should detect two underlined shortcuts");

        assert!(regions.iter().any(|r| r.text == "File"));
        assert!(regions.iter().any(|r| r.text == "Edit"));
    }

    #[test]
    fn test_button_not_checkbox() {
        // Ensure [ OK ] is detected as button, not checkbox
        let screen = Screen::from_text("[ OK ]");

        let mut counter = 0;
        let regions = detect_patterns(&screen, &mut counter);

        // Should be detected as button, not checkbox
        let buttons: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::Button)
            .collect();
        let checkboxes: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::Checkbox)
            .collect();

        assert_eq!(buttons.len(), 1, "Should detect as button");
        assert_eq!(checkboxes.len(), 0, "Should not detect as checkbox");
    }

    #[test]
    fn test_checkbox_not_button() {
        // Ensure [x] is detected as checkbox, not button
        let screen = Screen::from_text("[x]");

        let mut counter = 0;
        let regions = detect_patterns(&screen, &mut counter);

        let checkboxes: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::Checkbox)
            .collect();

        assert_eq!(checkboxes.len(), 1, "Should detect as checkbox");
        assert_eq!(checkboxes[0].text, "[x]");
    }

    #[test]
    fn test_mixed_patterns_multiline() {
        let screen = Screen::from_text(
            "(F)ile  (E)dit  (V)iew\n\
             [x] Option 1\n\
             [ ] Option 2\n\
             (*) Choice A\n\
             ( ) Choice B\n\
             [ OK ]  [ Cancel ]",
        );

        let mut counter = 0;
        let regions = detect_patterns(&screen, &mut counter);

        let buttons: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::Button)
            .collect();
        let checkboxes: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::Checkbox)
            .collect();
        let radios: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::RadioButton)
            .collect();
        let menus: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::MenuItem)
            .collect();

        assert_eq!(menus.len(), 3, "Should detect 3 menu shortcuts");
        assert_eq!(checkboxes.len(), 2, "Should detect 2 checkboxes");
        assert_eq!(radios.len(), 2, "Should detect 2 radio buttons");
        assert_eq!(buttons.len(), 2, "Should detect 2 buttons");
    }

    #[test]
    fn test_ref_ids_increment() {
        let screen = Screen::from_text("[x] [ ] (*)");

        let mut counter = 5; // Start from 5
        let regions = detect_patterns(&screen, &mut counter);

        assert_eq!(regions.len(), 3);
        assert_eq!(regions[0].ref_id.as_str(), "@e6");
        assert_eq!(regions[1].ref_id.as_str(), "@e7");
        assert_eq!(regions[2].ref_id.as_str(), "@e8");
        assert_eq!(counter, 8);
    }

    #[test]
    fn test_button_requires_space_padding() {
        // "[OK]" without spaces should NOT be detected as a button
        // (it could be a checkbox-like indicator)
        let screen = Screen::from_text("[OK]  [Cancel]");

        let mut counter = 0;
        let regions = detect_patterns(&screen, &mut counter);

        let buttons: Vec<_> = regions
            .iter()
            .filter(|r| r.region_type == RegionType::Button)
            .collect();

        assert_eq!(
            buttons.len(),
            0,
            "Buttons without space padding should not be detected"
        );
    }

    #[test]
    fn test_detect_patterns_correct_row() {
        let screen = Screen::from_text(
            "Line 0\n\
             Line 1 [ OK ]\n\
             Line 2",
        );

        let mut counter = 0;
        let regions = detect_patterns(&screen, &mut counter);

        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].bounds.y, 1, "Button should be on row 1");
    }

    #[test]
    fn test_deduplicate_removes_overlapping_unknown() {
        // When a button is detected by both box detector (Unknown) and pattern detector (Button),
        // deduplication should keep only the Button (more specific type)
        let regions = vec![
            Region {
                ref_id: RefId::new("@e1"),
                bounds: Rect {
                    x: 10,
                    y: 5,
                    width: 7,
                    height: 1,
                },
                region_type: RegionType::Unknown,
                text: "< Yes >".to_string(),
                focused: false,
            },
            Region {
                ref_id: RefId::new("@e2"),
                bounds: Rect {
                    x: 10,
                    y: 5,
                    width: 7,
                    height: 1,
                },
                region_type: RegionType::Button,
                text: "< Yes >".to_string(),
                focused: false,
            },
        ];

        let deduped = deduplicate_regions(regions);

        assert_eq!(deduped.len(), 1, "Should keep only one region");
        assert_eq!(
            deduped[0].region_type,
            RegionType::Button,
            "Should keep the Button (more specific)"
        );
    }

    #[test]
    fn test_deduplicate_keeps_non_overlapping() {
        // Regions that don't overlap should both be kept
        let regions = vec![
            Region {
                ref_id: RefId::new("@e1"),
                bounds: Rect {
                    x: 10,
                    y: 5,
                    width: 7,
                    height: 1,
                },
                region_type: RegionType::Button,
                text: "< Yes >".to_string(),
                focused: false,
            },
            Region {
                ref_id: RefId::new("@e2"),
                bounds: Rect {
                    x: 30,
                    y: 5,
                    width: 6,
                    height: 1,
                },
                region_type: RegionType::Button,
                text: "< No >".to_string(),
                focused: false,
            },
        ];

        let deduped = deduplicate_regions(regions);

        assert_eq!(deduped.len(), 2, "Should keep both non-overlapping regions");
    }

    #[test]
    fn test_deduplicate_prefers_smaller_when_same_type() {
        // When both have the same type, prefer the smaller (more precise) region
        let regions = vec![
            Region {
                ref_id: RefId::new("@e1"),
                bounds: Rect {
                    x: 5,
                    y: 5,
                    width: 20,
                    height: 5,
                },
                region_type: RegionType::Unknown,
                text: "Large box".to_string(),
                focused: false,
            },
            Region {
                ref_id: RefId::new("@e2"),
                bounds: Rect {
                    x: 10,
                    y: 7,
                    width: 7,
                    height: 1,
                },
                region_type: RegionType::Unknown,
                text: "< OK >".to_string(),
                focused: false,
            },
        ];

        let deduped = deduplicate_regions(regions);

        assert_eq!(deduped.len(), 1, "Should deduplicate overlapping same-type");
        assert_eq!(deduped[0].text, "< OK >", "Should keep the smaller region");
    }

    // Box-drawing character detection tests

    #[test]
    fn test_is_box_drawing_char_unicode() {
        // Unicode box-drawing characters
        assert!(is_box_drawing_char('┌'));
        assert!(is_box_drawing_char('┐'));
        assert!(is_box_drawing_char('└'));
        assert!(is_box_drawing_char('┘'));
        assert!(is_box_drawing_char('─'));
        assert!(is_box_drawing_char('│'));
        assert!(is_box_drawing_char('╔'));
        assert!(is_box_drawing_char('═'));
    }

    #[test]
    fn test_is_box_drawing_char_vt100() {
        // VT100 line-drawing mode characters
        assert!(is_box_drawing_char('l')); // top-left
        assert!(is_box_drawing_char('k')); // top-right
        assert!(is_box_drawing_char('m')); // bottom-left
        assert!(is_box_drawing_char('j')); // bottom-right
        assert!(is_box_drawing_char('q')); // horizontal
        assert!(is_box_drawing_char('x')); // vertical
        assert!(is_box_drawing_char('t')); // left tee
        assert!(is_box_drawing_char('u')); // right tee
        assert!(is_box_drawing_char('n')); // cross
    }

    #[test]
    fn test_is_box_drawing_char_ascii_fallback() {
        // ASCII fallback characters used for boxes
        assert!(is_box_drawing_char('+'));
        assert!(is_box_drawing_char('-'));
        assert!(is_box_drawing_char('|'));
    }

    #[test]
    fn test_is_box_drawing_char_regular() {
        // Regular characters should not be box-drawing
        assert!(!is_box_drawing_char('a'));
        assert!(!is_box_drawing_char('Z'));
        assert!(!is_box_drawing_char('0'));
        assert!(!is_box_drawing_char(' '));
        assert!(!is_box_drawing_char('!'));
        assert!(!is_box_drawing_char('<'));
        assert!(!is_box_drawing_char('>'));
    }

    #[test]
    fn test_is_box_drawing_text_vt100_corners() {
        // VT100 line-drawing: "mqj" (bottom-left, horizontal, bottom-right)
        assert!(is_box_drawing_text("mqj"));
        assert!(is_box_drawing_text("lqk")); // top-left, horizontal, top-right
        assert!(is_box_drawing_text("tqu")); // left-tee, horizontal, right-tee
    }

    #[test]
    fn test_is_box_drawing_text_unicode() {
        assert!(is_box_drawing_text("┌──┐"));
        assert!(is_box_drawing_text("└──┘"));
        assert!(is_box_drawing_text("│  │")); // mostly whitespace, but has vertical lines
    }

    #[test]
    fn test_is_box_drawing_text_mixed() {
        // More box than content = box drawing
        assert!(is_box_drawing_text("lqqqk"));
        // More content than box = not box drawing
        assert!(!is_box_drawing_text("Hello"));
        assert!(!is_box_drawing_text("< Yes >"));
        assert!(!is_box_drawing_text("[ OK ]"));
    }

    #[test]
    fn test_is_box_drawing_text_edge_cases() {
        // Empty string
        assert!(!is_box_drawing_text(""));
        // Only whitespace
        assert!(!is_box_drawing_text("   "));
        // Equal box and non-box (tie goes to not box-drawing due to > not >=)
        assert!(!is_box_drawing_text("lA")); // 1 box, 1 other
    }

    #[test]
    fn test_highlight_skips_box_drawing_text() {
        // Create cells with highlighted VT100 line-drawing characters
        // This simulates what dialog does when rendering box borders with background colors
        let mut row: Vec<Cell> = vec![];

        // "mqj" (bottom corners) with inverse video
        for c in "mqj".chars() {
            row.push(make_highlighted_cell(c, true));
        }

        let screen = Screen::from_cells(vec![row]);

        let mut counter = 0;
        let regions = detect_highlighted_regions(&screen, &mut counter);

        assert_eq!(
            regions.len(),
            0,
            "Should skip highlighted box-drawing characters"
        );
    }

    #[test]
    fn test_highlight_keeps_real_content() {
        // Real content like "Edit" should still be detected
        let mut row: Vec<Cell> = vec![];

        for c in "Edit".chars() {
            row.push(make_highlighted_cell(c, true));
        }

        let screen = Screen::from_cells(vec![row]);

        let mut counter = 0;
        let regions = detect_highlighted_regions(&screen, &mut counter);

        assert_eq!(regions.len(), 1, "Should detect real highlighted content");
        assert_eq!(regions[0].text, "Edit");
    }
}
