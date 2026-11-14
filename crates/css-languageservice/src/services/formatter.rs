//! Port of `cssFormatter.ts`.

use lsp_types::{Position, Range, TextEdit};
use serde::Deserialize;
use text_document::TextDocument;

use crate::beautify::{BeautifyOptions, css_beautify};
use crate::utils::is_js_whitespace;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BraceStyle {
    Collapse,
    Expand,
}

/// `CSSFormatConfiguration`. The numbers are integers: upstream converts
/// them with `parseInt`. `wrapLineLength` has no effect upstream (see
/// `beautify.rs`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CssFormatConfiguration {
    pub tab_size: Option<u32>,
    pub insert_spaces: Option<bool>,
    pub insert_final_newline: Option<bool>,
    pub newline_between_selectors: Option<bool>,
    pub newline_between_rules: Option<bool>,
    pub space_around_selector_separator: Option<bool>,
    pub brace_style: Option<BraceStyle>,
    pub preserve_new_lines: Option<bool>,
    pub max_preserve_new_lines: Option<i64>,
    pub indent_empty_lines: Option<bool>,
}

/// `format`.
pub fn format(
    document: &TextDocument,
    range: Option<Range>,
    options: &CssFormatConfiguration,
) -> Vec<TextEdit> {
    let text = document.text();
    let b = text.as_bytes();
    let mut value = text.to_owned();
    let mut includes_end = true;
    let mut initial_indent_level = 0;
    let mut in_rule = false;
    let tab_size = match options.tab_size {
        None | Some(0) => 4,
        Some(n) => n,
    };
    let range = match range {
        Some(range) => {
            let mut start = document.offset_at(range.start);
            // Include all leading whitespace if at the start of the line,
            // else keep at least one whitespace.
            let mut extended_start = start;
            while extended_start > 0 && matches!(b[extended_start - 1], b' ' | b'\t') {
                extended_start -= 1;
            }
            if extended_start == 0 || matches!(b[extended_start - 1], b'\r' | b'\n') {
                start = extended_start;
            } else if extended_start < start {
                start = extended_start + 1;
            }
            // Include all following whitespace up to the end of the line.
            let mut end = document.offset_at(range.end);
            let mut extended_end = end;
            while extended_end < b.len() && matches!(b[extended_end], b' ' | b'\t') {
                extended_end += 1;
            }
            if extended_end == b.len() || matches!(b[extended_end], b'\r' | b'\n') {
                end = extended_end;
            }
            let range = Range::new(document.position_at(start), document.position_at(end));
            in_rule = is_in_rule(b, start);
            includes_end = end == b.len();
            value = text[start.min(end)..start.max(end)].to_owned();
            if start != 0 {
                let start_of_line = document.offset_at(Position::new(range.start.line, 0));
                initial_indent_level = compute_indent_level(text, start_of_line, tab_size);
            }
            if in_rule {
                value = format!("{{\n{}", trim_left(&value));
            }
            range
        }
        None => Range::new(Position::new(0, 0), document.position_at(text.len())),
    };

    let insert_spaces = options.insert_spaces.unwrap_or(false);
    let preserve_newlines = options.preserve_new_lines.unwrap_or(true);
    let beautify_options = BeautifyOptions {
        indent_size: tab_size.into(),
        indent_with_tabs: !insert_spaces,
        end_with_newline: includes_end && options.insert_final_newline.unwrap_or(false),
        selector_separator_newline: options.newline_between_selectors.unwrap_or(true),
        newline_between_rules: options.newline_between_rules.unwrap_or(true),
        space_around_combinator: options.space_around_selector_separator.unwrap_or(false),
        brace_style_expand: options.brace_style == Some(BraceStyle::Expand),
        indent_empty_lines: options.indent_empty_lines.unwrap_or(false),
        max_preserve_newlines: if preserve_newlines {
            options.max_preserve_new_lines.unwrap_or(32786)
        } else {
            0
        },
        loose_directives: false,
    };

    let mut result = css_beautify(&value, &beautify_options);
    if in_rule {
        let mut units = 0;
        let skipped = result
            .find(|c: char| {
                let skip = units < 2;
                units += c.len_utf16();
                !skip
            })
            .unwrap_or(result.len());
        result = trim_left(&result[skipped..]).to_owned();
    }
    if initial_indent_level > 0 {
        let indent = if insert_spaces {
            " ".repeat((tab_size * initial_indent_level) as usize)
        } else {
            "\t".repeat(initial_indent_level as usize)
        };
        result = result.replace('\n', &format!("\n{indent}"));
        if range.start.character == 0 {
            result.insert_str(0, &indent);
        }
    }
    vec![TextEdit::new(range, result)]
}

fn trim_left(s: &str) -> &str {
    s.trim_start_matches(is_js_whitespace)
}

/// `isInRule`: the first brace at or before `offset` is `{`.
fn is_in_rule(b: &[u8], offset: usize) -> bool {
    b[..b.len().min(offset + 1)]
        .iter()
        .rev()
        .find(|&&c| c == b'{' || c == b'}')
        == Some(&b'{')
}

fn compute_indent_level(content: &str, offset: usize, tab_size: u32) -> u32 {
    let mut n = 0;
    for c in content[offset..].bytes() {
        match c {
            b' ' => n += 1,
            b'\t' => n += tab_size,
            _ => break,
        }
    }
    n / tab_size
}
