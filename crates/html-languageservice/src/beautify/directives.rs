//! `Directives` of js-beautify: `beautify ignore:start` and `ignore:end`.

use css_languageservice::utils::is_js_whitespace;

fn whitespace_len(text: &str, i: usize) -> usize {
    text[i..].len() - text[i..].trim_start_matches(is_js_whitespace).len()
}

/// `Directives.__directives_block_pattern`:
/// `start\s*beautify(\s+\w+[:]\w+)+\s*end`.
fn has_directives_block(text: &str, start: &str, end: &str) -> bool {
    let b = text.as_bytes();
    text.match_indices(start).any(|(i, _)| {
        let mut j = i + start.len();
        let w = whitespace_len(text, j);
        if !text[j + w..].starts_with("beautify") {
            return false;
        }
        j += w + "beautify".len();
        let mut groups = 0;
        loop {
            let w = whitespace_len(text, j);
            if groups > 0 && text[j + w..].starts_with(end) {
                return true;
            }
            if w == 0 {
                return false;
            }
            let name = j + w;
            let name_end = word_end(b, name);
            if name_end == name || b.get(name_end) != Some(&b':') {
                return false;
            }
            let value_end = word_end(b, name_end + 1);
            if value_end == name_end + 1 {
                return false;
            }
            j = value_end;
            groups += 1;
        }
    })
}

fn word_end(b: &[u8], mut i: usize) -> usize {
    while b
        .get(i)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
    {
        i += 1;
    }
    i
}

/// `get_directives(text).ignore === 'start'`. The directives are all the
/// matches of `/ (\w+)[:](\w+)/g` in the text; the last one wins.
pub fn is_ignore_start(text: &str, start: &str, end: &str) -> bool {
    if !has_directives_block(text, start, end) {
        return false;
    }
    let b = text.as_bytes();
    let mut ignore = None;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b' ' {
            let name_end = word_end(b, i + 1);
            if name_end > i + 1 && b.get(name_end) == Some(&b':') {
                let value_end = word_end(b, name_end + 1);
                if value_end > name_end + 1 {
                    if &b[i + 1..name_end] == b"ignore" {
                        ignore = Some(&b[name_end + 1..value_end]);
                    }
                    i = value_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    ignore == Some(b"start")
}

/// The end of the first match of `start\sbeautify\signore:end\send` in
/// `text`.
pub fn find_ignore_end(text: &str, start: &str, end: &str) -> Option<usize> {
    text.match_indices(start).find_map(|(i, _)| {
        let mut rest = &text[i + start.len()..];
        for part in ["beautify", "ignore:end", end] {
            let mut chars = rest.chars();
            if !chars.next().is_some_and(is_js_whitespace) {
                return None;
            }
            rest = chars.as_str().strip_prefix(part)?;
        }
        Some(text.len() - rest.len())
    })
}
