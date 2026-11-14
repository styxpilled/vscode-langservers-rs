//! Port of `htmlFolding.ts`.

use lsp_types::{FoldingRange, FoldingRangeKind};
use text_document::TextDocument;

use crate::LanguageService;
use crate::language_facts::HtmlDataManager;
use crate::parser::scanner::{Scanner, TokenType};
use crate::utils::is_js_whitespace;

fn range(start_line: u32, end_line: u32, kind: Option<FoldingRangeKind>) -> FoldingRange {
    FoldingRange {
        start_line,
        end_line,
        kind,
        ..Default::default()
    }
}

/// `/^\s*#(end)?region\b/`: `Some(true)` for a region start, `Some(false)` for
/// an end.
fn region_marker(text: &str) -> Option<bool> {
    let rest = text
        .trim_start_matches(is_js_whitespace)
        .strip_prefix('#')?;
    let (start, rest) = match rest.strip_prefix("endregion") {
        Some(rest) => (false, rest),
        None => (true, rest.strip_prefix("region")?),
    };
    let word_continues = rest
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
    (!word_continues).then_some(start)
}

impl LanguageService {
    /// `getFoldingRanges`.
    pub fn get_folding_ranges(
        &self,
        document: &TextDocument,
        range_limit: Option<usize>,
    ) -> Vec<FoldingRange> {
        let text = document.text();
        let mut scanner = Scanner::at(text, 0);
        let mut token = scanner.scan();
        let mut ranges: Vec<FoldingRange> = Vec::new();
        let mut stack: Vec<(u32, &str)> = Vec::new();
        let mut last_tag_name: Option<&str> = None;
        let mut prev_start: i64 = -1;
        let line_of = |offset: usize| document.position_at(offset).line;
        let void_elements = self.data_manager.void_elements();

        let add = |ranges: &mut Vec<FoldingRange>, prev_start: &mut i64, r: FoldingRange| {
            *prev_start = i64::from(r.start_line);
            ranges.push(r);
        };

        while token != TokenType::Eos {
            let mut close = false;
            match token {
                TokenType::StartTag => {
                    let tag_name = scanner.token_text();
                    stack.push((line_of(scanner.token_offset()), tag_name));
                    last_tag_name = Some(tag_name);
                }
                TokenType::EndTag => last_tag_name = Some(scanner.token_text()),
                TokenType::StartTagClose => {
                    close = last_tag_name
                        .is_some_and(|t| HtmlDataManager::is_void_element(t, void_elements));
                }
                TokenType::EndTagClose | TokenType::StartTagSelfClose => close = true,
                TokenType::Comment => {
                    let mut start_line = line_of(scanner.token_offset());
                    match region_marker(scanner.token_text()) {
                        Some(true) => stack.push((start_line, "")),
                        Some(false) => {
                            if let Some(i) = stack.iter().rposition(|(_, t)| t.is_empty()) {
                                let (line, _) = stack[i];
                                stack.truncate(i);
                                let end_line = start_line;
                                start_line = line;
                                if end_line > start_line && prev_start != i64::from(start_line) {
                                    add(
                                        &mut ranges,
                                        &mut prev_start,
                                        range(start_line, end_line, Some(FoldingRangeKind::Region)),
                                    );
                                }
                            }
                        }
                        None => {
                            let end_line = line_of(scanner.token_end());
                            if start_line < end_line {
                                add(
                                    &mut ranges,
                                    &mut prev_start,
                                    range(start_line, end_line, Some(FoldingRangeKind::Comment)),
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
            if close && let Some(i) = stack.iter().rposition(|&(_, t)| Some(t) == last_tag_name) {
                let (start_line, _) = stack[i];
                stack.truncate(i);
                let line = line_of(scanner.token_offset());
                // `line - 1` can be -1 upstream; then it is not greater.
                if line > start_line + 1 && prev_start != i64::from(start_line) {
                    add(
                        &mut ranges,
                        &mut prev_start,
                        range(start_line, line - 1, None),
                    );
                }
            }
            token = scanner.scan();
        }

        let range_limit = range_limit.filter(|&n| n != 0).unwrap_or(usize::MAX);
        if ranges.len() > range_limit {
            return limit_ranges(ranges, range_limit);
        }
        ranges
    }
}

/// `limitRanges`.
fn limit_ranges(mut ranges: Vec<FoldingRange>, range_limit: usize) -> Vec<FoldingRange> {
    ranges.sort_by(|a, b| {
        a.start_line
            .cmp(&b.start_line)
            .then(a.end_line.cmp(&b.end_line))
    });
    let mut top: Option<usize> = None;
    let mut previous: Vec<usize> = Vec::new();
    let mut nesting_levels: Vec<Option<usize>> = vec![None; ranges.len()];
    let mut nesting_level_counts: Vec<usize> = Vec::new();
    let mut set_level = |levels: &mut Vec<Option<usize>>, i: usize, level: usize| {
        levels[i] = Some(level);
        if level < 30 {
            if nesting_level_counts.len() <= level {
                nesting_level_counts.resize(level + 1, 0);
            }
            nesting_level_counts[level] += 1;
        }
    };
    for i in 0..ranges.len() {
        let entry = &ranges[i];
        match top {
            None => {
                top = Some(i);
                set_level(&mut nesting_levels, i, 0);
            }
            Some(t) => {
                if entry.start_line > ranges[t].start_line {
                    if entry.end_line <= ranges[t].end_line {
                        previous.push(t);
                        top = Some(i);
                        set_level(&mut nesting_levels, i, previous.len());
                    } else if entry.start_line > ranges[t].end_line {
                        loop {
                            top = previous.pop();
                            match top {
                                Some(t) if entry.start_line > ranges[t].end_line => {}
                                _ => break,
                            }
                        }
                        if let Some(t) = top {
                            previous.push(t);
                        }
                        top = Some(i);
                        set_level(&mut nesting_levels, i, previous.len());
                    }
                }
            }
        }
    }
    let mut entries = 0;
    let mut max_level = 0;
    for (i, &n) in nesting_level_counts.iter().enumerate() {
        if n > 0 {
            if n + entries > range_limit {
                max_level = i;
                break;
            }
            entries += n;
        }
    }
    ranges
        .into_iter()
        .zip(nesting_levels)
        .filter_map(|(r, level)| {
            let level = level?;
            let keep = level < max_level
                || level == max_level && {
                    let k = entries < range_limit;
                    entries += 1;
                    k
                };
            keep.then_some(r)
        })
        .collect()
}
