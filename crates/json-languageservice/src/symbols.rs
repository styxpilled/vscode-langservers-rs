use std::collections::VecDeque;

use jsonc::NodeType;
use lsp_types::{DocumentSymbol, Location, Range, SymbolInformation, SymbolKind};
use serde_json::Value;

use crate::strings::js_number_to_string;
use crate::{AstKind, JsonDocument, NodeId, TextDocument};

fn is_keybindings(document: &TextDocument) -> bool {
    let uri = document.uri().as_str();
    uri == "vscode://defaultsettings/keybindings.json"
        || uri.to_lowercase().ends_with("/user/keybindings.json")
}

/// For keybinding files: the `key` property of each item, with the item range.
fn keybindings(doc: &JsonDocument, root: NodeId) -> Vec<(NodeId, NodeId, NodeId)> {
    let mut result = Vec::new();
    for &item in doc.children(root) {
        if doc[item].kind != AstKind::Object {
            continue;
        }
        for &property in doc.children(item) {
            if doc.key(property) == "key"
                && let Some(value) = doc.value(property)
            {
                result.push((item, doc.children(property)[0], value));
            }
        }
    }
    result
}

/// `0` means no limit, as upstream.
fn effective_limit(limit: Option<usize>) -> usize {
    limit.filter(|&l| l > 0).unwrap_or(usize::MAX)
}

#[allow(deprecated)]
pub fn find_document_symbols(
    document: &TextDocument,
    doc: &JsonDocument,
    result_limit: Option<usize>,
) -> Vec<SymbolInformation> {
    let Some(root) = doc.root() else {
        return Vec::new();
    };
    let mut limit = effective_limit(result_limit);
    let symbol =
        |name: String, kind, node: NodeId, container_name: Option<String>| SymbolInformation {
            name,
            kind,
            tags: None,
            deprecated: None,
            location: Location::new(document.uri().clone(), range(document, doc, node)),
            container_name,
        };

    if is_keybindings(document) && doc[root].kind == AstKind::Array {
        let mut result = Vec::new();
        for (item, _, value) in keybindings(doc, root) {
            result.push(symbol(name(doc, value), SymbolKind::FUNCTION, item, None));
            limit -= 1;
            if limit == 0 {
                break;
            }
        }
        return result;
    }

    let mut result = Vec::new();
    let mut to_visit = VecDeque::from([(root, String::new())]);
    while let Some((node, container_name)) = to_visit.pop_front() {
        match doc[node].kind {
            AstKind::Array => {
                for &item in doc.children(node) {
                    to_visit.push_back((item, container_name.clone()));
                }
            }
            AstKind::Object => {
                for &property in doc.children(node) {
                    let Some(value) = doc.value(property) else {
                        continue;
                    };
                    if limit == 0 {
                        continue;
                    }
                    limit -= 1;
                    let key = doc.key(property);
                    let child_container_name = if container_name.is_empty() {
                        key.to_owned()
                    } else {
                        format!("{container_name}.{key}")
                    };
                    result.push(symbol(
                        key_label(key),
                        symbol_kind(doc[value].node_type()),
                        property,
                        Some(container_name.clone()),
                    ));
                    to_visit.push_back((value, child_container_name));
                }
            }
            _ => {}
        }
    }
    result
}

