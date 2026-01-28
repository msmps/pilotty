//! Classification: converting clusters into interactive elements.
//!
//! The classifier applies priority-ordered rules to determine each cluster's
//! kind. Only interactive elements (Button, Input, Toggle) are returned;
//! non-interactive content stays in `snapshot.text`.
//!
//! # Rule Priority (highest to lowest)
//!
//! 1. Cursor position → Input (confidence: 1.0, focused: true)
//! 2. Checkbox patterns `[x]`, `[ ]`, `☑`, `☐` → Toggle (confidence: 1.0)
//! 3. Inverse video → Button (confidence: 1.0, focused: true)
//! 4. Bracket patterns `[OK]`, `<Cancel>` → Button (confidence: 0.8)
//! 5. Underscore field `____` → Input (confidence: 0.6)
//!
//! Non-interactive patterns (links, progress bars, errors, status indicators,
//! box-drawing, menu prefixes, static text) are filtered out.

use unicode_width::UnicodeWidthStr;

use crate::elements::segment::Cluster;
use crate::elements::{Element, ElementKind};

// ============================================================================
// Constants
// ============================================================================

/// Maximum cluster text length to process for tokenization.
/// Protects against memory exhaustion from malicious terminal output.
/// Terminal lines rarely exceed this; longer text won't contain meaningful UI elements.
const MAX_CLUSTER_TEXT_LEN: usize = 4096;

// ============================================================================
// Token Extraction
// ============================================================================

/// A token extracted from a cluster's text.
///
/// Tokens are sub-patterns within a cluster that match interactive elements:
/// - Bracketed tokens: `[OK]`, `<Cancel>`, `[ ]`, `[x]`
/// - Underscore runs: `____`, `__________`
#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    /// Text content of the token.
    text: String,
    /// Byte offset from start of cluster text (used to slice prefix for width calculation).
    byte_offset: usize,
}

/// Calculate the display-width column offset for a token within cluster text.
///
/// This handles CJK characters correctly (width 2) by computing the display
/// width of the text prefix before the token.
fn token_col_offset(text: &str, byte_offset: usize) -> u16 {
    text.get(..byte_offset)
        .map(|prefix| prefix.width().min(u16::MAX as usize) as u16)
        .unwrap_or(0)
}

/// Extract bracketed tokens from text.
///
/// Finds patterns like `[OK]`, `<Cancel>`, `(Submit)`, `[ ]`, `[x]`.
/// Returns tokens with their byte offsets within the text (for display width calculation).
///
/// Returns empty if text exceeds MAX_CLUSTER_TEXT_LEN to prevent memory exhaustion.
fn extract_bracketed_tokens(text: &str) -> Vec<Token> {
    // Protect against memory exhaustion from extremely long input
    if text.len() > MAX_CLUSTER_TEXT_LEN {
        return Vec::new();
    }

    let mut tokens = Vec::new();

    for (char_idx, ch) in text.char_indices() {
        // Look for opening brackets
        let close_bracket = match ch {
            '[' => Some(']'),
            '<' => Some('>'),
            '(' => Some(')'),
            '【' => Some('】'),
            '「' => Some('」'),
            _ => None,
        };

        if let Some(closer) = close_bracket {
            // Find matching closer in the remainder of the string
            if let Some(end_rel) = text[char_idx + ch.len_utf8()..].find(closer) {
                let token_start = char_idx;
                let token_end = char_idx + ch.len_utf8() + end_rel + closer.len_utf8();
                let token_text = &text[token_start..token_end];

                // Only extract if it looks interactive (not just empty or single char)
                if token_text.chars().count() >= 3 || is_unicode_checkbox(token_text) {
                    tokens.push(Token {
                        text: token_text.to_string(),
                        byte_offset: token_start,
                    });
                }
            }
        }
    }

    // Deduplicate overlapping tokens by keeping only non-overlapping ones
    let mut result = Vec::new();
    let mut last_end = 0;
    for token in tokens {
        if token.byte_offset >= last_end {
            last_end = token.byte_offset + token.text.len();
            result.push(token);
        }
    }

    result
}

