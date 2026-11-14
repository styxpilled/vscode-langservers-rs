use jsonc::{NodeType, Scanner, SyntaxKind};
use lsp_types::{Position, Range, SelectionRange};

use crate::{JsonDocument, TextDocument};

pub fn get_selection_ranges(
    document: &TextDocument,
    positions: &[Position],
    doc: &JsonDocument,
) -> Vec<SelectionRange> {
    let mut scanner = Scanner::new(document.text(), true);
    let mut offset_after_comma = |offset: usize| {
        scanner.set_position(offset);
        (scanner.scan() == SyntaxKind::CommaToken)
            .then(|| scanner.token_offset() + scanner.token_length())
    };

    positions
        .iter()
        .map(|&position| {
            let offset = document.offset_at(position);
            let mut result: Vec<Range> = Vec::new();
            let mut node = doc.get_node_from_offset(offset, true);
            while let Some(id) = node {
                let ast = &doc[id];
                let node_type = ast.node_type();
                if matches!(
                    node_type,
                    NodeType::String | NodeType::Object | NodeType::Array
                ) {
                    // The range without the quotes, brackets or braces.
                    let (content_start, content_end) =
                        (ast.offset() + 1, ast.end().saturating_sub(1));
                    if content_start < content_end
                        && offset >= content_start
                        && offset <= content_end
                    {
                        result.push(document.range(content_start, content_end));
                    }
                }
                result.push(document.range(ast.offset(), ast.end()));
                let in_array = ast
                    .parent
                    .is_some_and(|parent| doc[parent].node_type() == NodeType::Array);
                if (node_type == NodeType::Property || in_array)
                    && let Some(after_comma) = offset_after_comma(ast.end())
                {
                    result.push(document.range(ast.offset(), after_comma));
                }
                node = ast.parent;
            }

            let mut current: Option<SelectionRange> = None;
            for range in result.into_iter().rev() {
                current = Some(SelectionRange {
                    range,
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
