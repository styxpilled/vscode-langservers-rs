//! Ports of `htmlHighlighting.ts`, `htmlSymbolsProvider.ts`, `htmlRename.ts`,
//! `htmlLinkedEditing.ts` and `htmlMatchingTagPosition.ts`.

use std::collections::HashMap;

use lsp_types::{
    DocumentHighlight, DocumentHighlightKind, DocumentSymbol, Location, Position, Range,
    SymbolInformation, SymbolKind, TextEdit, WorkspaceEdit,
};
use text_document::TextDocument;

use crate::parser::scanner::{Scanner, TokenType};
use crate::parser::{HtmlDocument, NodeId};
use crate::utils::is_js_whitespace;

fn is_before_or_equal(a: Position, b: Position) -> bool {
    a.line < b.line || a.line == b.line && a.character <= b.character
}

fn covers(range: Range, position: Position) -> bool {
    is_before_or_equal(range.start, position) && is_before_or_equal(position, range.end)
}

fn tag_name_range(
    token_type: TokenType,
    document: &TextDocument,
    start_offset: usize,
) -> Option<Range> {
    let mut scanner = Scanner::at(document.text(), start_offset);
    let mut token = scanner.scan();
    while token != TokenType::Eos && token != token_type {
        token = scanner.scan();
    }
    (token != TokenType::Eos).then(|| document.range(scanner.token_offset(), scanner.token_end()))
}

/// `findDocumentHighlights`.
pub fn find_document_highlights(
    document: &TextDocument,
    position: Position,
    html_document: &HtmlDocument,
) -> Vec<DocumentHighlight> {
    let offset = document.offset_at(position);
    let node = &html_document[html_document.find_node_at(offset)];
    if node.tag.is_none() {
        return Vec::new();
    }
    let start = tag_name_range(TokenType::StartTag, document, node.start);
    let end = node
        .end_tag_start
        .and_then(|e| tag_name_range(TokenType::EndTag, document, e));
    let mut result = Vec::new();
    if start.is_some_and(|r| covers(r, position)) || end.is_some_and(|r| covers(r, position)) {
        for range in [start, end].into_iter().flatten() {
            result.push(DocumentHighlight {
                range,
                kind: Some(DocumentHighlightKind::READ),
            });
        }
    }
    result
}

/// `findDocumentSymbols`: the walk of `findDocumentSymbols2`, without the
/// tree.
#[allow(deprecated)]
pub fn find_document_symbols(
    document: &TextDocument,
    html_document: &HtmlDocument,
) -> Vec<SymbolInformation> {
    fn walk(
        document: &TextDocument,
        doc: &HtmlDocument,
        id: NodeId,
        container_name: &str,
        out: &mut Vec<SymbolInformation>,
    ) {
        let node = &doc[id];
        let index = out.len();
        out.push(SymbolInformation {
            name: String::new(),
            kind: SymbolKind::FIELD,
            tags: None,
            deprecated: None,
            location: Location::new(document.uri().clone(), document.range(node.start, node.end)),
            container_name: Some(container_name.to_owned()),
        });
        let name = node_to_name(doc, id);
        for &child in &node.children {
            walk(document, doc, child, &name, out);
        }
        out[index].name = name;
    }
    let mut symbols = Vec::with_capacity(html_document.node_count());
    for &id in html_document.roots() {
        walk(document, html_document, id, "", &mut symbols);
    }
    symbols
}

/// `findDocumentSymbols2`.
pub fn find_document_symbols2(
    document: &TextDocument,
    html_document: &HtmlDocument,
) -> Vec<DocumentSymbol> {
    #[allow(deprecated)]
    fn symbol(document: &TextDocument, doc: &HtmlDocument, id: NodeId) -> DocumentSymbol {
        let node = &doc[id];
        let range = document.range(node.start, node.end);
        let children = (!node.children.is_empty()).then(|| {
            node.children
                .iter()
                .map(|&c| symbol(document, doc, c))
                .collect()
        });
        DocumentSymbol {
            name: node_to_name(doc, id),
            detail: None,
            kind: SymbolKind::FIELD,
            tags: None,
            deprecated: None,
            range,
            selection_range: range,
            children,
        }
    }
    html_document
        .roots()
        .iter()
        .map(|&id| symbol(document, html_document, id))
        .collect()
}

