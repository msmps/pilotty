//! Input encoding for terminal commands.
//!
//! Handles conversion of text and key names to bytes for PTY input.

use crate::protocol::ScrollDirection;

/// Encode text for PTY input, handling escape sequences.
///
/// Converts common escape sequences in the input string:
/// - `\n` -> newline (0x0A)
/// - `\r` -> carriage return (0x0D)
/// - `\t` -> tab (0x09)
/// - `\\` -> backslash
/// - Other text is passed through as UTF-8
pub fn encode_text(text: &str) -> Vec<u8> {
    let mut result = Vec::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            // Check for escape sequence
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    result.push(b'\n');
                }
                Some('r') => {
                    chars.next();
                    result.push(b'\r');
                }
                Some('t') => {
                    chars.next();
                    result.push(b'\t');
                }
                Some('\\') => {
                    chars.next();
                    result.push(b'\\');
                }
                Some('x') => {
                    // Hex escape: \xNN
                    chars.next(); // consume 'x'
                    let hex: String = chars.by_ref().take(2).collect();
                    if hex.len() == 2 {
                        if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                            result.push(byte);
                            continue;
                        }
                    }
                    // Invalid hex escape, output as-is
                    result.extend_from_slice(b"\\x");
                    result.extend_from_slice(hex.as_bytes());
                }
                _ => {
                    // Not a recognized escape, output backslash literally
                    result.push(b'\\');
                }
            }
        } else {
            // Regular character - encode as UTF-8
            let mut buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut buf);
            result.extend_from_slice(encoded.as_bytes());
        }
    }

    result
}

/// Named keys and their byte sequences.
///
/// Returns the escape sequence for a named key, or None if not recognized.
pub fn key_to_bytes(key: &str) -> Option<Vec<u8>> {
    // Normalize key name (case insensitive)
    let key_lower = key.to_lowercase();
    let key_str = key_lower.as_str();

    let bytes: &[u8] = match key_str {
        // Basic keys
        "enter" | "return" => b"\r",
        "tab" => b"\t",
        "escape" | "esc" => b"\x1b",
        "backspace" => b"\x7f",
        "delete" | "del" => b"\x1b[3~",
        "space" => b" ",
        "plus" => b"+", // Named alias for literal + (useful since + is combo separator)

        // Arrow keys (standard ANSI)
        "up" | "arrowup" => b"\x1b[A",
        "down" | "arrowdown" => b"\x1b[B",
        "right" | "arrowright" => b"\x1b[C",
        "left" | "arrowleft" => b"\x1b[D",

        // Navigation keys
        "home" => b"\x1b[H",
        "end" => b"\x1b[F",
        "pageup" | "pgup" => b"\x1b[5~",
        "pagedown" | "pgdn" => b"\x1b[6~",
        "insert" | "ins" => b"\x1b[2~",

        // Function keys (F1-F12)
        "f1" => b"\x1bOP",
        "f2" => b"\x1bOQ",
        "f3" => b"\x1bOR",
        "f4" => b"\x1bOS",
        "f5" => b"\x1b[15~",
        "f6" => b"\x1b[17~",
        "f7" => b"\x1b[18~",
        "f8" => b"\x1b[19~",
        "f9" => b"\x1b[20~",
        "f10" => b"\x1b[21~",
        "f11" => b"\x1b[23~",
        "f12" => b"\x1b[24~",

        _ => return None,
    };

    Some(bytes.to_vec())
}

/// Parse a key combo like "Ctrl+C" or "Alt+F" and return the bytes.
///
/// Supports:
/// - Ctrl+<key>: Control character (Ctrl+A = 0x01, Ctrl+C = 0x03, etc.)
/// - Alt+<key>: Escape prefix + key (Alt+F = ESC f)
/// - Shift+<key>: Uppercase for letters, otherwise ignored
/// - Combinations: Ctrl+Alt+<key>, etc.
pub fn parse_key_combo(combo: &str) -> Option<Vec<u8>> {
    let parts: Vec<&str> = combo.split('+').collect();

    if parts.is_empty() {
        return None;
    }

    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key_part = "";

    for part in &parts {
        let lower = part.to_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" | "meta" | "option" => alt = true,
            "shift" => shift = true,
            _ => key_part = part,
        }
    }

    if key_part.is_empty() {
        return None;
    }

    // Handle Ctrl+Space specially (produces NUL)
    if ctrl && key_part.to_lowercase() == "space" {
        let mut result = Vec::new();
        if alt {
            result.push(0x1b);
        }
        result.push(0x00);
        return Some(result);
    }

    // Try as named key first
    if let Some(bytes) = key_to_bytes(key_part) {
        // For named keys, modifiers are typically not applied (except Alt prefix)
        if alt {
            let mut result = vec![0x1b];
            result.extend(bytes);
            return Some(result);
        }
        return Some(bytes);
    }

    // Single character
    let chars: Vec<char> = key_part.chars().collect();
    if chars.len() != 1 {
        return None;
    }

    let mut c = chars[0];

    // Apply shift (uppercase for letters)
    if shift && c.is_ascii_lowercase() {
        c = c.to_ascii_uppercase();
    }

    // Apply Ctrl (control characters)
    if ctrl {
        let ctrl_char = if c.is_ascii_alphabetic() {
            // Ctrl+A = 0x01, Ctrl+B = 0x02, ..., Ctrl+Z = 0x1A
            let base = c.to_ascii_uppercase() as u8;
            base - b'A' + 1
        } else {
            // Some special Ctrl combos
            match c {
                '[' | '3' => 0x1b, // Ctrl+[ = Escape
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '7' => 0x1f,
                '@' | '2' | ' ' => 0x00, // Ctrl+Space = NUL
                '?' => 0x7f,             // Ctrl+? = DEL
                _ => return None,
            }
        };

        let mut result = Vec::new();
        if alt {
            result.push(0x1b);
        }
        result.push(ctrl_char);
        return Some(result);
    }

    // Apply Alt (escape prefix)
    if alt {
        let mut result = vec![0x1b];
        result.extend(c.to_string().as_bytes());
        return Some(result);
    }

    // Just a regular key
    Some(c.to_string().into_bytes())
}

