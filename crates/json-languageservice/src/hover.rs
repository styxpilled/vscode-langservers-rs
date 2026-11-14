//! Port of `services/jsonHover.ts`.

use lsp_types::{Hover, HoverContents, MarkedString, Position};
use serde_json::Value;

use crate::schema_service::JsonSchemaService;
use crate::strings::{js_stringify, js_trim};
use crate::{AstKind, JsonDocument, TextDocument};

pub fn do_hover(
    service: &mut JsonSchemaService,
    document: &TextDocument,
    position: Position,
    doc: &JsonDocument,
) -> Option<Hover> {
    let offset = document.offset_at(position);
    let mut node = doc.get_node_from_offset(offset, false)?;
    let n = &doc[node];
    if matches!(n.kind, AstKind::Object | AstKind::Array) && offset > n.offset() + 1 {
        // An unclosed node can end with a non-ASCII character.
        let last_char = document.text()[..n.end()]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i);
        if offset < last_char {
            return None;
        }
    }
    let hover_range = document.range(n.offset(), n.end());

    if matches!(n.kind, AstKind::String(_))
        && let Some(parent) = n.parent
        && matches!(doc[parent].kind, AstKind::Property { .. })
        && doc.children(parent)[0] == node
    {
        node = doc.value(parent)?;
    }

    let schema = service.get_schema_for_resource(document.uri().as_str(), Some(doc))?;
    let mut title = None;
    let mut markdown_description = None;
    let mut markdown_enum_value_description = None;
    let mut enum_value = None;
    let matching = doc.get_matching_schemas(
        &schema.schemas,
        schema.root,
        Some(doc[node].offset()),
        None,
        schema.active_vocabularies.as_ref(),
    );
    for s in matching.iter().filter(|s| s.node == node && !s.inverted) {
        let s = &schema.schemas[s.schema];
        title = title.or_else(|| s.title.clone().filter(|t| !t.is_empty()));
        markdown_description = markdown_description
            .or_else(|| s.markdown_description.clone().filter(|d| !d.is_empty()))
            .or_else(|| to_markdown(s.description.as_deref()));
        let Some(e) = &s.enum_ else {
            continue;
        };
        let value = doc.node_value(node);
        let index = e.iter().position(|v| strict_equals(v, &value));
        let at = |list: &Option<Vec<Option<String>>>| {
            list.as_ref()
                .zip(index)
                .and_then(|(l, i)| l.get(i).cloned().flatten())
        };
        if s.vscode().markdown_enum_descriptions.is_some() {
            markdown_enum_value_description = at(&s.vscode().markdown_enum_descriptions);
        } else if s.vscode().enum_descriptions.is_some() {
            markdown_enum_value_description =
                to_markdown(at(&s.vscode().enum_descriptions).as_deref());
        }
        if let (Some(d), Some(i)) = (&markdown_enum_value_description, index)
            && !d.is_empty()
        {
            enum_value = Some(match &e[i] {
                Value::String(s) => s.clone(),
                v => js_stringify(v),
            });
        }
    }

    let mut result = title
        .and_then(|t| to_markdown(Some(&t)))
        .unwrap_or_default();
    if let Some(d) = markdown_description {
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        result.push_str(&d);
    }
    if let (Some(d), Some(v)) = (
        markdown_enum_value_description.filter(|d| !d.is_empty()),
        enum_value,
    ) {
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        result.push_str(&format!("`{}`: {d}", to_markdown_code_block(&v)));
    }
    Some(Hover {
        contents: HoverContents::Array(vec![MarkedString::String(result)]),
        range: Some(hover_range),
    })
}

/// JS `===` on JSON values: objects and arrays are never equal.
fn strict_equals(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(_) | Value::Object(_), _) => false,
        _ => a == b,
    }
}

/// Escapes markdown syntax. Returns `None` for an empty result.
fn to_markdown(plain: Option<&str>) -> Option<String> {
    let plain = js_trim(plain.filter(|p| !p.is_empty())?);
    let mut escaped = String::with_capacity(plain.len());
    for c in plain.chars() {
        if "\\`*_{}[]()<>#+-.!".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    let mut out = String::with_capacity(escaped.len());
    let mut line_start = true;
    let mut chars = escaped.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' => {
                let mut n = 1;
                while chars.next_if_eq(&' ').is_some() {
                    n += 1;
                }
                if line_start {
                    out.push_str(&"&nbsp;".repeat(n));
                } else if n >= 2 {
                    out.push(' ');
                    out.push_str(&"&nbsp;".repeat(n - 1));
                } else {
                    out.push(' ');
                }
            }
            '\t' => out.push_str("&nbsp;&nbsp;&nbsp;&nbsp;"),
            '\n' => out.push_str("\\\n"),
            c => out.push(c),
        }
        line_start = matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    }
    Some(out).filter(|s| !s.is_empty())
}

fn to_markdown_code_block(content: &str) -> String {
    if content.contains('`') {
        format!("`` {content} ``")
    } else {
        content.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::to_markdown;

    #[test]
    fn markdown() {
        assert_eq!(
            to_markdown(Some(
                "For example:\n\n<script>\n  alert(1)\n</script>\n\n    Test [1]"
            ))
            .unwrap(),
            "For example:\\\n\\\n\\<script\\>\\\n&nbsp;&nbsp;alert\\(1\\)\\\n\\</script\\>\\\n\\\n&nbsp;&nbsp;&nbsp;&nbsp;Test \\[1\\]"
        );
        assert_eq!(
            to_markdown(Some("a  b\t c")).unwrap(),
            "a &nbsp;b&nbsp;&nbsp;&nbsp;&nbsp; c"
        );
        assert_eq!(to_markdown(Some("  ")), None);
    }
}
