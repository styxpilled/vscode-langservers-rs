use jsonc::{ScanError, Scanner, SyntaxKind};
use lsp_types::{FoldingRange, FoldingRangeKind, Position};

use crate::TextDocument;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Object,
    Array,
    Comment,
    Region,
}

struct Range {
    start_line: usize,
    end_line: usize,
    kind: Kind,
}

/// Upstream gives object and array ranges the kinds `object` and `array`. LSP does not define
/// these kinds, so these ranges have no kind.
pub fn get_folding_ranges(
    document: &TextDocument,
    range_limit: Option<usize>,
) -> Vec<FoldingRange> {
    let mut ranges: Vec<Range> = Vec::new();
    let mut nesting_levels: Vec<usize> = Vec::new();
    let mut stack: Vec<Range> = Vec::new();
    let mut prev_start = None;
    let text = document.text();
    let mut scanner = Scanner::new(text, false);
    let line_of = |offset: usize| document.position_at(offset).line as usize;

    let mut add_range = |range: Range, stack_len: usize| {
        ranges.push(range);
        nesting_levels.push(stack_len);
    };

    let mut token = scanner.scan();
    while token != SyntaxKind::EOF {
        match token {
            SyntaxKind::OpenBraceToken | SyntaxKind::OpenBracketToken => {
                let start_line = line_of(scanner.token_offset());
                let kind = if token == SyntaxKind::OpenBraceToken {
                    Kind::Object
                } else {
                    Kind::Array
                };
                stack.push(Range {
                    start_line,
                    end_line: start_line,
                    kind,
                });
            }
            SyntaxKind::CloseBraceToken | SyntaxKind::CloseBracketToken => {
                let kind = if token == SyntaxKind::CloseBraceToken {
                    Kind::Object
                } else {
                    Kind::Array
                };
                if let Some(mut range) = stack.pop_if(|r| r.kind == kind) {
                    let line = line_of(scanner.token_offset());
                    if line > range.start_line + 1 && prev_start != Some(range.start_line) {
                        range.end_line = line - 1;
                        prev_start = Some(range.start_line);
                        add_range(range, stack.len());
                    }
                }
            }
            SyntaxKind::BlockCommentTrivia => {
                let start_line = line_of(scanner.token_offset());
                let end_line = line_of(scanner.token_offset() + scanner.token_length());
                if scanner.token_error() == ScanError::UnexpectedEndOfComment
                    && start_line + 1 < document.line_count()
                {
                    let next_line = Position::new(start_line as u32 + 1, 0);
                    scanner.set_position(document.offset_at(next_line));
                } else if start_line < end_line {
                    add_range(
                        Range {
                            start_line,
                            end_line,
                            kind: Kind::Comment,
                        },
                        stack.len(),
                    );
                    prev_start = Some(start_line);
                }
            }
            SyntaxKind::LineCommentTrivia => {
                let start = scanner.token_offset();
                let comment = &text[start..start + scanner.token_length()];
                match region_marker(comment) {
                    Some(true) => {
                        let line = line_of(start);
                        stack.push(Range {
                            start_line: line,
                            end_line: line,
                            kind: Kind::Region,
                        });
                    }
                    Some(false) => {
                        if let Some(i) = stack.iter().rposition(|r| r.kind == Kind::Region) {
                            let line = line_of(start);
                            let mut range = stack.split_off(i).swap_remove(0);
                            if line > range.start_line && prev_start != Some(range.start_line) {
                                range.end_line = line;
                                prev_start = Some(range.start_line);
                                add_range(range, stack.len());
                            }
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
        token = scanner.scan();
    }

    let to_lsp = |range: Range| FoldingRange {
        start_line: range.start_line as u32,
        end_line: range.end_line as u32,
        kind: match range.kind {
            Kind::Comment => Some(FoldingRangeKind::Comment),
            Kind::Region => Some(FoldingRangeKind::Region),
            Kind::Object | Kind::Array => None,
        },
        ..Default::default()
    };

    let Some(range_limit) = range_limit.filter(|&limit| ranges.len() > limit) else {
        return ranges.into_iter().map(to_lsp).collect();
    };

    let mut counts = [0usize; 30];
    for &level in &nesting_levels {
        if level < counts.len() {
            counts[level] += 1;
        }
    }
    let mut entries = 0;
    let mut max_level = 0;
    for (level, &n) in counts.iter().enumerate() {
        if n > 0 {
            if n + entries > range_limit {
                max_level = level;
                break;
            }
            entries += n;
        }
    }
    ranges
        .into_iter()
        .zip(nesting_levels)
        .filter(|&(_, level)| {
            let keep = level < max_level || (level == max_level && entries < range_limit);
            if level == max_level {
                entries += 1;
            }
            keep
        })
        .map(|(range, _)| to_lsp(range))
        .collect()
}

/// `/^\/\/\s*#(end)?region\b/`: `Some(true)` for a region start, `Some(false)`
/// for a region end.
fn region_marker(comment: &str) -> Option<bool> {
    let rest = comment.strip_prefix("//")?.trim_start().strip_prefix('#')?;
    let (start, rest) = match rest.strip_prefix("endregion") {
        Some(rest) => (false, rest),
        None => (true, rest.strip_prefix("region")?),
    };
    (!rest.starts_with(is_word_char)).then_some(start)
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::region_marker;

    #[test]
    fn endregion_only_at_start() {
        assert_eq!(region_marker("// #region a"), Some(true));
        assert_eq!(region_marker("//  #endregion"), Some(false));
        assert_eq!(region_marker("// not endregion"), None);
        assert_eq!(region_marker("// #regions"), None);
    }
}
