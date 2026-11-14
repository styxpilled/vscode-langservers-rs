//! Port of `services/jsonLinks.ts`.

use lsp_types::DocumentLink;

use crate::{AstKind, JsonDocument, NodeId, TextDocument};

pub fn find_links(document: &TextDocument, doc: &JsonDocument) -> Vec<DocumentLink> {
    let mut links = Vec::new();
    doc.visit(|node| {
        if matches!(doc[node].kind, AstKind::Property { .. })
            && doc.key(node) == "$ref"
            && let Some(value) = doc.value(node)
            && let Some(path) = doc.string(value)
            && let Some(target) = find_target_node(doc, path)
        {
            let position = document.position_at(doc[target].offset());
            let target = format!(
                "{}#{},{}",
                document.uri().as_str(),
                position.line + 1,
                position.character + 1
            );
            let v = &doc[value];
            links.push(DocumentLink {
                range: document.range(v.offset() + 1, v.end() - 1),
                target: target.parse().ok(),
                tooltip: None,
                data: None,
            });
        }
        true
    });
    links
}

fn unescape(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

fn find_target_node(doc: &JsonDocument, path: &str) -> Option<NodeId> {
    if path == "#" {
        return doc.root();
    }
    if let Some(pointer) = path.strip_prefix("#/") {
        return find_node(doc, pointer, doc.root());
    }
    if let Some(anchor) = path.strip_prefix('#') {
        return find_anchor(doc, doc.root()?, anchor);
    }
    let (uri, fragment) = match path.split_once('#') {
        Some((uri, fragment)) => (uri, Some(fragment)),
        None => (path, None),
    };
    if uri.is_empty() {
        return None;
    }
    let embedded = find_embedded_schema(doc, uri)?;
    match fragment {
        None | Some("") => Some(embedded),
        Some(f) => match f.strip_prefix('/') {
            Some(pointer) => find_node(doc, pointer, Some(embedded)),
            None => find_anchor(doc, embedded, f),
        },
    }
}

fn find_node(doc: &JsonDocument, pointer: &str, node: Option<NodeId>) -> Option<NodeId> {
    let mut node = node?;
    for token in pointer.split('/').map(unescape) {
        let n = &doc[node];
        node = match n.kind {
            AstKind::Object => {
                let property = *doc.children(node).iter().find(|&&p| doc.key(p) == token)?;
                doc.value(property)?
            }
            AstKind::Array
                if token == "0"
                    || (!token.starts_with('0')
                        && !token.is_empty()
                        && token.bytes().all(|b| b.is_ascii_digit())) =>
            {
                *doc.children(node).get(token.parse::<usize>().ok()?)?
            }
            _ => return None,
        };
    }
    Some(node)
}

/// Depth-first search for the first object for which `matches` is true.
fn find_object(
    doc: &JsonDocument,
    root: NodeId,
    skip_root: bool,
    matches: impl Fn(&str, &str) -> bool,
) -> Option<NodeId> {
    let mut result = None;
    doc.visit_from(root, |node| {
        if result.is_some() {
            return false;
        }
        if doc[node].kind == AstKind::Object
            && !(skip_root && node == root)
            && doc.children(node).iter().any(|&p| {
                doc.value(p)
                    .and_then(|v| doc.string(v))
                    .is_some_and(|v| matches(doc.key(p), v))
            })
        {
            result = Some(node);
            return false;
        }
        true
    });
    result
}

fn find_anchor(doc: &JsonDocument, root: NodeId, anchor: &str) -> Option<NodeId> {
    if anchor.is_empty() {
        return None;
    }
    find_object(doc, root, false, |key, value| {
        (key == "$anchor" && value == anchor)
            || (key == "$id" && value.strip_prefix('#') == Some(anchor))
    })
}

fn find_embedded_schema(doc: &JsonDocument, uri: &str) -> Option<NodeId> {
    find_object(doc, doc.root()?, true, |key, value| {
        (key == "$id" || key == "id") && value == uri
    })
}

#[cfg(test)]
mod tests {
    use crate::{TextDocument, parse};

    #[test]
    fn non_ascii_targets() {
        let text = r##"{"$ref": "#é", "a": {"$anchor": "é"}, "c": {"$id": "http://é", "x": {"$id": "#€"}}, "d": {"$ref": "http://é#€"}, "e": {"$ref": "#/a/$anchor"}}"##;
        let document =
            TextDocument::new("test://t.json".parse().unwrap(), "json", 0, text.to_owned());
        let targets: Vec<String> = super::find_links(&document, &parse(&document, true))
            .into_iter()
            .map(|l| l.target.unwrap().as_str().to_owned())
            .collect();
        assert_eq!(
            targets,
            [
                "test://t.json#1,21",
                "test://t.json#1,69",
                "test://t.json#1,33"
            ]
        );
    }
}
