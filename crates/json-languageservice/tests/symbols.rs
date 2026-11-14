//! Port of the tests in `vscode-json-languageservice/src/test/documentSymbols.test.ts` that do
//! not use a schema.

mod common;

use json_languageservice::{
    Color, color_from_hex, find_document_symbols, find_document_symbols2, get_color_presentations,
    parse,
};
use lsp_types::{DocumentSymbol, Position, Range, SymbolInformation, SymbolKind};

fn flat_outline(text: &str, limit: Option<usize>) -> Vec<SymbolInformation> {
    let document = common::document("test://test.json", text);
    let json = parse(&document, true);
    find_document_symbols(&document, &json, limit)
}

fn hierarchical_outline(text: &str, limit: Option<usize>) -> Vec<DocumentSymbol> {
    let document = common::document("test://test.json", text);
    let json = parse(&document, true);
    find_document_symbols2(&document, &json, limit)
}

fn assert_outline(text: &str, expected: &[(&str, SymbolKind)]) {
    let actual: Vec<(String, SymbolKind)> = flat_outline(text, None)
        .into_iter()
        .map(|s| (s.name, s.kind))
        .collect();
    let expected: Vec<(String, SymbolKind)> =
        expected.iter().map(|(n, k)| (n.to_string(), *k)).collect();
    assert_eq!(actual, expected);
}

struct Expected {
    label: &'static str,
    kind: SymbolKind,
    detail: Option<&'static str>,
    children: Vec<Expected>,
}

fn e(
    label: &'static str,
    kind: SymbolKind,
    detail: Option<&'static str>,
    children: Vec<Expected>,
) -> Expected {
    Expected {
        label,
        kind,
        detail,
        children,
    }
}

fn assert_symbols(actual: &[DocumentSymbol], expected: &[Expected]) {
    assert_eq!(actual.len(), expected.len());
    for (a, e) in actual.iter().zip(expected) {
        assert_eq!(a.name, e.label);
        assert_eq!(a.kind, e.kind);
        assert_eq!(a.detail.as_deref(), e.detail);
        assert_symbols(a.children.as_deref().unwrap_or_default(), &e.children);
    }
}

fn assert_hierarchical_outline(text: &str, expected: &[Expected]) {
    assert_symbols(&hierarchical_outline(text, None), expected);
}

#[test]
fn outline_base_types() {
    assert_outline(
        r#"{ "key1": 1, "key2": "foo", "key3" : true }"#,
        &[
            ("key1", SymbolKind::NUMBER),
            ("key2", SymbolKind::STRING),
            ("key3", SymbolKind::BOOLEAN),
        ],
    );
}

#[test]
fn outline_arrays() {
    assert_outline(
        r#"{ "key1": 1, "key2": [ 1, 2, 3 ], "key3" : [ { "k1": 1 }, {"k2": 2 } ] }"#,
        &[
            ("key1", SymbolKind::NUMBER),
            ("key2", SymbolKind::ARRAY),
            ("key3", SymbolKind::ARRAY),
            ("k1", SymbolKind::NUMBER),
            ("k2", SymbolKind::NUMBER),
        ],
    );
}

#[test]
fn outline_objects() {
    assert_outline(
        r#"{ "key1": { "key2": true }, "key3" : { "k1":  { } }"#,
        &[
            ("key1", SymbolKind::MODULE),
            ("key3", SymbolKind::MODULE),
            ("key2", SymbolKind::BOOLEAN),
            ("k1", SymbolKind::MODULE),
        ],
    );
}

#[test]
fn outline_object_with_syntax_error() {
    assert_outline(
        r#"{ "key1": { "key2": true, "key3":, "key4": false } }"#,
        &[
            ("key1", SymbolKind::MODULE),
            ("key2", SymbolKind::BOOLEAN),
            ("key4", SymbolKind::BOOLEAN),
        ],
    );
}