/// Generate mouse click escape sequences (SGR extended encoding).
///
/// SGR mouse encoding is: `\x1b[<button;x;yM` for press, `\x1b[<button;x;ym` for release
/// Coordinates are 1-indexed.
///
/// Returns (press_sequence, release_sequence).
pub fn encode_mouse_click(x: u16, y: u16) -> (Vec<u8>, Vec<u8>) {
    // Convert to 1-indexed coordinates
    let x1 = x.saturating_add(1);
    let y1 = y.saturating_add(1);

    // Button 0 = left click
    let press = format!("\x1b[<0;{};{}M", x1, y1);
    let release = format!("\x1b[<0;{};{}m", x1, y1);

    (press.into_bytes(), release.into_bytes())
}

/// Generate a complete mouse click (press + release) as a single sequence.
pub fn encode_mouse_click_combined(x: u16, y: u16) -> Vec<u8> {
    let (press, release) = encode_mouse_click(x, y);
    let mut result = press;
    result.extend(release);
    result
}

/// Generate scroll wheel sequences.
///
/// Scroll up = button 64 (0x40), scroll down = button 65 (0x41)
/// Using SGR encoding: `\x1b[<button;x;yM`
pub fn encode_scroll(direction: ScrollDirection, x: u16, y: u16) -> Vec<u8> {
    let x1 = x.saturating_add(1);
    let y1 = y.saturating_add(1);

    let button = match direction {
        ScrollDirection::Up => 64,
        ScrollDirection::Down => 65,
    };

    format!("\x1b[<{};{};{}M", button, x1, y1).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_text_plain() {
        assert_eq!(encode_text("hello"), b"hello");
        assert_eq!(encode_text("Hello World"), b"Hello World");
    }

    #[test]
    fn test_encode_text_newline() {
        assert_eq!(encode_text("line1\\nline2"), b"line1\nline2");
    }

    #[test]
    fn test_encode_text_tab() {
        assert_eq!(encode_text("col1\\tcol2"), b"col1\tcol2");
    }

    #[test]
    fn test_encode_text_carriage_return() {
        assert_eq!(encode_text("text\\r"), b"text\r");
    }

    #[test]
    fn test_encode_text_backslash() {
        assert_eq!(encode_text("path\\\\file"), b"path\\file");
    }

    #[test]
    fn test_encode_text_hex_escape() {
        assert_eq!(encode_text("\\x1b"), vec![0x1b]);
        assert_eq!(encode_text("\\x00\\xff"), vec![0x00, 0xff]);
    }

    #[test]
    fn test_encode_text_unicode() {
        let result = encode_text("hello 世界");
        assert_eq!(result, "hello 世界".as_bytes());
    }

    #[test]
    fn test_key_to_bytes_enter() {
        assert_eq!(key_to_bytes("Enter"), Some(b"\r".to_vec()));
        assert_eq!(key_to_bytes("ENTER"), Some(b"\r".to_vec()));
        assert_eq!(key_to_bytes("enter"), Some(b"\r".to_vec()));
    }

    #[test]
    fn test_key_to_bytes_escape() {
        assert_eq!(key_to_bytes("Escape"), Some(vec![0x1b]));
        assert_eq!(key_to_bytes("Esc"), Some(vec![0x1b]));
    }

    #[test]
    fn test_key_to_bytes_arrows() {
        assert_eq!(key_to_bytes("Up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_to_bytes("Down"), Some(b"\x1b[B".to_vec()));
        assert_eq!(key_to_bytes("Right"), Some(b"\x1b[C".to_vec()));
        assert_eq!(key_to_bytes("Left"), Some(b"\x1b[D".to_vec()));
    }

    #[test]
    fn test_key_to_bytes_function_keys() {
        assert_eq!(key_to_bytes("F1"), Some(b"\x1bOP".to_vec()));
        assert_eq!(key_to_bytes("F5"), Some(b"\x1b[15~".to_vec()));
        assert_eq!(key_to_bytes("F12"), Some(b"\x1b[24~".to_vec()));
    }

    #[test]
    fn test_key_to_bytes_unknown() {
        assert_eq!(key_to_bytes("NotAKey"), None);
    }

    #[test]
    fn test_key_to_bytes_plus() {
        // "plus" is a named alias for the literal + character
        assert_eq!(key_to_bytes("plus"), Some(b"+".to_vec()));
        assert_eq!(key_to_bytes("Plus"), Some(b"+".to_vec()));
        assert_eq!(key_to_bytes("PLUS"), Some(b"+".to_vec()));
    }

    #[test]
    fn test_parse_key_combo_ctrl_c() {
        assert_eq!(parse_key_combo("Ctrl+C"), Some(vec![0x03]));
        assert_eq!(parse_key_combo("ctrl+c"), Some(vec![0x03]));
    }

    #[test]
    fn test_parse_key_combo_ctrl_letters() {
        assert_eq!(parse_key_combo("Ctrl+A"), Some(vec![0x01]));
        assert_eq!(parse_key_combo("Ctrl+Z"), Some(vec![0x1a]));
        assert_eq!(parse_key_combo("Ctrl+S"), Some(vec![0x13])); // XOFF
        assert_eq!(parse_key_combo("Ctrl+Q"), Some(vec![0x11])); // XON
    }

    #[test]
    fn test_parse_key_combo_alt_letter() {
        // Alt+F should be ESC followed by 'f'
        assert_eq!(parse_key_combo("Alt+f"), Some(vec![0x1b, b'f']));
        assert_eq!(parse_key_combo("Alt+F"), Some(vec![0x1b, b'F']));
    }

    #[test]
    fn test_parse_key_combo_ctrl_alt() {
        // Ctrl+Alt+C = ESC followed by Ctrl+C
        assert_eq!(parse_key_combo("Ctrl+Alt+C"), Some(vec![0x1b, 0x03]));
    }

    #[test]
    fn test_parse_key_combo_named_key() {
        assert_eq!(parse_key_combo("Enter"), Some(b"\r".to_vec()));
        assert_eq!(parse_key_combo("Tab"), Some(b"\t".to_vec()));
    }

    #[test]
    fn test_parse_key_combo_alt_named_key() {
        // Alt+Enter = ESC followed by CR
        let result = parse_key_combo("Alt+Enter");
        assert_eq!(result, Some(vec![0x1b, b'\r']));
    }

    #[test]
    fn test_parse_key_combo_shift() {
        // Shift+a = A
        assert_eq!(parse_key_combo("Shift+a"), Some(b"A".to_vec()));
    }

    #[test]
    fn test_parse_key_combo_ctrl_special() {
        assert_eq!(parse_key_combo("Ctrl+["), Some(vec![0x1b])); // Escape
        assert_eq!(parse_key_combo("Ctrl+Space"), Some(vec![0x00])); // NUL
    }

    #[test]
    fn test_encode_mouse_click() {
        // Click at (0, 0) should produce 1-indexed coordinates (1, 1)
        let (press, release) = encode_mouse_click(0, 0);
        assert_eq!(press, b"\x1b[<0;1;1M");
        assert_eq!(release, b"\x1b[<0;1;1m");
    }

    #[test]
    fn test_encode_mouse_click_position() {
        // Click at (10, 5) should produce (11, 6)
        let (press, release) = encode_mouse_click(10, 5);
        assert_eq!(press, b"\x1b[<0;11;6M");
        assert_eq!(release, b"\x1b[<0;11;6m");
    }

    #[test]
    fn test_encode_mouse_click_combined() {
        let combined = encode_mouse_click_combined(5, 3);
        // Should contain both press and release
        assert!(combined.starts_with(b"\x1b[<0;6;4M"));
        assert!(combined.ends_with(b"\x1b[<0;6;4m"));
    }

    #[test]
    fn test_encode_scroll_up() {
        let scroll = encode_scroll(ScrollDirection::Up, 10, 5);
        // Button 64 for scroll up
        assert_eq!(scroll, b"\x1b[<64;11;6M");
    }

    #[test]
    fn test_encode_scroll_down() {
        let scroll = encode_scroll(ScrollDirection::Down, 10, 5);
        // Button 65 for scroll down
        assert_eq!(scroll, b"\x1b[<65;11;6M");
    }
}
