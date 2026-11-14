//! Port of the tests in `vscode-json-languageservice/src/test/parser.test.ts` that do not use a
//! schema.

mod common;

use common::parse_text;
use json_languageservice::{AstKind, ErrorCode, JsonDocument, NodeId};
use jsonc::{NodeType, Segment};
use lsp_types::NumberOrString;

fn codes(json: &JsonDocument) -> Vec<ErrorCode> {
    json.syntax_errors
        .iter()
        .map(|e| match e.code {
            Some(NumberOrString::Number(n)) => ErrorCode(n as u32),
            _ => panic!("no code"),
        })
        .collect()
}

fn is_valid(text: &str) {
    let (_, json) = parse_text(text);
    assert_eq!(json.syntax_errors, [], "{text:?}");
}

fn is_invalid(text: &str, expected: &[ErrorCode]) {
    let (_, json) = parse_text(text);
    if expected.is_empty() {
        assert!(!json.syntax_errors.is_empty(), "{text:?}");
    } else {
        assert_eq!(codes(&json), expected, "{text:?}");
    }
}

fn node_type(json: &JsonDocument, node: Option<NodeId>) -> Option<NodeType> {
    node.map(|id| json[id].node_type())
}

fn path(segments: &[Segment]) -> Vec<Segment> {
    segments.to_vec()
}

fn assert_object(json: &JsonDocument, node: NodeId, expected: &[&str]) {
    assert_eq!(json[node].kind, AstKind::Object);
    let keys: Vec<&str> = json.children(node).iter().map(|&p| json.key(p)).collect();
    assert_eq!(keys, expected);
}

#[test]
fn invalid_body() {
    let (_, json) = parse_text("*");
    assert_eq!(json.syntax_errors.len(), 1);
    is_invalid("{}[]", &[]);
}

#[test]
fn trailing_whitespace() {
    is_valid("{}\n\n");
}

#[test]
fn no_content() {
    is_valid("");
    is_valid("   ");
    is_valid("\n\n");
    is_valid("/*hello*/  ");
}