/// Check if text is a single unicode checkbox character.
fn is_unicode_checkbox(text: &str) -> bool {
    matches!(text, "☑" | "☐" | "□" | "✓" | "✔" | "☒")
}

/// Extract underscore runs from text.
///
/// Finds patterns like `____`, `__________` (3+ underscores).
/// Returns tokens with their byte offsets within the text (for display width calculation).
///
/// Returns empty if text exceeds MAX_CLUSTER_TEXT_LEN to prevent memory exhaustion.
fn extract_underscore_runs(text: &str) -> Vec<Token> {
    // Protect against memory exhaustion from extremely long input
    if text.len() > MAX_CLUSTER_TEXT_LEN {
        return Vec::new();
    }

    let mut tokens = Vec::new();
    let mut in_run = false;
    let mut run_start = 0;

    for (byte_idx, ch) in text.char_indices() {
        if ch == '_' {
            if !in_run {
                in_run = true;
                run_start = byte_idx;
            }
        } else if in_run {
            // End of underscore run
            let run_text = &text[run_start..byte_idx];
            if run_text.len() >= 3 {
                tokens.push(Token {
                    text: run_text.to_string(),
                    byte_offset: run_start,
                });
            }
            in_run = false;
        }
    }

    // Handle run at end of string
    if in_run {
        let run_text = &text[run_start..];
        if run_text.len() >= 3 {
            tokens.push(Token {
                text: run_text.to_string(),
                byte_offset: run_start,
            });
        }
    }

    tokens
}

/// Context for classification decisions that depend on screen position.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClassifyContext {
    /// Optional cursor row (if known). Clusters at cursor position become Input.
    pub cursor_row: Option<u16>,
    /// Optional cursor column (if known).
    pub cursor_col: Option<u16>,
}

impl ClassifyContext {
    /// Create a new context with no cursor information.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set cursor position.
    #[must_use]
    pub fn with_cursor(mut self, row: u16, col: u16) -> Self {
        self.cursor_row = Some(row);
        self.cursor_col = Some(col);
        self
    }
}

/// Internal element data during classification.
///
/// Used during classification to collect elements before converting
/// to the public Element type.
#[derive(Debug, Clone)]
struct DetectedElement {
    kind: ElementKind,
    row: u16,
    col: u16,
    width: u16,
    text: String,
    confidence: f32,
    checked: Option<bool>,
    focused: bool,
}

impl DetectedElement {
    /// Create a button element.
    fn button(row: u16, col: u16, text: String, confidence: f32, focused: bool) -> Self {
        Self {
            kind: ElementKind::Button,
            row,
            col,
            width: text.width().min(u16::MAX as usize) as u16,
            text,
            confidence,
            checked: None,
            focused,
        }
    }

    /// Create an input element.
    fn input(row: u16, col: u16, text: String, confidence: f32, focused: bool) -> Self {
        Self {
            kind: ElementKind::Input,
            row,
            col,
            width: text.width().min(u16::MAX as usize) as u16,
            text,
            confidence,
            checked: None,
            focused,
        }
    }

    /// Create a toggle element.
    fn toggle(row: u16, col: u16, text: String, checked: bool) -> Self {
        Self {
            kind: ElementKind::Toggle,
            row,
            col,
            width: text.width().min(u16::MAX as usize) as u16,
            text,
            confidence: 1.0,
            checked: Some(checked),
            focused: false,
        }
    }

    /// Convert to Element.
    fn into_element(self) -> Element {
        let mut elem = Element::new(
            self.kind,
            self.row,
            self.col,
            self.width,
            self.text,
            self.confidence,
        );
        if let Some(checked) = self.checked {
            elem = elem.with_checked(checked);
        }
        if self.focused {
            elem = elem.with_focused(true);
        }
        elem
    }
}

// ============================================================================
// Pattern Detection Helpers
// ============================================================================

