//! Port of `htmlSelectionRange.ts`.

use lsp_types::{Position, Range, SelectionRange};
use text_document::TextDocument;

use crate::LanguageService;
use crate::parser::scanner::{Scanner, TokenType};
use crate::parser::{HtmlDocument, Node};

impl LanguageService {
    /// `getSelectionRanges`.
    pub fn get_selection_ranges(
        &self,
        document: &TextDocument,
        positions: &[Position],
    ) -> Vec<SelectionRange> {
        let html_document = self.parse_html_document(document);
        positions
            .iter()
            .map(|&p| selection_range(p, document, &html_document))
            .collect()
    }
}

/// The selection range at one position of a parsed document.
pub fn selection_range(
    position: Position,
    document: &TextDocument,
    doc: &HtmlDocument,
) -> SelectionRange {
    let ranges = applicable_ranges(document, position, doc);
    let mut prev: Option<(usize, usize)> = None;
    let mut current: Option<SelectionRange> = None;
    for &range in ranges.iter().rev() {
        if prev != Some(range) {
            current = Some(SelectionRange {
                range: document.range(range.0, range.1),
                parent: current.map(Box::new),
            });
        }
        prev = Some(range);
    }
    current.unwrap_or(SelectionRange {
        range: Range::new(position, position),
        parent: None,
    })
}

fn applicable_ranges(
    document: &TextDocument,
    position: Position,
    doc: &HtmlDocument,
) -> Vec<(usize, usize)> {
    let offset = document.offset_at(position);
    let id = doc.find_node_at(offset);
    let node = &doc[id];
    let mut result = parent_tag_ranges(doc, id);

    // Self-closing or void elements.
    if let Some(start_tag_end) = node.start_tag_end_truthy()
        && node.end_tag_start_truthy().is_none()
    {
        // Unmatched tag pairs like <div></div1>.
        if start_tag_end != node.end {
            return vec![(node.start, node.end)];
        }
        let self_closing = start_tag_end >= 2
            && &document.text().as_bytes()[start_tag_end - 2..start_tag_end] == b"/>";
        let inner_end = if self_closing {
            start_tag_end - 2
        } else {
            start_tag_end - 1
        };
        result.insert(0, (node.start + 1, inner_end));
        let mut attribute_ranges = attribute_level_ranges(document.text(), node, offset);
        attribute_ranges.extend(result);
        return attribute_ranges;
    }

    let (Some(start_tag_end), Some(end_tag_start)) =
        (node.start_tag_end_truthy(), node.end_tag_start_truthy())
    else {
        return result;
    };

    // `<div class="foo">bar</div>`
    result.insert(0, (node.start, node.end));
    if node.start < offset && offset < start_tag_end {
        // Cursor inside `<div class="foo">`.
        result.insert(0, (node.start + 1, start_tag_end - 1));
        let mut attribute_ranges = attribute_level_ranges(document.text(), node, offset);
        attribute_ranges.extend(result);
        attribute_ranges
    } else if start_tag_end <= offset && offset <= end_tag_start {
        // Cursor inside `bar`.
        result.insert(0, (start_tag_end, end_tag_start));
        result
    } else {
        // `div` inside `</div>`.
        if offset >= end_tag_start + 2 {
            result.insert(0, (end_tag_start + 2, node.end - 1));
        }
        result
    }
}

fn parent_tag_ranges(doc: &HtmlDocument, mut id: usize) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    while let Some(parent) = doc[id].parent {
        id = parent;
        let n = &doc[id];
        match (n.start_tag_end_truthy(), n.end_tag_start_truthy()) {
            (Some(s), Some(e)) if s < e => {
                result.push((s, e));
                result.push((n.start, n.end));
            }
            _ => result.push((n.start, n.end)),
        }
    }
    result
}

/// `getAttributeLevelRanges`. The scanner reads only the text of the node.
fn attribute_level_ranges(text: &str, node: &Node, offset: usize) -> Vec<(usize, usize)> {
    let node_text = &text[node.start..node.end];
    let relative_offset = offset - node.start;
    let mut scanner = Scanner::at(node_text, 0);
    let mut token = scanner.scan();
    let mut result: Vec<(usize, usize)> = Vec::new();
    let mut inside_attribute = false;
    let mut attr_start = 0;
    while token != TokenType::Eos {
        match token {
            TokenType::AttributeName => {
                if relative_offset < scanner.token_offset() {
                    inside_attribute = false;
                } else {
                    if relative_offset <= scanner.token_end() {
                        // `class`
                        result.insert(0, (scanner.token_offset(), scanner.token_end()));
                    }
                    inside_attribute = true;
                    attr_start = scanner.token_offset();
                }
            }
            TokenType::AttributeValue if inside_attribute => {
                let value = scanner.token_text().as_bytes();
                let (start, end) = (scanner.token_offset(), scanner.token_end());
                if relative_offset < start {
                    // `class="foo"`
                    result.push((attr_start, end));
                } else if relative_offset <= end {
                    // `"foo"`
                    result.insert(0, (start, end));
                    // `foo`
                    if matches!(
                        (value.first(), value.last()),
                        (Some(b'"'), Some(b'"')) | (Some(b'\''), Some(b'\''))
                    ) && relative_offset > start
                        && relative_offset < end
                    {
                        result.insert(0, (start + 1, end - 1));
                    }
                    // `class="foo"`
                    result.push((attr_start, end));
                }
            }
            _ => {}
        }
        token = scanner.scan();
    }
    result
        .into_iter()
        .map(|(a, b)| (a + node.start, b + node.start))
        .collect()
}
