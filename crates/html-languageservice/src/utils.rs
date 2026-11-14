//! JS behavior that the services need.

pub use css_languageservice::utils::{is_js_whitespace, utf16_len};

/// `/^\s*$/.test(ch)` for one character, or `''` (true).
pub fn is_whitespace_or_empty(c: Option<char>) -> bool {
    c.is_none_or(is_js_whitespace)
}

/// The character before `offset`.
pub fn char_before(text: &str, offset: usize) -> Option<char> {
    text[..offset].chars().next_back()
}

/// The character at `offset`.
pub fn char_at(text: &str, offset: usize) -> Option<char> {
    text.get(offset..).and_then(|s| s.chars().next())
}

/// `isLetterOrDigit(text, index)`.
pub fn is_letter_or_digit(text: &str, index: usize) -> bool {
    text.as_bytes()
        .get(index)
        .is_some_and(u8::is_ascii_alphanumeric)
}

/// `String.prototype.substring`: swaps the bounds if `start > end`.
pub fn substring(text: &str, start: usize, end: usize) -> &str {
    &text[start.min(end)..start.max(end)]
}