/// Check if text matches a single button bracket pattern: `[OK]`, `<Cancel>`, `(Confirm)`
///
/// Requires:
/// - Exactly one pair of matching brackets
/// - At least one non-bracket character inside
/// - No brackets in the interior (to reject `[Yes] [No]`)
fn is_button_pattern(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 3 {
        return false;
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let first = chars[0];
    let last = chars[chars.len() - 1];

    // Check for matching bracket pairs
    let (opener, closer) = match (first, last) {
        ('[', ']') => ('[', ']'),
        ('<', '>') => ('<', '>'),
        ('(', ')') => ('(', ')'),
        ('【', '】') => ('【', '】'),
        ('「', '」') => ('「', '」'),
        _ => return false,
    };

    // Interior must have non-whitespace content (not just empty brackets)
    let interior: String = chars[1..chars.len() - 1].iter().collect();

    // Reject if interior contains more brackets (e.g., "[Yes] [No]")
    if interior.contains(opener) || interior.contains(closer) {
        return false;
    }

    // Reject if it looks like a checkbox pattern
    if is_checkbox_content(&interior) {
        return false;
    }

    // Reject if it looks like a progress bar inside brackets
    if is_progress_bar_content(&interior) {
        return false;
    }

    // Must have actual label content
    !interior.trim().is_empty()
}

/// Helper to check if content inside brackets looks like progress bar content.
fn is_progress_bar_content(content: &str) -> bool {
    if content.is_empty() {
        return false;
    }

    // Count progress-bar typical characters
    let progress_chars: usize = content
        .chars()
        .filter(|&c| matches!(c, '=' | '>' | '-' | '#' | ' ' | '█' | '░'))
        .count();

    // If more than 80% of chars are progress-like, it's probably a progress bar
    progress_chars * 10 >= content.len() * 8
}

/// Check if text matches checkbox patterns.
///
/// Supported patterns:
/// - `[x]`, `[X]`, `[ ]` - ASCII checkboxes
/// - `[*]`, `[-]` - Alternative markers
/// - `☑`, `☐`, `✓`, `✗` - Unicode checkboxes
/// - `(x)`, `( )`, `(*)` - Parenthesized variants
fn is_checkbox_pattern(text: &str) -> Option<bool> {
    let trimmed = text.trim();

    // Single character unicode checkboxes
    match trimmed {
        "☑" | "✓" | "✔" | "☒" => return Some(true),
        "☐" | "□" => return Some(false),
        _ => {}
    }

    // Bracketed checkboxes: [x], [ ], [*], [-], etc.
    if trimmed.len() == 3 {
        let chars: Vec<char> = trimmed.chars().collect();
        if (chars[0] == '[' && chars[2] == ']') || (chars[0] == '(' && chars[2] == ')') {
            return match chars[1] {
                'x' | 'X' | '*' | '✓' | '✔' => Some(true),
                ' ' | '.' => Some(false),
                '-' => Some(false), // indeterminate treated as unchecked
                _ => None,
            };
        }
    }

    None
}

/// Helper to check if content inside brackets looks like checkbox content.
fn is_checkbox_content(content: &str) -> bool {
    let trimmed = content.trim();
    matches!(trimmed, "x" | "X" | " " | "*" | "-" | "✓" | "✔")
}

/// Check if text looks like an input field placeholder.
///
/// Patterns: `____`, `[          ]`, `: _____`
fn is_input_pattern(text: &str) -> bool {
    let trimmed = text.trim();

    // Series of underscores
    if trimmed.chars().all(|c| c == '_') && trimmed.len() >= 3 {
        return true;
    }

    // Empty bracketed field with mostly spaces
    if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 4 {
        let inner: String = trimmed.chars().skip(1).take(trimmed.len() - 2).collect();
        if inner.trim().is_empty() && inner.len() >= 2 {
            return true;
        }
    }

    // Colon followed by underscores: "Name: ___"
    if let Some(colon_pos) = trimmed.find(':') {
        let after_colon = trimmed[colon_pos + 1..].trim_start();
        if after_colon.chars().all(|c| c == '_') && after_colon.len() >= 3 {
            return true;
        }
    }

    false
}

// ============================================================================
// Core Classification
// ============================================================================

/// Classify a text pattern into a detected element at the given position.
///
/// This is the low-level classifier that doesn't consider tokenization.
/// Returns `None` for non-interactive patterns.
///
/// Classification priority:
/// 1. Checkbox patterns → Toggle (state is unambiguous)
/// 2. Inverse video → Button (focused) - TUI convention for selection
/// 3. Bracket patterns → Button (with focus if cursor present)
/// 4. Underscore/labeled fields → Input (with focus if cursor present)
/// 5. Cursor on unrecognized text → Input (fallback for editable regions)
fn classify_text(
    text: &str,
    row: u16,
    col: u16,
    is_inverse: bool,
    cursor_in_range: bool,
) -> Option<DetectedElement> {
    // Rule 1: Checkbox patterns → Toggle
    // Checkboxes have unambiguous visual state, highest confidence
    if let Some(checked) = is_checkbox_pattern(text) {
        return Some(DetectedElement::toggle(row, col, text.to_string(), checked));
    }

    // Rule 2: Inverse video → Button (focused)
    // TUI convention: inverse video = selected/focused item
    if is_inverse {
        return Some(DetectedElement::button(
            row,
            col,
            text.to_string(),
            1.0,
            true,
        ));
    }

    // Rule 3: Bracket patterns → Button
    // Cursor on button makes it focused, not an input
    if is_button_pattern(text) {
        return Some(DetectedElement::button(
            row,
            col,
            text.to_string(),
            if cursor_in_range { 1.0 } else { 0.8 },
            cursor_in_range,
        ));
    }

    // Rule 4: Underscore field → Input
    if is_input_pattern(text) {
        return Some(DetectedElement::input(
            row,
            col,
            text.to_string(),
            if cursor_in_range { 1.0 } else { 0.6 },
            cursor_in_range,
        ));
    }

    // Rule 5: Cursor on unrecognized pattern → Input (fallback)
    // If cursor is here and we don't know what it is, assume editable
    if cursor_in_range {
        return Some(DetectedElement::input(
            row,
            col,
            text.to_string(),
            1.0,
            true,
        ));
    }

    None
}

/// Check if cursor is within a range.
///
/// Uses saturating arithmetic to prevent overflow when col + width exceeds u16::MAX.
fn cursor_in_range(ctx: &ClassifyContext, row: u16, col: u16, width: u16) -> bool {
    if let (Some(cursor_row), Some(cursor_col)) = (ctx.cursor_row, ctx.cursor_col) {
        cursor_row == row && cursor_col >= col && cursor_col < col.saturating_add(width)
    } else {
        false
    }
}

/// Extract elements from a cluster using tokenization.
///
/// If the cluster contains bracketed tokens or underscore runs, those are
/// extracted as separate elements. The parent cluster is dropped if tokens
/// are found (tokens win, inherit parent's focus if inverse).
///
/// This handles cases like:
/// - `"Save [OK] Cancel"` → extracts `[OK]` as Button
/// - `"Name: ____"` → extracts `____` as Input
fn extract_elements_from_cluster(cluster: &Cluster, ctx: &ClassifyContext) -> Vec<DetectedElement> {
    let row = cluster.row;
    let col = cluster.col;
    let text = &cluster.text;
    let is_inverse = cluster.style.is_inverse();

    // First, try to classify the whole cluster
    let cursor_hit = cursor_in_range(ctx, row, col, cluster.width);
    let whole_cluster_elem = classify_text(text, row, col, is_inverse, cursor_hit);

    // Check if the whole cluster is already a "tight" interactive pattern
    // (checkbox, bracketed button, or underscore-only input)
    if let Some(ref elem) = whole_cluster_elem {
        // If it's a toggle (checkbox pattern), return immediately
        if elem.kind == ElementKind::Toggle {
            return vec![elem.clone()];
        }

        // If it's a bracket button and the text is entirely the bracket pattern
        if elem.kind == ElementKind::Button && is_button_pattern(text) {
            return vec![elem.clone()];
        }

        // If it's an input and the text is entirely underscores
        if elem.kind == ElementKind::Input && text.trim().chars().all(|c| c == '_') {
            return vec![elem.clone()];
        }
    }

    // Try to extract tokens from within the cluster
    let mut elements = Vec::new();
    let parent_focused = is_inverse; // Tokens inherit focus from inverse parent

    // Extract bracketed tokens
    for token in extract_bracketed_tokens(text) {
        let token_col = col + token_col_offset(text, token.byte_offset);
        let token_cursor_hit = cursor_in_range(ctx, row, token_col, token.text.width() as u16);

        // Classify the token text
        // Note: tokens extracted from inverse clusters inherit focus
        if let Some(mut elem) = classify_text(&token.text, row, token_col, false, token_cursor_hit)
        {
            if parent_focused && !elem.focused {
                elem.focused = true;
                // Upgrade confidence if inheriting focus
                if elem.confidence < 1.0 {
                    elem.confidence = 1.0;
                }
            }
            elements.push(elem);
        }
    }

    // Extract underscore runs (only if no bracketed tokens found)
    if elements.is_empty() {
        for token in extract_underscore_runs(text) {
            let token_col = col + token_col_offset(text, token.byte_offset);
            let token_cursor_hit = cursor_in_range(ctx, row, token_col, token.text.width() as u16);

            if let Some(mut elem) =
                classify_text(&token.text, row, token_col, false, token_cursor_hit)
            {
                if parent_focused && !elem.focused {
                    elem.focused = true;
                    elem.confidence = 1.0;
                }
                elements.push(elem);
            }
        }
    }

    // If tokens were found, return them (dedup rule: tokens win)
    if !elements.is_empty() {
        return elements;
    }

    // No tokens found, return whole cluster classification if any
    whole_cluster_elem.into_iter().collect()
}

/// Classify clusters into interactive elements.
///
/// Uses tokenization to extract sub-elements from clusters. If a cluster
/// contains bracketed tokens or underscore runs, those are extracted as
/// separate elements and the parent cluster is dropped (dedup rule).
///
/// Only returns interactive elements (Button, Input, Toggle).
/// Non-interactive clusters are filtered out.
///
/// Elements are sorted by position (row, then col) for consistent ordering.
#[must_use]
pub fn classify(clusters: Vec<Cluster>, ctx: &ClassifyContext) -> Vec<Element> {
    let mut detected: Vec<DetectedElement> = Vec::new();

    for cluster in clusters {
        detected.extend(extract_elements_from_cluster(&cluster, ctx));
    }

    // Sort by position (row, then col) for consistent ordering
    detected.sort_by(|a, b| (a.row, a.col).cmp(&(b.row, b.col)));

    // Convert to Elements
    detected
        .into_iter()
        .map(|elem| elem.into_element())
        .collect()
}

/// Convenience function: segment a grid and classify in one step.
///
/// This is the main entry point for element detection.
#[must_use]
pub fn detect<G: crate::elements::grid::ScreenGrid>(
    grid: &G,
    ctx: &ClassifyContext,
) -> Vec<Element> {
    let clusters = crate::elements::segment::segment(grid);
    classify(clusters, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elements::grid::test_support::SimpleGrid;
    use crate::elements::segment::Cluster;
    use crate::elements::style::CellStyle;

    fn cluster(text: &str) -> Cluster {
        Cluster::new(0, 0, text.to_string(), CellStyle::default())
    }

    fn cluster_at(row: u16, col: u16, text: &str) -> Cluster {
        Cluster::new(row, col, text.to_string(), CellStyle::default())
    }

    fn inverse_cluster(text: &str) -> Cluster {
        Cluster::new(0, 0, text.to_string(), CellStyle::new().with_inverse(true))
    }

    fn classify_cluster(cluster: &Cluster, ctx: &ClassifyContext) -> Option<DetectedElement> {
        extract_elements_from_cluster(cluster, ctx)
            .into_iter()
            .next()
    }

    #[test]
    fn button_bracket_patterns() {
        let ctx = ClassifyContext::new();

        let result = classify_cluster(&cluster("[OK]"), &ctx).unwrap();
        assert_eq!(result.kind, ElementKind::Button);
        assert!((result.confidence - 0.8).abs() < f32::EPSILON);

        assert_eq!(
            classify_cluster(&cluster("<Cancel>"), &ctx).unwrap().kind,
            ElementKind::Button
        );
        assert_eq!(
            classify_cluster(&cluster("(Submit)"), &ctx).unwrap().kind,
            ElementKind::Button
        );
    }

    #[test]
    fn toggle_checkbox_patterns() {
        let ctx = ClassifyContext::new();

        let checked = classify_cluster(&cluster("[x]"), &ctx).unwrap();
        assert_eq!(checked.kind, ElementKind::Toggle);
        assert_eq!(checked.checked, Some(true));

        let unchecked = classify_cluster(&cluster("[ ]"), &ctx).unwrap();
        assert_eq!(unchecked.kind, ElementKind::Toggle);
        assert_eq!(unchecked.checked, Some(false));
    }

    #[test]
    fn input_patterns() {
        let ctx = ClassifyContext::new();

        let underscore = classify_cluster(&cluster("_____"), &ctx).unwrap();
        assert_eq!(underscore.kind, ElementKind::Input);
        assert!((underscore.confidence - 0.6).abs() < f32::EPSILON);

        // Cursor position creates focused input
        let ctx_cursor = ClassifyContext::new().with_cursor(0, 5);
        let cursor_input = classify_cluster(&cluster_at(0, 0, "some text"), &ctx_cursor).unwrap();
        assert_eq!(cursor_input.kind, ElementKind::Input);
        assert!(cursor_input.focused);
    }

    #[test]
    fn inverse_video_creates_focused_button() {
        let ctx = ClassifyContext::new();
        let result = classify_cluster(&inverse_cluster("File"), &ctx).unwrap();
        assert_eq!(result.kind, ElementKind::Button);
        assert!(result.focused);
        assert!((result.confidence - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn non_interactive_filtered() {
        let ctx = ClassifyContext::new();
        assert!(classify_cluster(&cluster("Hello World"), &ctx).is_none());
        assert!(classify_cluster(&cluster("https://example.com"), &ctx).is_none());
    }

    #[test]
    fn classify_returns_sorted_elements() {
        let ctx = ClassifyContext::new();
        let clusters = vec![cluster("[OK]"), cluster("[Cancel]"), cluster("[ ]")];
        let elements = classify(clusters, &ctx);

        assert_eq!(elements.len(), 3);
        assert_eq!(elements[0].kind, ElementKind::Button);
        assert_eq!(elements[1].kind, ElementKind::Button);
        assert_eq!(elements[2].kind, ElementKind::Toggle);
    }

    #[test]
    fn detect_full_pipeline() {
        let mut grid = SimpleGrid::from_text(&["[OK] [Cancel] [ ]"], 20);
        let inverse = CellStyle::new().with_inverse(true);
        let bold = CellStyle::new().with_bold(true);

        grid.style_range(0, 0, 4, inverse);
        grid.style_range(0, 5, 13, bold);

        let elements = detect(&grid, &ClassifyContext::new());
        let kinds: Vec<ElementKind> = elements.iter().map(|e| e.kind).collect();

        assert!(kinds.contains(&ElementKind::Button));
        assert!(kinds.contains(&ElementKind::Toggle));
    }

    #[test]
    fn tokenizer_extracts_from_text() {
        let tokens = extract_bracketed_tokens("Save [OK] [Cancel]");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].text, "[OK]");
        assert_eq!(tokens[1].text, "[Cancel]");
    }

    #[test]
    fn dedup_extracts_button_from_text() {
        let ctx = ClassifyContext::new();
        let elements = extract_elements_from_cluster(&cluster("Save [OK] Cancel"), &ctx);

        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].text, "[OK]");
        assert_eq!(elements[0].col, 5);
    }

    // ========================================================================
    // Security & Edge Case Tests
    // ========================================================================

    #[test]
    fn extract_tokens_rejects_oversized_input() {
        // Verify that extremely long text is rejected to prevent memory exhaustion
        let huge_text = "[".repeat(MAX_CLUSTER_TEXT_LEN + 1);
        assert!(extract_bracketed_tokens(&huge_text).is_empty());

        let huge_underscores = "_".repeat(MAX_CLUSTER_TEXT_LEN + 1);
        assert!(extract_underscore_runs(&huge_underscores).is_empty());
    }

    #[test]
    fn cursor_in_range_handles_overflow() {
        // Verify saturating_add prevents overflow panic
        let ctx = ClassifyContext::new().with_cursor(0, u16::MAX);

        // Should not panic even with extreme values
        assert!(!cursor_in_range(&ctx, 0, u16::MAX - 10, 100));

        // Cursor near MAX should still work correctly
        let ctx = ClassifyContext::new().with_cursor(0, u16::MAX - 5);
        assert!(cursor_in_range(&ctx, 0, u16::MAX - 10, 10));
    }

    // ========================================================================
    // Unicode Width Tests
    // ========================================================================

    #[test]
    fn element_width_cjk() {
        // CJK characters should have width 2 each
        let ctx = ClassifyContext::new();
        let elem = classify_cluster(&cluster("[确认]"), &ctx).unwrap();
        // [=1 + 确=2 + 认=2 + ]=1 = 6
        assert_eq!(elem.width, 6);
    }

    #[test]
    fn element_width_ascii() {
        // ASCII characters should have width 1 each
        let ctx = ClassifyContext::new();
        let elem = classify_cluster(&cluster("[OK]"), &ctx).unwrap();
        // [=1 + O=1 + K=1 + ]=1 = 4
        assert_eq!(elem.width, 4);
    }

    #[test]
    fn element_width_mixed() {
        // Mixed ASCII and CJK
        let ctx = ClassifyContext::new();
        let elem = classify_cluster(&cluster("[OK确认]"), &ctx).unwrap();
        // [=1 + O=1 + K=1 + 确=2 + 认=2 + ]=1 = 8
        assert_eq!(elem.width, 8);
    }

    #[test]
    fn token_col_with_cjk_prefix_bracketed() {
        // CJK characters before a bracketed token should offset by display width, not char count
        let ctx = ClassifyContext::new();
        // 确(width=2) + 认(width=2) = 4 columns before [OK]
        let cluster = Cluster::new(0, 0, "确认[OK]".to_string(), CellStyle::default());
        let elements = extract_elements_from_cluster(&cluster, &ctx);
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].text, "[OK]");
        assert_eq!(elements[0].col, 4); // Not 2 (char count)!
    }

    #[test]
    fn token_col_with_cjk_prefix_underscore() {
        // CJK characters before an underscore run should offset by display width
        let ctx = ClassifyContext::new();
        // 名(width=2) + 前(width=2) + :(width=1) = 5 columns before ____
        let cluster = Cluster::new(0, 0, "名前:____".to_string(), CellStyle::default());
        let elements = extract_elements_from_cluster(&cluster, &ctx);
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].text, "____");
        assert_eq!(elements[0].col, 5); // Not 3 (char count)!
    }

    #[test]
    fn token_col_ascii_unchanged() {
        // ASCII text should still work correctly (char count == display width)
        let ctx = ClassifyContext::new();
        let cluster = Cluster::new(0, 5, "Save [OK] Cancel".to_string(), CellStyle::default());
        let elements = extract_elements_from_cluster(&cluster, &ctx);
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].text, "[OK]");
        assert_eq!(elements[0].col, 10); // 5 (cluster col) + 5 (offset of [OK])
    }
}