fn node_to_name(doc: &HtmlDocument, id: NodeId) -> String {
    let node = &doc[id];
    let mut name = node.tag.clone();
    let push = |name: &mut Option<String>, s: &str| {
        name.get_or_insert_default().push_str(s);
    };
    if node.attributes.is_some() {
        if let Some(Some(id)) = node.attribute("id")
            && !id.is_empty()
        {
            push(&mut name, &format!("#{}", id.replace(['"', '\''], "")));
        }
        if let Some(Some(classes)) = node.attribute("class")
            && !classes.is_empty()
        {
            let classes = classes.replace(['"', '\''], "");
            let joined: String = split_whitespace_js(&classes)
                .map(|c| format!(".{c}"))
                .collect();
            push(&mut name, &joined);
        }
    }
    match name {
        Some(n) if !n.is_empty() => n,
        _ => "?".to_owned(),
    }
}

/// `s.split(/\s+/)`.
fn split_whitespace_js(s: &str) -> impl Iterator<Item = &str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if is_js_whitespace(c) {
            parts.push(&s[start..i]);
            let mut end = i + c.len_utf8();
            while let Some(&(j, d)) = chars.peek() {
                if !is_js_whitespace(d) {
                    break;
                }
                end = j + d.len_utf8();
                chars.next();
            }
            start = end;
        }
    }
    parts.push(&s[start..]);
    parts.into_iter()
}

/// `doRename`.
pub fn do_rename(
    document: &TextDocument,
    position: Position,
    new_name: &str,
    html_document: &HtmlDocument,
) -> Option<WorkspaceEdit> {
    let offset = document.offset_at(position);
    let node = &html_document[html_document.find_node_at(offset)];
    let tag = node.tag.as_deref()?;
    let len = tag.len();
    let in_end_tag = node
        .end_tag_start
        .is_some_and(|e| e + 2 <= offset && offset <= e + 2 + len);
    if !(in_end_tag || node.start < offset && offset <= node.start + 1 + len) {
        return None;
    }
    let mut edits = vec![TextEdit::new(
        document.range(node.start + 1, node.start + 1 + len),
        new_name.to_owned(),
    )];
    if let Some(e) = node.end_tag_start {
        edits.push(TextEdit::new(
            document.range(e + 2, e + 2 + len),
            new_name.to_owned(),
        ));
    }
    Some(WorkspaceEdit {
        changes: Some(HashMap::from([(document.uri().clone(), edits)])),
        ..Default::default()
    })
}

/// `findLinkedEditingRanges`.
pub fn find_linked_editing_ranges(
    document: &TextDocument,
    position: Position,
    html_document: &HtmlDocument,
) -> Option<Vec<Range>> {
    let offset = document.offset_at(position);
    let node = &html_document[html_document.find_node_at(offset)];
    let len = node.tag.as_ref().map_or(0, String::len);
    let e = node.end_tag_start?;
    let in_start = node.start < offset && offset <= node.start + 1 + len;
    let in_end = e + 2 <= offset && offset <= e + 2 + len;
    (in_start || in_end).then(|| {
        vec![
            document.range(node.start + 1, node.start + 1 + len),
            document.range(e + 2, e + 2 + len),
        ]
    })
}

/// `findMatchingTagPosition`.
pub fn find_matching_tag_position(
    document: &TextDocument,
    position: Position,
    html_document: &HtmlDocument,
) -> Option<Position> {
    let offset = document.offset_at(position);
    let node = &html_document[html_document.find_node_at(offset)];
    let len = node.tag.as_ref()?.len();
    let e = node.end_tag_start?;
    if node.start < offset && offset <= node.start + 1 + len {
        return Some(document.position_at(offset - 1 - node.start + e + 2));
    }
    if e + 2 <= offset && offset <= e + 2 + len {
        return Some(document.position_at(offset - 2 - e + node.start + 1));
    }
    None
}