/// Hierarchical document symbols.
#[allow(deprecated)]
pub fn find_document_symbols2(
    document: &TextDocument,
    doc: &JsonDocument,
    result_limit: Option<usize>,
) -> Vec<DocumentSymbol> {
    let Some(root) = doc.root() else {
        return Vec::new();
    };
    let mut limit = effective_limit(result_limit);
    let symbol =
        |name: String, kind, range: Range, selection_range: Range, detail| DocumentSymbol {
            name,
            detail,
            kind,
            tags: None,
            deprecated: None,
            range,
            selection_range,
            children: Some(Vec::new()),
        };

    if is_keybindings(document) && doc[root].kind == AstKind::Array {
        let mut result = Vec::new();
        for (item, key, value) in keybindings(doc, root) {
            let mut s = symbol(
                name(doc, value),
                SymbolKind::FUNCTION,
                range(document, doc, item),
                range(document, doc, key),
                None,
            );
            s.children = None;
            result.push(s);
            limit -= 1;
            if limit == 0 {
                break;
            }
        }
        return result;
    }

    // Breadth-first, so that a limit keeps the symbols near the root. The symbols are built
    // flat with parent links and then nested.
    let mut flat: Vec<(DocumentSymbol, Option<usize>)> = Vec::new();
    let mut to_visit = VecDeque::from([(root, None)]);
    while let Some((node, parent)) = to_visit.pop_front() {
        let is_array = doc[node].kind == AstKind::Array;
        for (index, &child) in doc.children(node).iter().enumerate() {
            let s = if is_array {
                let range = range(document, doc, child);
                symbol(
                    index.to_string(),
                    symbol_kind(doc[child].node_type()),
                    range,
                    range,
                    None,
                )
            } else if doc[node].kind == AstKind::Object
                && let Some(value) = doc.value(child)
            {
                symbol(
                    key_label(doc.key(child)),
                    symbol_kind(doc[value].node_type()),
                    range(document, doc, child),
                    range(document, doc, doc.children(child)[0]),
                    detail(doc, value),
                )
            } else {
                continue;
            };
            if limit == 0 {
                continue;
            }
            limit -= 1;
            let value = if is_array {
                child
            } else {
                doc.value(child).unwrap()
            };
            to_visit.push_back((value, Some(flat.len())));
            flat.push((s, parent));
        }
    }

    // Children come after their parents. Going from the end, each symbol is complete when it
    // is moved into its parent. The children are collected in reverse order.
    let mut result = Vec::new();
    let mut slots: Vec<(Option<DocumentSymbol>, Option<usize>)> = flat
        .into_iter()
        .map(|(s, parent)| (Some(s), parent))
        .collect();
    for i in (0..slots.len()).rev() {
        let mut s = slots[i].0.take().unwrap();
        if let Some(children) = &mut s.children {
            children.reverse();
        }
        match slots[i].1 {
            Some(parent) => {
                let parent = slots[parent].0.as_mut().unwrap();
                parent.children.get_or_insert_with(Vec::new).push(s);
            }
            None => result.push(s),
        }
    }
    result.reverse();
    result
}

fn range(document: &TextDocument, doc: &JsonDocument, node: NodeId) -> Range {
    document.range(doc[node].offset(), doc[node].end())
}

fn name(doc: &JsonDocument, node: NodeId) -> String {
    match doc.node_value(node) {
        Value::String(s) if !s.is_empty() => s,
        Value::Bool(true) => "true".to_owned(),
        Value::Number(n) if n.as_f64() != Some(0.0) => n.to_string(),
        _ => "<empty>".to_owned(),
    }
}

fn symbol_kind(node_type: NodeType) -> SymbolKind {
    match node_type {
        NodeType::Object => SymbolKind::MODULE,
        NodeType::String => SymbolKind::STRING,
        NodeType::Number => SymbolKind::NUMBER,
        NodeType::Array => SymbolKind::ARRAY,
        NodeType::Boolean => SymbolKind::BOOLEAN,
        NodeType::Null | NodeType::Property => SymbolKind::VARIABLE,
    }
}

fn key_label(key: &str) -> String {
    let name = key.replace('\n', "↵");
    if name.trim().is_empty() {
        format!("\"{name}\"")
    } else {
        name
    }
}

fn detail(doc: &JsonDocument, node: NodeId) -> Option<String> {
    let ast = &doc[node];
    match &ast.kind {
        AstKind::Boolean(b) => Some(b.to_string()),
        AstKind::Number { value, .. } => Some(js_number_to_string(*value)),
        AstKind::Null => Some("null".to_owned()),
        AstKind::String(_) => doc.string(node).map(str::to_owned),
        AstKind::Array if doc.children(node).is_empty() => Some("[]".to_owned()),
        AstKind::Object if doc.children(node).is_empty() => Some("{}".to_owned()),
        _ => None,
    }
}