#[test]
fn objects() {
    is_valid("{}");
    is_valid(r#"{"key": "value"}"#);
    is_valid(r#"{"key1": true, "key2": 3, "key3": [null], "key4": { "nested": {}}}"#);
    is_valid(r#"{"constructor": true }"#);

    is_invalid("{", &[]);
    is_invalid("{3:3}", &[]);
    is_invalid("{'key': 3}", &[]);
    is_invalid(r#"{"key" 3}"#, &[ErrorCode::COLON_EXPECTED]);
    is_invalid(r#"{"key":3 "key2": 4}"#, &[ErrorCode::COMMA_EXPECTED]);
    is_invalid(r#"{"key":42, }"#, &[ErrorCode::TRAILING_COMMA]);
    is_invalid(
        r#"{"key:42"#,
        &[
            ErrorCode::UNEXPECTED_END_OF_STRING,
            ErrorCode::COLON_EXPECTED,
        ],
    );
}

#[test]
fn arrays() {
    is_valid("[]");
    is_valid("[1, 2]");
    is_valid(r#"[1, "string", false, {}, [null]]"#);

    is_invalid("[", &[]);
    is_invalid("[,]", &[ErrorCode::VALUE_EXPECTED]);
    is_invalid("[1 2]", &[ErrorCode::COMMA_EXPECTED]);
    is_invalid("[true false]", &[ErrorCode::COMMA_EXPECTED]);
    is_invalid("[1, ]", &[ErrorCode::TRAILING_COMMA]);
    is_invalid("[[]", &[ErrorCode::COMMA_OR_CLOSE_BACKET_EXPECTED]);
    is_invalid(r#"["something""#, &[]);
    is_invalid("[magic]", &[]);
}

#[test]
fn strings() {
    is_valid(r#"["string"]"#);
    is_valid(r#"["\"\\\/\b\f\n\r\tሴካ"]"#);
    is_valid(r#"["\\"]"#);

    is_invalid(r#"[""#, &[]);
    is_invalid(r#"["]"#, &[]);
    is_invalid(r#"["\z"]"#, &[]);
    is_invalid(r#"["\u"]"#, &[]);
    is_invalid(r#"["\u123"]"#, &[]);
    is_invalid(r#"["\u123Z"]"#, &[]);
    is_invalid("['string']", &[]);
    is_invalid("\"\tabc\"", &[ErrorCode::INVALID_CHARACTER]);
}

#[test]
fn numbers() {
    is_valid("[0, -1, 186.1, 0.123, -1.583e+4, 1.583E-4, 5e8]");

    is_invalid("[+1]", &[]);
    is_invalid("[01]", &[]);
    is_invalid("[1.]", &[]);
    is_invalid("[1.1+3]", &[]);
    is_invalid("[1.4e]", &[]);
    is_invalid("[-A]", &[]);
}

#[test]
fn comments() {
    is_valid("/*d*/ { } /*e*/");
    is_invalid("/*d { }", &[]);

    is_valid(r#"{ "//": "comment1", "//": "comment2" }"#);
    is_invalid(
        r#"{ "regularKey": "value1", "regularKey": "value2" }"#,
        &[ErrorCode::DUPLICATE_KEY, ErrorCode::DUPLICATE_KEY],
    );
}

#[test]
fn simple_ast() {
    {
        let (_, json) = parse_text("{}");
        assert_eq!(json.syntax_errors, []);
        let node = json.get_node_from_offset(1, false).unwrap();
        assert_eq!(json[node].node_type(), NodeType::Object);
        assert_eq!(json.node_path(node), []);
        assert_eq!(json.get_node_from_offset(2, false), None);
    }
    {
        let (_, json) = parse_text("[null]");
        assert_eq!(json.syntax_errors, []);
        let node = json.get_node_from_offset(2, false).unwrap();
        assert_eq!(json[node].node_type(), NodeType::Null);
        assert_eq!(json.node_path(node), path(&[0.into()]));
    }
    {
        let (_, json) = parse_text(r#"{"a":true}"#);
        assert_eq!(json.syntax_errors, []);

        let node = json.get_node_from_offset(3, false).unwrap();
        assert_eq!(json[node].node_type(), NodeType::String);
        assert_eq!(json.node_path(node), path(&["a".into()]));

        let node = json.get_node_from_offset(4, false);
        assert_eq!(node_type(&json, node), Some(NodeType::Property));

        let node = json.get_node_from_offset(0, false);
        assert_eq!(node_type(&json, node), Some(NodeType::Object));

        assert_eq!(json.get_node_from_offset(10, false), None);

        let node = json.get_node_from_offset(5, false).unwrap();
        assert_eq!(json[node].node_type(), NodeType::Boolean);
        assert_eq!(json.node_path(node), path(&["a".into()]));
    }
}

#[test]
fn nested_ast() {
    let content = "{\n\t\"key\" : {\n\t\"key2\": 42\n\t}\n}";
    let (_, json) = parse_text(content);
    assert_eq!(json.syntax_errors, []);

    let node = json
        .get_node_from_offset(content.find("key2").unwrap() + 2, false)
        .unwrap();
    assert_eq!(json.node_path(node), path(&["key".into(), "key2".into()]));

    let node = json
        .get_node_from_offset(content.find("42").unwrap() + 1, false)
        .unwrap();
    assert_eq!(json.node_path(node), path(&["key".into(), "key2".into()]));
}

#[test]
fn nested_ast_in_array() {
    let (_, json) = parse_text(r#"{"key":[{"key2":42}]}"#);
    assert_eq!(json.syntax_errors, []);
    let node = json.get_node_from_offset(17, false).unwrap();
    assert_eq!(
        json.node_path(node),
        path(&["key".into(), 0.into(), "key2".into()])
    );
}

#[test]
fn multiline() {
    {
        let content = "{\n\t\n}";
        let (_, json) = parse_text(content);
        assert_eq!(json.syntax_errors, []);
        let node = json.get_node_from_offset(content.find('\t').unwrap() + 1, false);
        assert!(node.is_some());
    }
    {
        let content = "{\n\"first\":true\n\n}";
        let (_, json) = parse_text(content);
        let node = json.get_node_from_offset(content.len() - 2, false);
        assert_eq!(node_type(&json, node), Some(NodeType::Object));
        let node = json.get_node_from_offset(content.len() - 4, false);
        assert_eq!(node_type(&json, node), Some(NodeType::Boolean));
    }
}

#[test]
fn expand_errors_to_entire_tokens() {
    let content = "{\n\"key\":32,\nerror\n}";
    let (document, json) = parse_text(content);
    assert_eq!(json.syntax_errors.len(), 2);
    let start = content.find("error").unwrap();
    assert_eq!(
        json.syntax_errors[0].range,
        document.range(start, start + 5)
    );
}

#[test]
fn errors_at_the_end_of_the_file() {
    let content = "{\n\"key\":32\n ";
    let (document, json) = parse_text(content);
    assert_eq!(json.syntax_errors.len(), 1);
    assert_eq!(json.syntax_errors[0].range, document.range(9, 10));
}

#[test]
fn getting_keys_out_of_an_object() {
    let content = "{\n\"key\":32,\n\n\"key2\":45}";
    let (_, json) = parse_text(content);
    assert_eq!(json.syntax_errors, []);
    let node = json
        .get_node_from_offset(content.find("32,\n").unwrap() + 4, false)
        .unwrap();
    assert_object(&json, node, &["key", "key2"]);
}

#[test]
fn missing_colon() {
    let content = "{\n\"key\":32,\n\"key2\"\n\"key3\": 4 }";
    let (_, json) = parse_text(content);
    assert_eq!(codes(&json), [ErrorCode::COLON_EXPECTED]);
    assert_object(&json, json.root().unwrap(), &["key", "key2", "key3"]);
}

#[test]
fn missing_comma() {
    let content = "{\n\"key\":32,\n\"key2\": 1 \n\"key3\": 4 }";
    let (_, json) = parse_text(content);
    assert_eq!(codes(&json), [ErrorCode::COMMA_EXPECTED]);
    assert_object(&json, json.root().unwrap(), &["key", "key2", "key3"]);
}

#[test]
fn get_node_from_offset() {
    let content = "{\"a\": 1,\n\n\"d\": 2}";
    let (_, json) = parse_text(content);
    assert_eq!(json.syntax_errors, []);
    let node = json.get_node_from_offset(content.find(": 2").unwrap() + 1, false);
    assert_eq!(node_type(&json, node), Some(NodeType::Property));
}

#[test]
fn duplicate_keys() {
    for (text, count) in [
        (r#"{"a": 1, "a": 2}"#, 2),
        (r#"{"a": { "a": 2, "a": 3}}"#, 2),
        (r#"[{ "a": 2, "a": 3, "a": 7}]"#, 3),
    ] {
        let (_, json) = parse_text(text);
        assert_eq!(json.syntax_errors.len(), count, "{text}");
    }
}

#[test]
fn positions_are_utf16() {
    let content = "{\"😀\": 1 2}";
    let (document, json) = parse_text(content);
    let start = content.find('2').unwrap();
    assert_eq!(
        json.syntax_errors[0].range,
        document.range(start, start + 1)
    );
    assert_eq!(json.syntax_errors[0].range.start.character, 9);
}
