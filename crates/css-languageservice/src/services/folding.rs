//! Port of `cssFolding.ts`.

use lsp_types::{FoldingRange, FoldingRangeKind};
use text_document::TextDocument;

use crate::parser::Dialect;
use crate::parser::scanner::{Scanner, Token, TokenType};
use crate::utils::is_js_whitespace;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DelimiterType {
    Brace,
    Comment,
}

#[derive(Clone, Copy)]
struct Delimiter {
    line: u32,
    ty: DelimiterType,
    is_start: bool,
}

/// `getFoldingRanges`.
pub fn get_folding_ranges(
    document: &TextDocument,
    range_limit: Option<usize>,
) -> Vec<FoldingRange> {
    limit_folding_ranges(compute_folding_ranges(document), range_limit)
}

fn compute_folding_ranges(document: &TextDocument) -> Vec<FoldingRange> {
    let start_line = |t: &Token| document.position_at(t.offset).line;
    let end_line = |t: &Token| document.position_at(t.end()).line;
    let language_id = document.language_id();
    let dialect = match language_id {
        "scss" => Dialect::Scss,
        "less" => Dialect::Less,
        _ => Dialect::Css,
    };

    let mut ranges = Vec::new();
    let mut stack: Vec<Delimiter> = Vec::new();
    let mut scanner = Scanner::new(dialect);
    scanner.ignore_comment = false;
    scanner.set_source(document.text());

    // The end of the previous token. Its line is only read at a `}`.
    let mut prev_end: Option<usize> = None;
    loop {
        let token = scanner.scan();
        match token.kind {
            TokenType::Eof => break,
            TokenType::CurlyL | TokenType::InterpolationFunction => stack.push(Delimiter {
                line: start_line(&token),
                ty: DelimiterType::Brace,
                is_start: true,
            }),
            TokenType::CurlyR => {
                if let Some(prev) = pop_prev_start_delimiter(&mut stack, DelimiterType::Brace) {
                    let mut end = end_line(&token);
                    // Keep the closing brace visible, except after `color: red; }`.
                    if prev_end.is_some_and(|o| document.position_at(o).line != end) {
                        end -= 1;
                    }
                    if prev.line != end {
                        ranges.push(folding_range(prev.line, end, None));
                    }
                }
            }
            TokenType::Comment => {
                let marker = region_marker(&scanner.text(&token), dialect != Dialect::Css);
                match marker {
                    Some(true) => stack.push(Delimiter {
                        line: start_line(&token),
                        ty: DelimiterType::Comment,
                        is_start: true,
                    }),
                    Some(false) => {
                        let line = end_line(&token);
                        if let Some(prev) =
                            pop_prev_start_delimiter(&mut stack, DelimiterType::Comment)
                            && prev.line != line
                        {
                            ranges.push(folding_range(
                                prev.line,
                                line,
                                Some(FoldingRangeKind::Region),
                            ));
                        }
                    }
                    None => {
                        let (start, end) = (start_line(&token), end_line(&token));
                        if start != end {
                            ranges.push(folding_range(start, end, Some(FoldingRangeKind::Comment)));
                        }
                    }
                }
            }
            _ => {}
        }
        prev_end = Some(token.end());
    }
    ranges
}

fn folding_range(start_line: u32, end_line: u32, kind: Option<FoldingRangeKind>) -> FoldingRange {
    FoldingRange {
        start_line,
        end_line,
        kind,
        ..Default::default()
    }
}

/// `Some(true)` for `#region`, `Some(false)` for `#endregion`. Upstream
/// regular expressions:
/// `^\s*\/\*\s*(#region|#endregion)\b\s*(.*?)\s*\*\/` and, for SCSS and Less,
/// `^\s*\/\/\s*(#region|#endregion)\b\s*(.*?)\s*`.
fn region_marker(text: &str, line_comments: bool) -> Option<bool> {
    let keyword = |rest: &str| -> Option<(bool, usize)> {
        let rest_trimmed = rest.trim_start_matches(is_js_whitespace);
        let skipped = rest.len() - rest_trimmed.len();
        let (is_start, len) = if rest_trimmed.starts_with("#region") {
            (true, "#region".len())
        } else if rest_trimmed.starts_with("#endregion") {
            (false, "#endregion".len())
        } else {
            return None;
        };
        let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
        if rest_trimmed.as_bytes().get(len).is_some_and(|&c| word(c)) {
            return None;
        }
        Some((is_start, skipped + len))
    };
    let text = text.trim_start_matches(is_js_whitespace);
    if let Some(rest) = text.strip_prefix("/*")
        && let Some((is_start, len)) = keyword(rest)
    {
        // `\s*(.*?)\s*\*\/`: the text up to a `*/` has no line terminator
        // between its leading and trailing white space.
        let rest = &rest[len..];
        let found = rest.match_indices("*/").any(|(i, _)| {
            !rest[..i]
                .trim_matches(is_js_whitespace)
                .contains(['\n', '\r', '\u{2028}', '\u{2029}'])
        });
        if found {
            return Some(is_start);
        }
    }
    if line_comments && let Some(rest) = text.strip_prefix("//") {
        return keyword(rest).map(|(is_start, _)| is_start);
    }
    None
}

fn pop_prev_start_delimiter(stack: &mut Vec<Delimiter>, ty: DelimiterType) -> Option<Delimiter> {
    let i = stack.iter().rposition(|d| d.ty == ty && d.is_start)?;
    Some(stack.remove(i))
}

/// `limitFoldingRanges`: sorts, removes intersections and keeps at most
/// `range_limit` ranges.
fn limit_folding_ranges(
    mut ranges: Vec<FoldingRange>,
    range_limit: Option<usize>,
) -> Vec<FoldingRange> {
    ranges.sort_by_key(|r| (r.start_line, r.end_line));
    let mut valid = Vec::new();
    let mut prev_end_line: i64 = -1;
    for r in ranges {
        let (start, end) = (i64::from(r.start_line), i64::from(r.end_line));
        if !(start < prev_end_line && prev_end_line < end) {
            prev_end_line = end;
            valid.push(r);
        }
    }
    if let Some(limit) = range_limit.filter(|&l| l > 0) {
        valid.truncate(limit);
    }
    valid
}