#[test]
fn outline_empty_name() {
    let content = r#"{ "": 1, " ": 2 }"#;
    assert_outline(
        content,
        &[
            (r#""""#, SymbolKind::NUMBER),
            (r#"" ""#, SymbolKind::NUMBER),
        ],
    );
    assert_hierarchical_outline(
        content,
        &[
            e(r#""""#, SymbolKind::NUMBER, Some("1"), vec![]),
            e(r#"" ""#, SymbolKind::NUMBER, Some("2"), vec![]),
        ],
    );
}

#[test]
fn outline_new_line_in_name() {
    let content = r#"{ "1\n2": 1 }"#;
    assert_outline(content, &[("1↵2", SymbolKind::NUMBER)]);
    assert_hierarchical_outline(content, &[e("1↵2", SymbolKind::NUMBER, Some("1"), vec![])]);
}

#[test]
fn hierarchical_outline_object() {
    assert_hierarchical_outline(
        r#"{ "key1": { "key2": true }, "key3" : { "k1":  { } }"#,
        &[
            e(
                "key1",
                SymbolKind::MODULE,
                None,
                vec![e("key2", SymbolKind::BOOLEAN, Some("true"), vec![])],
            ),
            e(
                "key3",
                SymbolKind::MODULE,
                None,
                vec![e("k1", SymbolKind::MODULE, Some("{}"), vec![])],
            ),
        ],
    );
}

#[test]
fn hierarchical_outline_array() {
    assert_hierarchical_outline(
        r#"{ "key1": [ { "key2": true }, { "k1": [] } ]}"#,
        &[e(
            "key1",
            SymbolKind::ARRAY,
            None,
            vec![
                e(
                    "0",
                    SymbolKind::MODULE,
                    None,
                    vec![e("key2", SymbolKind::BOOLEAN, Some("true"), vec![])],
                ),
                e(
                    "1",
                    SymbolKind::MODULE,
                    None,
                    vec![e("k1", SymbolKind::ARRAY, Some("[]"), vec![])],
                ),
            ],
        )],
    );
}

#[test]
fn outline_limit_1() {
    let mut content = String::from("{");
    for i in 0..100 {
        content.push_str(&format!("\"prop{i}\": {i},"));
    }
    content.push('}');
    assert_eq!(flat_outline(&content, Some(10)).len(), 10);
    assert_eq!(hierarchical_outline(&content, Some(10)).len(), 10);
}

#[test]
fn outline_limit_2() {
    let mut content = String::from("[");
    for i in 0..10 {
        content.push('{');
        for k in 0..10 {
            content.push_str(&format!("\"{i}-{k}\": {k},"));
        }
        content.push_str("},");
    }
    content.push(']');

    let flat = flat_outline(&content, Some(25));
    let names: Vec<&str> = flat.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names.join(","),
        "0-0,0-1,0-2,0-3,0-4,0-5,0-6,0-7,0-8,0-9,1-0,1-1,1-2,1-3,1-4,1-5,1-6,1-7,1-8,1-9,2-0,2-1,2-2,2-3,2-4"
    );

    let hierarchical = hierarchical_outline(&content, Some(25));
    let child_count = |i: usize| hierarchical[i].children.as_ref().unwrap().len();
    assert_eq!(hierarchical.len(), 10);
    assert_eq!(child_count(0), 10);
    assert_eq!(child_count(1), 5);
    assert_eq!(child_count(2), 0);
}

#[test]
fn color_presentations() {
    let range = Range::new(Position::new(0, 0), Position::new(0, 1));
    let rgb = |r: f64, g: f64, b: f64, a: f64| Color {
        red: r / 255.0,
        green: g / 255.0,
        blue: b / 255.0,
        alpha: a,
    };
    let check = |color: Color, expected: &str| {
        let result = get_color_presentations(color, range);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, expected);
        let edit = result[0].text_edit.as_ref().unwrap();
        assert_eq!(edit.range, range);
        assert_eq!(edit.new_text, format!("\"{expected}\""));
    };
    check(rgb(255.0, 0.0, 0.0, 1.0), "#ff0000");
    check(rgb(77.0, 33.0, 111.0, 0.5), "#4d216f80");
}

/// Colors are `f64` and serialize as `JSON.stringify` (values from Node).
#[test]
fn colors_as_js() {
    let json = |text: &str| serde_json::to_string(&color_from_hex(text).unwrap()).unwrap();
    assert_eq!(
        json("#6A5ACD80"),
        r#"{"red":0.41568627450980394,"green":0.35294117647058826,"blue":0.803921568627451,"alpha":0.5019607843137255}"#
    );
    assert_eq!(
        json("#AAAF"),
        r#"{"red":0.6666666666666666,"green":0.6666666666666666,"blue":0.6666666666666666,"alpha":1}"#
    );
    // Zed sends `f32` values as text.
    let color: Color =
        serde_json::from_str(r#"{"red":0.41568628,"green":0,"blue":1,"alpha":1}"#).unwrap();
    let range = Range::new(Position::new(0, 0), Position::new(0, 1));
    assert_eq!(get_color_presentations(color, range)[0].label, "#6a00ff");
}
