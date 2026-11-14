//! Port of `cssSelectionRange.ts`.

use lsp_types::{Position, Range, SelectionRange};
use text_document::TextDocument;

use crate::parser::Stylesheet;
use crate::parser::nodes::NodeType;

/// `getSelectionRanges`.
pub fn get_selection_ranges(
    document: &TextDocument,
    positions: &[Position],
    stylesheet: &Stylesheet,
) -> Vec<SelectionRange> {
    let position_at = |offset: isize| document.position_at(offset.max(0) as usize);
    positions
        .iter()
        .map(|&position| {
            let mut current: Option<SelectionRange> = None;
            for (start, end) in applicable_ranges(document, position, stylesheet)
                .into_iter()
                .rev()
            {
                current = Some(SelectionRange {
                    range: Range::new(position_at(start), position_at(end)),
                    parent: current.map(Box::new),
                });
            }
            current.unwrap_or(SelectionRange {
                range: Range::new(position, position),
                parent: None,
            })
        })
        .collect()
}

fn applicable_ranges(
    document: &TextDocument,
    position: Position,
    stylesheet: &Stylesheet,
) -> Vec<(isize, isize)> {
    let ast = &stylesheet.ast;
    let offset = document.offset_at(position) as isize;
    let mut result = Vec::new();
    let mut current = ast.find_child_at_offset(stylesheet.root, offset, true);
    while let Some(node) = current {
        let (start, end) = (ast.offset(node), ast.end(node));
        if let Some(parent) = ast.parent(node)
            && start == ast.offset(parent)
            && end == ast.end(parent)
        {
            current = Some(parent);
            continue;
        }
        // The `{ }` part of `.a { }`: also the range inside the braces.
        if ast.node_type(node) == NodeType::Declarations && offset > start && offset < end {
            result.push((start + 1, end - 1));
        }
        result.push((start, end));
        current = ast.parent(node);
    }
    result
}
