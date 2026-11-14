//! Compares the node trees with the upstream parser for each `internalParse`
//! call of the upstream tests. Fixture: `scripts/css/trace-parser.mjs`.

use css_languageservice::parser::nodes::{Ast, Kind, NodeId};
use css_languageservice::parser::{Dialect, Parser};
use serde_json::{Map, Value, json};

fn utf16(text: &str, offset: isize) -> isize {
    if offset < 0 {
        return offset;
    }
    let n = (offset as usize).min(text.len());
    (text[..n].encode_utf16().count() + offset as usize - n) as isize
}

fn camel(name: &str) -> String {
    let mut chars = name.chars();
    let first = chars.next().unwrap().to_ascii_lowercase();
    std::iter::once(first).chain(chars).collect()
}

fn dump(ast: &Ast, id: NodeId) -> Value {
    let text = ast.text();
    let node = ast.node(id);
    let start = utf16(text, node.offset());
    let length = if node.offset() < 0 {
        node.length()
    } else {
        utf16(text, node.end()) - start
    };
    let mut out = Map::new();
    out.insert("c".into(), json!(format!("{:?}", node.kind)));
    out.insert("t".into(), json!(format!("{:?}", node.node_type())));
    out.insert("o".into(), json!(start));
    out.insert("l".into(), json!(length));
    let issues = ast.issues(id);
    if !issues.is_empty() {
        let issues: Vec<Value> = issues
            .iter()
            .map(|m| {
                let o = utf16(text, m.offset);
                json!([
                    m.rule.id,
                    o,
                    utf16(text, m.offset + m.length) - o,
                    m.level as u8
                ])
            })
            .collect();
        out.insert("i".into(), json!(issues));
    }
    let children: Vec<NodeId> = ast.children(id).collect();
    let mut fields = Map::new();
    for (field, child) in ast.fields(id) {
        let value = match children.iter().position(|&c| c == child) {
            Some(i) => json!(i),
            None => dump(ast, child),
        };
        fields.insert(camel(&format!("{field:?}")), value);
    }
    if !fields.is_empty() {
        out.insert("f".into(), Value::Object(fields));
    }
    let mut scalars = Map::new();
    if node.kind == Kind::Identifier {
        scalars.insert("isCustomProperty".into(), json!(node.is_custom_property));
    }
    if let Some(e) = ast.extra(id) {
        let pos = |p: Option<u32>| p.map(|p| json!(utf16(text, p as isize)));
        if let Some(types) = e.reference_types {
            let types: Vec<u8> = types.iter().map(|t| *t as u8).collect();
            scalars.insert("referenceTypes".into(), json!(types));
        }
        for (name, value) in [
            ("colonPosition", pos(e.colon_position)),
            ("semicolonPosition", pos(e.semicolon_position)),
            ("lParent", pos(e.l_parent)),
            ("rParent", pos(e.r_parent)),
            ("isNegated", e.is_negated.map(|b| json!(b))),
        ] {
            if let Some(value) = value {
                scalars.insert(name.into(), value);
            }
        }
    }
    if node.kind == Kind::VariableDeclaration {
        let needs = ast.extra(id).is_none_or(|e| e.needs_semicolon);
        scalars.insert("needsSemicolon".into(), json!(needs));
    }
    if node.kind == Kind::Identifier && !scalars.contains_key("isCustomProperty") {
        scalars.insert("isCustomProperty".into(), json!(false));
    }
    if !scalars.is_empty() {
        out.insert("s".into(), Value::Object(scalars));
    }
    if !children.is_empty() {
        let children: Vec<Value> = children.iter().map(|&c| dump(ast, c)).collect();
        out.insert("ch".into(), json!(children));
    }
    Value::Object(out)
}

fn token_name(name: &str) -> String {
    name.to_lowercase()
}

#[test]
fn upstream_trees() {
    check(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/parser.json"
    ));
}

/// Each prefix of each style sheet of the fixture, also with non-ASCII text.
/// Skips without `CSS_PARSER_SWEEP` (the absolute path of the sweep file from
/// `scripts/css/gen-parser-sweep.mjs` and `trace-parser.mjs`).
#[test]
fn upstream_sweep() {
    if let Ok(path) = std::env::var("CSS_PARSER_SWEEP") {
        check(&path);
    }
}

fn check(path: &str) {
    let only = std::env::var("CSS_PARSER_DIALECTS").unwrap_or_else(|_| "css,scss,less".into());
    let fixture = std::fs::read_to_string(path).unwrap();
    let cases: Vec<Value> = serde_json::from_str(&fixture).unwrap();
    let mut failures = 0;
    let mut checked = 0;
    for case in &cases {
        let dialect_name = case["dialect"].as_str().unwrap();
        if !only.split(',').any(|d| d == dialect_name) {
            continue;
        }
        let dialect = match dialect_name {
            "scss" => Dialect::Scss,
            "less" => Dialect::Less,
            _ => Dialect::Css,
        };
        let input = case["input"].as_str().unwrap();
        let name = case["fn"].as_str().unwrap();
        let mut parser = Parser::new(dialect, input);
        let Some(result) = parser.internal_parse(name) else {
            panic!("no parse function {name}");
        };
        checked += 1;
        let tree = result.map_or(Value::Null, |id| dump(&parser.ast, id));
        let next = json!([
            token_name(&format!("{:?}", parser.token_kind())),
            utf16(input, parser.token_offset() as isize)
        ]);
        let expected_next = json!([
            token_name(case["next"][0].as_str().unwrap()),
            case["next"][1]
        ]);
        if tree != case["tree"] || next != expected_next {
            failures += 1;
            if failures <= 5 {
                eprintln!(
                    "{dialect_name} {name} {input:?}\n  expected {} {}\n  actual   {} {}",
                    case["tree"], expected_next, tree, next
                );
            }
        }
    }
    assert!(checked > 0);
    assert_eq!(failures, 0, "{failures} of {checked} parses differ");
}
