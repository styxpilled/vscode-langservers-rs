//! Port of `node-jsonc-parser/src/test/json.test.ts`.

use jsonc::{
    Literal, NodeId, NodeType, ParseError, ParseErrorCode, ParseOptions, ScanError, Scanner,
    Segment, Span, SyntaxKind, Tree, Visitor, get_location, parse, parse_tree, visit,
};
use serde_json::{Value, json};

fn p(name: &str) -> Segment {
    Segment::from(name)
}

fn i(index: usize) -> Segment {
    Segment::from(index)
}

fn from_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn path_json(path: &[Segment]) -> Value {
    path.iter()
        .map(|segment| match segment {
            Segment::Property(name) => json!(name),
            Segment::Index(index) => json!(index),
        })
        .collect()
}

fn error_json(error: ParseErrorCode, span: Span) -> Value {
    json!({
        "error": error as u8,
        "offset": span.offset,
        "length": span.length,
        "startLine": span.start_line,
        "startCharacter": span.start_character,
    })
}

fn errors_json(errors: &[ParseError]) -> Value {
    errors
        .iter()
        .map(|e| {
            let span = Span {
                offset: e.offset,
                length: e.length,
                start_line: e.start_line,
                start_character: e.start_character,
            };
            error_json(e.error, span)
        })
        .collect()
}

fn node_json(tree: &Tree, id: NodeId) -> Value {
    let node = &tree[id];
    let node_type = match node.node_type {
        NodeType::Object => "object",
        NodeType::Array => "array",
        NodeType::Property => "property",
        NodeType::String => "string",
        NodeType::Number => "number",
        NodeType::Boolean => "boolean",
        NodeType::Null => "null",
    };
    let mut result = json!({ "type": node_type, "offset": node.offset, "length": node.length });
    if let Some(colon_offset) = node.colon_offset {
        result["colonOffset"] = json!(colon_offset);
    }
    if let Some(value) = &node.value {
        result["value"] = value.to_json();
    }
    if matches!(
        node.node_type,
        NodeType::Object | NodeType::Array | NodeType::Property
    ) {
        for &child in &node.children {
            assert_eq!(tree[child].parent, Some(id));
        }
        result["children"] = node.children.iter().map(|&c| node_json(tree, c)).collect();
    }
    result
}

fn assert_kinds(text: &str, kinds: &[SyntaxKind]) {
    let mut scanner = Scanner::new(text, false);
    let mut actual = Vec::new();
    loop {
        let kind = scanner.scan();
        if kind == SyntaxKind::EOF {
            break;
        }
        assert_eq!(scanner.token_error(), ScanError::None, "{text:?}");
        actual.push(kind);
    }
    assert_eq!(actual, kinds, "{text:?}");
}

fn assert_scan_error(text: &str, scan_error: ScanError, kinds: &[SyntaxKind]) {
    let mut scanner = Scanner::new(text, false);
    scanner.scan();
    assert_eq!(scanner.token(), kinds[0], "{text:?}");
    assert_eq!(scanner.token_error(), scan_error, "{text:?}");
    let mut actual = Vec::new();
    loop {
        let kind = scanner.scan();
        if kind == SyntaxKind::EOF {
            break;
        }
        actual.push(kind);
    }
    assert_eq!(actual, kinds[1..], "{text:?}");
}

fn assert_valid_parse(input: &str, expected: Option<&str>, options: ParseOptions) {
    let (actual, errors) = parse(input, options);
    assert_eq!(errors, [], "{input:?}");
    assert_eq!(actual, expected.map(from_json), "{input:?}");
}

fn assert_invalid_parse(input: &str, expected: Option<&str>, options: ParseOptions) {
    let (actual, errors) = parse(input, options);
    assert!(!errors.is_empty(), "{input:?}");
    assert_eq!(actual, expected.map(from_json), "{input:?}");
}

fn assert_tree(input: &str, expected: &str, expected_errors: &str) {
    let (tree, errors) = parse_tree(input, ParseOptions::default());
    assert_eq!(
        errors_json(&errors),
        from_json(expected_errors),
        "{input:?}"
    );
    let tree = tree.unwrap();
    assert_eq!(tree[tree.root()].parent, None);
    assert_eq!(
        node_json(&tree, tree.root()),
        from_json(expected),
        "{input:?}"
    );
}

struct Recorder<'a> {
    input: &'a str,
    stop_offsets: &'a [usize],
    calls: Vec<Value>,
    errors: Vec<Value>,
}

impl Recorder<'_> {
    fn record(&mut self, id: &str, span: Span, arg: Option<Value>, path: Option<&[Segment]>) {
        let mut call = json!({
            "id": id,
            "text": &self.input[span.offset..span.offset + span.length],
            "startLine": span.start_line,
            "startCharacter": span.start_character,
        });
        if let Some(arg) = arg {
            call["arg"] = arg;
        }
        if let Some(path) = path {
            call["path"] = path_json(path);
        }
        self.calls.push(call);
    }
}

impl Visitor for Recorder<'_> {
    fn on_object_begin(&mut self, span: Span, path: &[Segment]) -> bool {
        self.record("onObjectBegin", span, None, Some(path));
        !self.stop_offsets.contains(&span.offset)
    }

    fn on_object_property(&mut self, name: &str, span: Span, path: &[Segment]) {
        self.record("onObjectProperty", span, Some(json!(name)), Some(path));
    }

    fn on_object_end(&mut self, span: Span) {
        self.record("onObjectEnd", span, None, None);
    }

    fn on_array_begin(&mut self, span: Span, path: &[Segment]) -> bool {
        self.record("onArrayBegin", span, None, Some(path));
        !self.stop_offsets.contains(&span.offset)
    }

    fn on_array_end(&mut self, span: Span) {
        self.record("onArrayEnd", span, None, None);
    }

    fn on_literal_value(&mut self, value: Literal, span: Span, path: &[Segment]) {
        self.record("onLiteralValue", span, Some(value.to_json()), Some(path));
    }

    fn on_separator(&mut self, separator: char, span: Span) {
        self.record("onSeparator", span, Some(json!(separator)), None);
    }

    fn on_comment(&mut self, span: Span) {
        self.record("onComment", span, None, None);
    }

    fn on_error(&mut self, error: ParseErrorCode, span: Span) {
        self.errors.push(error_json(error, span));
    }
}

fn assert_visit(
    input: &str,
    expected: &str,
    expected_errors: &str,
    disallow_comments: bool,
    stop_offsets: &[usize],
) {
    let mut recorder = Recorder {
        input,
        stop_offsets,
        calls: Vec::new(),
        errors: Vec::new(),
    };
    let options = ParseOptions {
        disallow_comments,
        ..Default::default()
    };
    visit(input, &mut recorder, options);
    assert_eq!(
        Value::from(recorder.errors),
        from_json(expected_errors),
        "{input:?}"
    );
    assert_eq!(
        Value::from(recorder.calls),
        from_json(expected),
        "{input:?}"
    );
}

fn assert_node_at_location(input: &str, segments: &[Segment], expected: Option<&str>) {
    let tree = parse_tree(input, ParseOptions::default()).0.unwrap();
    let actual = tree.find_node_at_location(tree.root(), segments);
    assert_eq!(
        actual.map(|id| tree.node_value(id)),
        expected.map(from_json),
        "{segments:?}"
    );
    if let Some(id) = actual {
        assert_eq!(tree.node_path(id), segments);
    }
}

fn split_cursor(input: &str) -> (String, usize) {
    let offset = input.find('|').unwrap();
    (input.replacen('|', "", 1), offset)
}

fn assert_location(
    input: &str,
    expected_segments: &[Segment],
    expected_node_type: Option<NodeType>,
    expected_complete_property: bool,
) {
    let (text, offset) = split_cursor(input);
    let actual = get_location(&text, offset);
    assert_eq!(actual.path, expected_segments, "{input:?}");
    assert_eq!(
        actual.previous_node.map(|node| node.node_type),
        expected_node_type,
        "{input:?}"
    );
    assert_eq!(
        actual.is_at_property_key, expected_complete_property,
        "{input:?}"
    );
}

fn assert_matches_location(input: &str, matching_segments: &[Segment], expected: bool) {
    let (text, offset) = split_cursor(input);
    let actual = get_location(&text, offset);
    assert_eq!(actual.matches(matching_segments), expected, "{input:?}");
}

#[test]
fn parse_proto_property() {
    assert_valid_parse(
        r#"{ "__proto__": { "polluted": true } }"#,
        Some(r#"{"__proto__":{"polluted":true}}"#),
        ParseOptions::default(),
    );
}

#[test]
fn tokens() {
    assert_kinds("{", &[SyntaxKind::OpenBraceToken]);
    assert_kinds("}", &[SyntaxKind::CloseBraceToken]);
    assert_kinds("[", &[SyntaxKind::OpenBracketToken]);
    assert_kinds("]", &[SyntaxKind::CloseBracketToken]);
    assert_kinds(":", &[SyntaxKind::ColonToken]);
    assert_kinds(",", &[SyntaxKind::CommaToken]);
}

#[test]
fn comments() {
    assert_kinds("// this is a comment", &[SyntaxKind::LineCommentTrivia]);
    assert_kinds(
        "// this is a comment\n",
        &[SyntaxKind::LineCommentTrivia, SyntaxKind::LineBreakTrivia],
    );
    assert_kinds("/* this is a comment*/", &[SyntaxKind::BlockCommentTrivia]);
    assert_kinds(
        "/* this is a \r\ncomment*/",
        &[SyntaxKind::BlockCommentTrivia],
    );
    assert_kinds(
        "/* this is a \ncomment*/",
        &[SyntaxKind::BlockCommentTrivia],
    );
    assert_scan_error(
        "/* this is a",
        ScanError::UnexpectedEndOfComment,
        &[SyntaxKind::BlockCommentTrivia],
    );
    assert_scan_error(
        "/* this is a \ncomment",
        ScanError::UnexpectedEndOfComment,
        &[SyntaxKind::BlockCommentTrivia],
    );
    assert_kinds(
        "/ ttt",
        &[SyntaxKind::Unknown, SyntaxKind::Trivia, SyntaxKind::Unknown],
    );
}

#[test]
fn strings() {
    assert_kinds("\"test\"", &[SyntaxKind::StringLiteral]);
    assert_kinds("\"\\\"\"", &[SyntaxKind::StringLiteral]);
    assert_kinds("\"\\/\"", &[SyntaxKind::StringLiteral]);
    assert_kinds("\"\\b\"", &[SyntaxKind::StringLiteral]);
    assert_kinds("\"\\f\"", &[SyntaxKind::StringLiteral]);
    assert_kinds("\"\\n\"", &[SyntaxKind::StringLiteral]);
    assert_kinds("\"\\r\"", &[SyntaxKind::StringLiteral]);
    assert_kinds("\"\\t\"", &[SyntaxKind::StringLiteral]);
    assert_kinds("\"\u{88ff}\"", &[SyntaxKind::StringLiteral]);
    assert_kinds("\"\u{200b}\u{2028}\"", &[SyntaxKind::StringLiteral]);
    assert_scan_error(
        "\"\\v\"",
        ScanError::InvalidEscapeCharacter,
        &[SyntaxKind::StringLiteral],
    );
    assert_scan_error(
        "\"test",
        ScanError::UnexpectedEndOfString,
        &[SyntaxKind::StringLiteral],
    );
    assert_scan_error(
        "\"test\n\"",
        ScanError::UnexpectedEndOfString,
        &[
            SyntaxKind::StringLiteral,
            SyntaxKind::LineBreakTrivia,
            SyntaxKind::StringLiteral,
        ],
    );
    assert_scan_error(
        "\"\t\"",
        ScanError::InvalidCharacter,
        &[SyntaxKind::StringLiteral],
    );
    assert_scan_error(
        "\"\t \"",
        ScanError::InvalidCharacter,
        &[SyntaxKind::StringLiteral],
    );
    assert_scan_error(
        "\"\u{0} \"",
        ScanError::InvalidCharacter,
        &[SyntaxKind::StringLiteral],
    );
}

#[test]
fn numbers() {
    assert_kinds("0", &[SyntaxKind::NumericLiteral]);
    assert_kinds("0.1", &[SyntaxKind::NumericLiteral]);
    assert_kinds("-0.1", &[SyntaxKind::NumericLiteral]);
    assert_kinds("-1", &[SyntaxKind::NumericLiteral]);
    assert_kinds("1", &[SyntaxKind::NumericLiteral]);
    assert_kinds("123456789", &[SyntaxKind::NumericLiteral]);
    assert_kinds("10", &[SyntaxKind::NumericLiteral]);
    assert_kinds("90", &[SyntaxKind::NumericLiteral]);
    assert_kinds("90E+123", &[SyntaxKind::NumericLiteral]);
    assert_kinds("90e+123", &[SyntaxKind::NumericLiteral]);
    assert_kinds("90e-123", &[SyntaxKind::NumericLiteral]);
    assert_kinds("90E-123", &[SyntaxKind::NumericLiteral]);
    assert_kinds("90E123", &[SyntaxKind::NumericLiteral]);
    assert_kinds("90e123", &[SyntaxKind::NumericLiteral]);
    assert_kinds(
        "01",
        &[SyntaxKind::NumericLiteral, SyntaxKind::NumericLiteral],
    );
    assert_kinds(
        "-01",
        &[SyntaxKind::NumericLiteral, SyntaxKind::NumericLiteral],
    );
    assert_kinds("-", &[SyntaxKind::Unknown]);
    assert_kinds(".0", &[SyntaxKind::Unknown]);
}

#[test]
fn keywords_true_false_null() {
    assert_kinds("true", &[SyntaxKind::TrueKeyword]);
    assert_kinds("false", &[SyntaxKind::FalseKeyword]);
    assert_kinds("null", &[SyntaxKind::NullKeyword]);
    assert_kinds(
        "true false null",
        &[
            SyntaxKind::TrueKeyword,
            SyntaxKind::Trivia,
            SyntaxKind::FalseKeyword,
            SyntaxKind::Trivia,
            SyntaxKind::NullKeyword,
        ],
    );
    assert_kinds("nulllll", &[SyntaxKind::Unknown]);
    assert_kinds("True", &[SyntaxKind::Unknown]);
    assert_kinds("foo-bar", &[SyntaxKind::Unknown]);
    assert_kinds(
        "foo bar",
        &[SyntaxKind::Unknown, SyntaxKind::Trivia, SyntaxKind::Unknown],
    );
    assert_kinds(
        "false//hello",
        &[SyntaxKind::FalseKeyword, SyntaxKind::LineCommentTrivia],
    );
}

#[test]
fn trivia() {
    assert_kinds(" ", &[SyntaxKind::Trivia]);
    assert_kinds("  \t  ", &[SyntaxKind::Trivia]);
    assert_kinds(
        "  \t  \n  \t  ",
        &[
            SyntaxKind::Trivia,
            SyntaxKind::LineBreakTrivia,
            SyntaxKind::Trivia,
        ],
    );
    assert_kinds("\r\n", &[SyntaxKind::LineBreakTrivia]);
    assert_kinds("\r", &[SyntaxKind::LineBreakTrivia]);
    assert_kinds("\n", &[SyntaxKind::LineBreakTrivia]);
    assert_kinds(
        "\n\r",
        &[SyntaxKind::LineBreakTrivia, SyntaxKind::LineBreakTrivia],
    );
    assert_kinds(
        "\n   \n",
        &[
            SyntaxKind::LineBreakTrivia,
            SyntaxKind::Trivia,
            SyntaxKind::LineBreakTrivia,
        ],
    );
}

#[test]
fn parse_literals() {
    assert_valid_parse("true", Some(r#"true"#), ParseOptions::default());
    assert_valid_parse("false", Some(r#"false"#), ParseOptions::default());
    assert_valid_parse("null", Some(r#"null"#), ParseOptions::default());
    assert_valid_parse("\"foo\"", Some(r#""foo""#), ParseOptions::default());
    assert_valid_parse(
        "\"\\\"-\\\\-\\/-\\b-\\f-\\n-\\r-\\t\"",
        Some(r#""\"-\\-/-\b-\f-\n-\r-\t""#),
        ParseOptions::default(),
    );
    assert_valid_parse("\"\\u00DC\"", Some(r#""Ü""#), ParseOptions::default());
    assert_valid_parse("9", Some(r#"9"#), ParseOptions::default());
    assert_valid_parse("-9", Some(r#"-9"#), ParseOptions::default());
    assert_valid_parse("0.129", Some(r#"0.129"#), ParseOptions::default());
    assert_valid_parse("23e3", Some(r#"23000"#), ParseOptions::default());
    assert_valid_parse("1.2E+3", Some(r#"1200"#), ParseOptions::default());
    assert_valid_parse("1.2E-3", Some(r#"0.0012"#), ParseOptions::default());
    assert_valid_parse(
        "1.2E-3 // comment",
        Some(r#"0.0012"#),
        ParseOptions::default(),
    );
}

#[test]
fn parse_objects() {
    assert_valid_parse("{}", Some(r#"{}"#), ParseOptions::default());
    assert_valid_parse(
        "{ \"foo\": true }",
        Some(r#"{"foo":true}"#),
        ParseOptions::default(),
    );
    assert_valid_parse(
        "{ \"bar\": 8, \"xoo\": \"foo\" }",
        Some(r#"{"bar":8,"xoo":"foo"}"#),
        ParseOptions::default(),
    );
    assert_valid_parse(
        "{ \"hello\": [], \"world\": {} }",
        Some(r#"{"hello":[],"world":{}}"#),
        ParseOptions::default(),
    );
    assert_valid_parse(
        "{ \"a\": false, \"b\": true, \"c\": [ 7.4 ] }",
        Some(r#"{"a":false,"b":true,"c":[7.4]}"#),
        ParseOptions::default(),
    );
    assert_valid_parse(
        "{ \"lineComment\": \"//\", \"blockComment\": [\"/*\", \"*/\"], \"brackets\": [ [\"{\", \"}\"], [\"[\", \"]\"], [\"(\", \")\"] ] }",
        Some(
            r#"{"lineComment":"//","blockComment":["/*","*/"],"brackets":[["{","}"],["[","]"],["(",")"]]}"#,
        ),
        ParseOptions::default(),
    );
    assert_valid_parse(
        "{ \"hello\": [], \"world\": {} }",
        Some(r#"{"hello":[],"world":{}}"#),
        ParseOptions::default(),
    );
    assert_valid_parse(
        "{ \"hello\": { \"again\": { \"inside\": 5 }, \"world\": 1 }}",
        Some(r#"{"hello":{"again":{"inside":5},"world":1}}"#),
        ParseOptions::default(),
    );
    assert_valid_parse(
        "{ \"foo\": /*hello*/true }",
        Some(r#"{"foo":true}"#),
        ParseOptions::default(),
    );
    assert_valid_parse(
        "{ \"\": true }",
        Some(r#"{"":true}"#),
        ParseOptions::default(),
    );
}

// Not ported: "parse: __proto__ own property is preserved without polluting prototype" (ReferenceError: parse is not defined)

#[test]
fn parse_arrays() {
    assert_valid_parse("[]", Some(r#"[]"#), ParseOptions::default());
    assert_valid_parse(
        "[ [],  [ [] ]]",
        Some(r#"[[],[[]]]"#),
        ParseOptions::default(),
    );
    assert_valid_parse("[ 1, 2, 3 ]", Some(r#"[1,2,3]"#), ParseOptions::default());
    assert_valid_parse(
        "[ { \"a\": null } ]",
        Some(r#"[{"a":null}]"#),
        ParseOptions::default(),
    );
}

#[test]
fn parse_objects_with_errors() {
    assert_invalid_parse("{,}", Some(r#"{}"#), ParseOptions::default());
    assert_invalid_parse(
        "{ \"foo\": true, }",
        Some(r#"{"foo":true}"#),
        ParseOptions::default(),
    );
    assert_invalid_parse(
        "{ \"bar\": 8 \"xoo\": \"foo\" }",
        Some(r#"{"bar":8,"xoo":"foo"}"#),
        ParseOptions::default(),
    );
    assert_invalid_parse(
        "{ ,\"bar\": 8 }",
        Some(r#"{"bar":8}"#),
        ParseOptions::default(),
    );
    assert_invalid_parse(
        "{ ,\"bar\": 8, \"foo\" }",
        Some(r#"{"bar":8}"#),
        ParseOptions::default(),
    );
    assert_invalid_parse(
        "{ \"bar\": 8, \"foo\": }",
        Some(r#"{"bar":8}"#),
        ParseOptions::default(),
    );
    assert_invalid_parse(
        "{ 8, \"foo\": 9 }",
        Some(r#"{"foo":9}"#),
        ParseOptions::default(),
    );
}

#[test]
fn parse_array_with_errors() {
    assert_invalid_parse("[,]", Some(r#"[]"#), ParseOptions::default());
    assert_invalid_parse("[ 1 2, 3 ]", Some(r#"[1,2,3]"#), ParseOptions::default());
    assert_invalid_parse("[ ,1, 2, 3 ]", Some(r#"[1,2,3]"#), ParseOptions::default());
    assert_invalid_parse("[ ,1, 2, 3, ]", Some(r#"[1,2,3]"#), ParseOptions::default());
}

#[test]
fn parse_errors() {
    assert_invalid_parse("", None, ParseOptions::default());
    assert_invalid_parse("1,1", Some(r#"1"#), ParseOptions::default());
}

#[test]
fn parse_disallow_comments() {
    assert_valid_parse(
        "[ 1, 2, null, \"foo\" ]",
        Some(r#"[1,2,null,"foo"]"#),
        ParseOptions {
            disallow_comments: true,
            ..Default::default()
        },
    );
    assert_valid_parse(
        "{ \"hello\": [], \"world\": {} }",
        Some(r#"{"hello":[],"world":{}}"#),
        ParseOptions {
            disallow_comments: true,
            ..Default::default()
        },
    );
    assert_invalid_parse(
        "{ \"foo\": /*comment*/ true }",
        Some(r#"{"foo":true}"#),
        ParseOptions {
            disallow_comments: true,
            ..Default::default()
        },
    );
}

#[test]
fn parse_trailing_comma() {
    assert_valid_parse(
        "{ \"hello\": [], }",
        Some(r#"{"hello":[]}"#),
        ParseOptions {
            allow_trailing_comma: true,
            ..Default::default()
        },
    );
    assert_valid_parse(
        "{ \"hello\": [] }",
        Some(r#"{"hello":[]}"#),
        ParseOptions {
            allow_trailing_comma: true,
            ..Default::default()
        },
    );
    assert_valid_parse(
        "{ \"hello\": [], \"world\": {}, }",
        Some(r#"{"hello":[],"world":{}}"#),
        ParseOptions {
            allow_trailing_comma: true,
            ..Default::default()
        },
    );
    assert_valid_parse(
        "{ \"hello\": [], \"world\": {} }",
        Some(r#"{"hello":[],"world":{}}"#),
        ParseOptions {
            allow_trailing_comma: true,
            ..Default::default()
        },
    );
    assert_valid_parse(
        "[ 1, 2, ]",
        Some(r#"[1,2]"#),
        ParseOptions {
            allow_trailing_comma: true,
            ..Default::default()
        },
    );
    assert_valid_parse(
        "[ 1, 2 ]",
        Some(r#"[1,2]"#),
        ParseOptions {
            allow_trailing_comma: true,
            ..Default::default()
        },
    );
    assert_invalid_parse(
        "{ \"hello\": [], }",
        Some(r#"{"hello":[]}"#),
        ParseOptions::default(),
    );
    assert_invalid_parse(
        "{ \"hello\": [], \"world\": {}, }",
        Some(r#"{"hello":[],"world":{}}"#),
        ParseOptions::default(),
    );
    assert_invalid_parse("[ 1, 2, ]", Some(r#"[1,2]"#), ParseOptions::default());
}

#[test]
fn location_properties() {
    assert_location("|{ \"foo\": \"bar\" }", &[], None, false);
    assert_location("{| \"foo\": \"bar\" }", &[p("")], None, true);
    assert_location(
        "{ |\"foo\": \"bar\" }",
        &[p("foo")],
        Some(NodeType::Property),
        true,
    );
    assert_location(
        "{ \"foo|\": \"bar\" }",
        &[p("foo")],
        Some(NodeType::Property),
        true,
    );
    assert_location(
        "{ \"foo\"|: \"bar\" }",
        &[p("foo")],
        Some(NodeType::Property),
        true,
    );
    assert_location(
        "{ \"foo\": \"bar\"| }",
        &[p("foo")],
        Some(NodeType::String),
        false,
    );
    assert_location("{ \"foo\":| \"bar\" }", &[p("foo")], None, false);
    assert_location(
        "{ \"foo\": {\"bar|\": 1, \"car\": 2 } }",
        &[p("foo"), p("bar")],
        Some(NodeType::Property),
        true,
    );
    assert_location(
        "{ \"foo\": {\"bar\": 1|, \"car\": 3 } }",
        &[p("foo"), p("bar")],
        Some(NodeType::Number),
        false,
    );
    assert_location(
        "{ \"foo\": {\"bar\": 1,| \"car\": 4 } }",
        &[p("foo"), p("")],
        None,
        true,
    );
    assert_location(
        "{ \"foo\": {\"bar\": 1, \"ca|r\": 5 } }",
        &[p("foo"), p("car")],
        Some(NodeType::Property),
        true,
    );
    assert_location(
        "{ \"foo\": {\"bar\": 1, \"car\": 6| } }",
        &[p("foo"), p("car")],
        Some(NodeType::Number),
        false,
    );
    assert_location(
        "{ \"foo\": {\"bar\": 1, \"car\": 7 }| }",
        &[p("foo")],
        None,
        false,
    );
    assert_location(
        "{ \"foo\": {\"bar\": 1, \"car\": 8 },| \"goo\": {} }",
        &[p("")],
        None,
        true,
    );
    assert_location(
        "{ \"foo\": {\"bar\": 1, \"car\": 9 }, \"go|o\": {} }",
        &[p("goo")],
        Some(NodeType::Property),
        true,
    );
    assert_location(
        "{ \"dep\": {\"bar\": 1, \"car\": |",
        &[p("dep"), p("car")],
        None,
        false,
    );
    assert_location(
        "{ \"dep\": {\"bar\": 1,, \"car\": |",
        &[p("dep"), p("car")],
        None,
        false,
    );
    assert_location(
        "{ \"dep\": {\"bar\": \"na\", \"dar\": \"ma\", \"car\": | } }",
        &[p("dep"), p("car")],
        None,
        false,
    );
}

#[test]
fn location_arrays() {
    assert_location("|[\"foo\", null ]", &[], None, false);
    assert_location("[|\"foo\", null ]", &[i(0)], Some(NodeType::String), false);
    assert_location("[\"foo\"|, null ]", &[i(0)], Some(NodeType::String), false);
    assert_location("[\"foo\",| null ]", &[i(1)], None, false);
    assert_location("[\"foo\", |null ]", &[i(1)], Some(NodeType::Null), false);
    assert_location("[\"foo\", null,| ]", &[i(2)], None, false);
    assert_location("[\"foo\", null,,| ]", &[i(3)], None, false);
    assert_location("[[\"foo\", null,, ],|", &[i(1)], None, false);
}

#[test]
fn tree_literals() {
    assert_tree(
        "true",
        r#"{"type":"boolean","offset":0,"length":4,"value":true}"#,
        r#"[]"#,
    );
    assert_tree(
        "false",
        r#"{"type":"boolean","offset":0,"length":5,"value":false}"#,
        r#"[]"#,
    );
    assert_tree(
        "null",
        r#"{"type":"null","offset":0,"length":4,"value":null}"#,
        r#"[]"#,
    );
    assert_tree(
        "23",
        r#"{"type":"number","offset":0,"length":2,"value":23}"#,
        r#"[]"#,
    );
    assert_tree(
        "-1.93e-19",
        r#"{"type":"number","offset":0,"length":9,"value":-1.93e-19}"#,
        r#"[]"#,
    );
    assert_tree(
        "\"hello\"",
        r#"{"type":"string","offset":0,"length":7,"value":"hello"}"#,
        r#"[]"#,
    );
}

#[test]
fn tree_arrays() {
    assert_tree(
        "[]",
        r#"{"type":"array","offset":0,"length":2,"children":[]}"#,
        r#"[]"#,
    );
    assert_tree(
        "[ 1 ]",
        r#"{"type":"array","offset":0,"length":5,"children":[{"type":"number","offset":2,"length":1,"value":1}]}"#,
        r#"[]"#,
    );
    assert_tree(
        "[ 1,\"x\"]",
        r#"{"type":"array","offset":0,"length":8,"children":[{"type":"number","offset":2,"length":1,"value":1},{"type":"string","offset":4,"length":3,"value":"x"}]}"#,
        r#"[]"#,
    );
    assert_tree(
        "[[]]",
        r#"{"type":"array","offset":0,"length":4,"children":[{"type":"array","offset":1,"length":2,"children":[]}]}"#,
        r#"[]"#,
    );
}

#[test]
fn tree_objects() {
    assert_tree(
        "{ }",
        r#"{"type":"object","offset":0,"length":3,"children":[]}"#,
        r#"[]"#,
    );
    assert_tree(
        "{ \"val\": 1 }",
        r#"{"type":"object","offset":0,"length":12,"children":[{"type":"property","offset":2,"length":8,"colonOffset":7,"children":[{"type":"string","offset":2,"length":5,"value":"val"},{"type":"number","offset":9,"length":1,"value":1}]}]}"#,
        r#"[]"#,
    );
    assert_tree(
        "{\"id\": \"$\", \"v\": [ null, null] }",
        r#"{"type":"object","offset":0,"length":32,"children":[{"type":"property","offset":1,"length":9,"colonOffset":5,"children":[{"type":"string","offset":1,"length":4,"value":"id"},{"type":"string","offset":7,"length":3,"value":"$"}]},{"type":"property","offset":12,"length":18,"colonOffset":15,"children":[{"type":"string","offset":12,"length":3,"value":"v"},{"type":"array","offset":17,"length":13,"children":[{"type":"null","offset":19,"length":4,"value":null},{"type":"null","offset":25,"length":4,"value":null}]}]}]}"#,
        r#"[]"#,
    );
    assert_tree(
        "{  \"id\": { \"foo\": { } } , }",
        r#"{"type":"object","offset":0,"length":27,"children":[{"type":"property","offset":3,"length":20,"colonOffset":7,"children":[{"type":"string","offset":3,"length":4,"value":"id"},{"type":"object","offset":9,"length":14,"children":[{"type":"property","offset":11,"length":10,"colonOffset":16,"children":[{"type":"string","offset":11,"length":5,"value":"foo"},{"type":"object","offset":18,"length":3,"children":[]}]}]}]}]}"#,
        r#"[{"error":3,"offset":26,"length":1,"startLine":0,"startCharacter":26},{"error":4,"offset":26,"length":1,"startLine":0,"startCharacter":26}]"#,
    );
}

#[test]
fn visit_object() {
    assert_visit(
        "{ }",
        r#"[{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":0,"path":[]},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":2}]"#,
        r#"[]"#,
        false,
        &[],
    );
    assert_visit(
        "{ \"foo\": \"bar\" }",
        r#"[{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":0,"path":[]},{"id":"onObjectProperty","text":"\"foo\"","startLine":0,"startCharacter":2,"arg":"foo","path":[]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":7,"arg":":"},{"id":"onLiteralValue","text":"\"bar\"","startLine":0,"startCharacter":9,"arg":"bar","path":["foo"]},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":15}]"#,
        r#"[]"#,
        false,
        &[],
    );
    assert_visit(
        "{ \"foo\": { \"goo\": 3 } }",
        r#"[{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":0,"path":[]},{"id":"onObjectProperty","text":"\"foo\"","startLine":0,"startCharacter":2,"arg":"foo","path":[]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":7,"arg":":"},{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":9,"path":["foo"]},{"id":"onObjectProperty","text":"\"goo\"","startLine":0,"startCharacter":11,"arg":"goo","path":["foo"]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":16,"arg":":"},{"id":"onLiteralValue","text":"3","startLine":0,"startCharacter":18,"arg":3,"path":["foo","goo"]},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":20},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":22}]"#,
        r#"[]"#,
        false,
        &[],
    );
    assert_visit(
        "{ \"foo\": \"bar\", \"a\": {\"b\": \"c\"} }",
        r#"[{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":0,"path":[]},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":32}]"#,
        r#"[]"#,
        false,
        &[0],
    );
    assert_visit(
        "{ \"a\": { \"b\": \"c\", \"d\": { \"e\": \"f\" } } }",
        r#"[{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":0,"path":[]},{"id":"onObjectProperty","text":"\"a\"","startLine":0,"startCharacter":2,"arg":"a","path":[]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":5,"arg":":"},{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":7,"path":["a"]},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":37},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":39}]"#,
        r#"[]"#,
        true,
        &[7],
    );
}

#[test]
fn visit_array() {
    assert_visit(
        "[]",
        r#"[{"id":"onArrayBegin","text":"[","startLine":0,"startCharacter":0,"path":[]},{"id":"onArrayEnd","text":"]","startLine":0,"startCharacter":1}]"#,
        r#"[]"#,
        false,
        &[],
    );
    assert_visit(
        "[ true, null, [] ]",
        r#"[{"id":"onArrayBegin","text":"[","startLine":0,"startCharacter":0,"path":[]},{"id":"onLiteralValue","text":"true","startLine":0,"startCharacter":2,"arg":true,"path":[0]},{"id":"onSeparator","text":",","startLine":0,"startCharacter":6,"arg":","},{"id":"onLiteralValue","text":"null","startLine":0,"startCharacter":8,"arg":null,"path":[1]},{"id":"onSeparator","text":",","startLine":0,"startCharacter":12,"arg":","},{"id":"onArrayBegin","text":"[","startLine":0,"startCharacter":14,"path":[2]},{"id":"onArrayEnd","text":"]","startLine":0,"startCharacter":15},{"id":"onArrayEnd","text":"]","startLine":0,"startCharacter":17}]"#,
        r#"[]"#,
        false,
        &[],
    );
    assert_visit(
        "[\r\n0,\r\n1,\r\n2\r\n]",
        r#"[{"id":"onArrayBegin","text":"[","startLine":0,"startCharacter":0,"path":[]},{"id":"onLiteralValue","text":"0","startLine":1,"startCharacter":0,"arg":0,"path":[0]},{"id":"onSeparator","text":",","startLine":1,"startCharacter":1,"arg":","},{"id":"onLiteralValue","text":"1","startLine":2,"startCharacter":0,"arg":1,"path":[1]},{"id":"onSeparator","text":",","startLine":2,"startCharacter":1,"arg":","},{"id":"onLiteralValue","text":"2","startLine":3,"startCharacter":0,"arg":2,"path":[2]},{"id":"onArrayEnd","text":"]","startLine":4,"startCharacter":0}]"#,
        r#"[]"#,
        false,
        &[],
    );
}

#[test]
fn visit_object_array() {
    assert_visit(
        "[ { \"p1\": [ { \"p11\": 1, \"p12\": [ true, [ false, 2 ] ] } ] } ]",
        r#"[{"id":"onArrayBegin","text":"[","startLine":0,"startCharacter":0,"path":[]},{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":2,"path":[0]},{"id":"onObjectProperty","text":"\"p1\"","startLine":0,"startCharacter":4,"arg":"p1","path":[0]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":8,"arg":":"},{"id":"onArrayBegin","text":"[","startLine":0,"startCharacter":10,"path":[0,"p1"]},{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":12,"path":[0,"p1",0]},{"id":"onObjectProperty","text":"\"p11\"","startLine":0,"startCharacter":14,"arg":"p11","path":[0,"p1",0]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":19,"arg":":"},{"id":"onLiteralValue","text":"1","startLine":0,"startCharacter":21,"arg":1,"path":[0,"p1",0,"p11"]},{"id":"onSeparator","text":",","startLine":0,"startCharacter":22,"arg":","},{"id":"onObjectProperty","text":"\"p12\"","startLine":0,"startCharacter":24,"arg":"p12","path":[0,"p1",0]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":29,"arg":":"},{"id":"onArrayBegin","text":"[","startLine":0,"startCharacter":31,"path":[0,"p1",0,"p12"]},{"id":"onLiteralValue","text":"true","startLine":0,"startCharacter":33,"arg":true,"path":[0,"p1",0,"p12",0]},{"id":"onSeparator","text":",","startLine":0,"startCharacter":37,"arg":","},{"id":"onArrayBegin","text":"[","startLine":0,"startCharacter":39,"path":[0,"p1",0,"p12",1]},{"id":"onLiteralValue","text":"false","startLine":0,"startCharacter":41,"arg":false,"path":[0,"p1",0,"p12",1,0]},{"id":"onSeparator","text":",","startLine":0,"startCharacter":46,"arg":","},{"id":"onLiteralValue","text":"2","startLine":0,"startCharacter":48,"arg":2,"path":[0,"p1",0,"p12",1,1]},{"id":"onArrayEnd","text":"]","startLine":0,"startCharacter":50},{"id":"onArrayEnd","text":"]","startLine":0,"startCharacter":52},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":54},{"id":"onArrayEnd","text":"]","startLine":0,"startCharacter":56},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":58},{"id":"onArrayEnd","text":"]","startLine":0,"startCharacter":60}]"#,
        r#"[]"#,
        false,
        &[],
    );
    assert_visit(
        "{ \"foo\": [ { \"a\": \"b\", \"c:\": \"d\", \"d\": { \"e\": \"f\" } } ] }",
        r#"[{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":0,"path":[]},{"id":"onObjectProperty","text":"\"foo\"","startLine":0,"startCharacter":2,"arg":"foo","path":[]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":7,"arg":":"},{"id":"onArrayBegin","text":"[","startLine":0,"startCharacter":9,"path":["foo"]},{"id":"onArrayEnd","text":"]","startLine":0,"startCharacter":54},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":56}]"#,
        r#"[]"#,
        true,
        &[9],
    );
}

#[test]
fn visit_comment() {
    assert_visit(
        "/* g */ { \"foo\": //f\n\"bar\" }",
        r#"[{"id":"onComment","text":"/* g */","startLine":0,"startCharacter":0},{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":8,"path":[]},{"id":"onObjectProperty","text":"\"foo\"","startLine":0,"startCharacter":10,"arg":"foo","path":[]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":15,"arg":":"},{"id":"onComment","text":"//f","startLine":0,"startCharacter":17},{"id":"onLiteralValue","text":"\"bar\"","startLine":1,"startCharacter":0,"arg":"bar","path":["foo"]},{"id":"onObjectEnd","text":"}","startLine":1,"startCharacter":6}]"#,
        r#"[]"#,
        false,
        &[],
    );
    assert_visit(
        "/* g\r\n */ { \"foo\": //f\n\"bar\" }",
        r#"[{"id":"onComment","text":"/* g\r\n */","startLine":0,"startCharacter":0},{"id":"onObjectBegin","text":"{","startLine":1,"startCharacter":4,"path":[]},{"id":"onObjectProperty","text":"\"foo\"","startLine":1,"startCharacter":6,"arg":"foo","path":[]},{"id":"onSeparator","text":":","startLine":1,"startCharacter":11,"arg":":"},{"id":"onComment","text":"//f","startLine":1,"startCharacter":13},{"id":"onLiteralValue","text":"\"bar\"","startLine":2,"startCharacter":0,"arg":"bar","path":["foo"]},{"id":"onObjectEnd","text":"}","startLine":2,"startCharacter":6}]"#,
        r#"[]"#,
        false,
        &[],
    );
    assert_visit(
        "/* g\n */ { \"foo\": //f\n\"bar\"\n}",
        r#"[{"id":"onObjectBegin","text":"{","startLine":1,"startCharacter":4,"path":[]},{"id":"onObjectProperty","text":"\"foo\"","startLine":1,"startCharacter":6,"arg":"foo","path":[]},{"id":"onSeparator","text":":","startLine":1,"startCharacter":11,"arg":":"},{"id":"onLiteralValue","text":"\"bar\"","startLine":2,"startCharacter":0,"arg":"bar","path":["foo"]},{"id":"onObjectEnd","text":"}","startLine":3,"startCharacter":0}]"#,
        r#"[{"error":10,"offset":0,"length":8,"startLine":0,"startCharacter":0},{"error":10,"offset":18,"length":3,"startLine":1,"startCharacter":13}]"#,
        true,
        &[],
    );
}

#[test]
fn visit_malformed() {
    assert_visit(
        "[ { \"a\", \"b\": [] 1, \"c\" [ 1 2 ], true: \"d\":, \"e\": }, 2, 3 4 ]",
        r#"[{"id":"onArrayBegin","text":"[","startLine":0,"startCharacter":0,"path":[]},{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":2,"path":[0]},{"id":"onObjectProperty","text":"\"a\"","startLine":0,"startCharacter":4,"arg":"a","path":[0]},{"id":"onSeparator","text":",","startLine":0,"startCharacter":7,"arg":","},{"id":"onObjectProperty","text":"\"b\"","startLine":0,"startCharacter":9,"arg":"b","path":[0]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":12,"arg":":"},{"id":"onArrayBegin","text":"[","startLine":0,"startCharacter":14,"path":[0,"b"]},{"id":"onArrayEnd","text":"]","startLine":0,"startCharacter":15},{"id":"onSeparator","text":",","startLine":0,"startCharacter":18,"arg":","},{"id":"onObjectProperty","text":"\"c\"","startLine":0,"startCharacter":20,"arg":"c","path":[0]},{"id":"onSeparator","text":",","startLine":0,"startCharacter":31,"arg":","},{"id":"onSeparator","text":",","startLine":0,"startCharacter":43,"arg":","},{"id":"onObjectProperty","text":"\"e\"","startLine":0,"startCharacter":45,"arg":"e","path":[0]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":48,"arg":":"},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":50},{"id":"onSeparator","text":",","startLine":0,"startCharacter":51,"arg":","},{"id":"onLiteralValue","text":"2","startLine":0,"startCharacter":53,"arg":2,"path":[1]},{"id":"onSeparator","text":",","startLine":0,"startCharacter":54,"arg":","},{"id":"onLiteralValue","text":"3","startLine":0,"startCharacter":56,"arg":3,"path":[2]},{"id":"onLiteralValue","text":"4","startLine":0,"startCharacter":58,"arg":4,"path":[3]},{"id":"onArrayEnd","text":"]","startLine":0,"startCharacter":60}]"#,
        r#"[{"error":5,"offset":7,"length":1,"startLine":0,"startCharacter":7},{"error":6,"offset":17,"length":1,"startLine":0,"startCharacter":17},{"error":3,"offset":17,"length":1,"startLine":0,"startCharacter":17},{"error":4,"offset":18,"length":1,"startLine":0,"startCharacter":18},{"error":5,"offset":24,"length":1,"startLine":0,"startCharacter":24},{"error":3,"offset":33,"length":4,"startLine":0,"startCharacter":33},{"error":4,"offset":43,"length":1,"startLine":0,"startCharacter":43},{"error":4,"offset":50,"length":1,"startLine":0,"startCharacter":50},{"error":6,"offset":58,"length":1,"startLine":0,"startCharacter":58}]"#,
        false,
        &[],
    );
}

#[test]
fn visit_incomplete() {
    assert_visit(
        "{\"prop1\":\"foo\",\"prop2\":\"foo2\",\"prop3\":{\"prp1\":{\"\"}}}",
        r#"[{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":0,"path":[]},{"id":"onObjectProperty","text":"\"prop1\"","startLine":0,"startCharacter":1,"arg":"prop1","path":[]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":8,"arg":":"},{"id":"onLiteralValue","text":"\"foo\"","startLine":0,"startCharacter":9,"arg":"foo","path":["prop1"]},{"id":"onSeparator","text":",","startLine":0,"startCharacter":14,"arg":","},{"id":"onObjectProperty","text":"\"prop2\"","startLine":0,"startCharacter":15,"arg":"prop2","path":[]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":22,"arg":":"},{"id":"onLiteralValue","text":"\"foo2\"","startLine":0,"startCharacter":23,"arg":"foo2","path":["prop2"]},{"id":"onSeparator","text":",","startLine":0,"startCharacter":29,"arg":","},{"id":"onObjectProperty","text":"\"prop3\"","startLine":0,"startCharacter":30,"arg":"prop3","path":[]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":37,"arg":":"},{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":38,"path":["prop3"]},{"id":"onObjectProperty","text":"\"prp1\"","startLine":0,"startCharacter":39,"arg":"prp1","path":["prop3"]},{"id":"onSeparator","text":":","startLine":0,"startCharacter":45,"arg":":"},{"id":"onObjectBegin","text":"{","startLine":0,"startCharacter":46,"path":["prop3","prp1"]},{"id":"onObjectProperty","text":"\"\"","startLine":0,"startCharacter":47,"arg":"","path":["prop3","prp1"]},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":49},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":50},{"id":"onObjectEnd","text":"}","startLine":0,"startCharacter":51}]"#,
        r#"[{"error":5,"offset":49,"length":1,"startLine":0,"startCharacter":49}]"#,
        false,
        &[],
    );
    assert_tree(
        "{\"prop1\":\"foo\",\"prop2\":\"foo2\",\"prop3\":{\"prp1\":{\"\"}}}",
        r#"{"type":"object","offset":0,"length":52,"children":[{"type":"property","offset":1,"length":13,"children":[{"type":"string","value":"prop1","offset":1,"length":7},{"type":"string","offset":9,"length":5,"value":"foo"}],"colonOffset":8},{"type":"property","offset":15,"length":14,"children":[{"type":"string","value":"prop2","offset":15,"length":7},{"type":"string","offset":23,"length":6,"value":"foo2"}],"colonOffset":22},{"type":"property","offset":30,"length":21,"children":[{"type":"string","value":"prop3","offset":30,"length":7},{"type":"object","offset":38,"length":13,"children":[{"type":"property","offset":39,"length":11,"children":[{"type":"string","value":"prp1","offset":39,"length":6},{"type":"object","offset":46,"length":4,"children":[{"type":"property","offset":47,"length":3,"children":[{"type":"string","value":"","offset":47,"length":2}]}]}],"colonOffset":45}]}],"colonOffset":37}]}"#,
        r#"[{"error":5,"offset":49,"length":1,"startLine":0,"startCharacter":49}]"#,
    );
}

#[test]
fn tree_find_location() {
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key1")],
        Some(r#"{"key11":["val111","val112"]}"#),
    );
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key1"), p("key11")],
        Some(r#"["val111","val112"]"#),
    );
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key1"), p("key11"), i(0)],
        Some(r#""val111""#),
    );
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key1"), p("key11"), i(1)],
        Some(r#""val112""#),
    );
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key1"), p("key11"), i(2)],
        None,
    );
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key2"), i(0), p("key21")],
        Some(r#"false"#),
    );
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key2"), i(0), p("key22")],
        Some(r#"221"#),
    );
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key2"), i(1)],
        Some(r#"null"#),
    );
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key2"), i(2)],
        Some(r#"[{}]"#),
    );
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key2"), i(2), i(0)],
        Some(r#"{}"#),
    );
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key3"), p("key31"), p("key311")],
        None,
    );
    assert_node_at_location(
        "{ \"key1\": { \"key11\": [ \"val111\", \"val112\" ] }, \"key2\": [ { \"key21\": false, \"key22\": 221 }, null, [{}] ], \"key3\": { \"key31\":, \"key32\": 32 } }",
        &[p("key3"), p("key32")],
        Some(r#"32"#),
    );
}

#[test]
fn location_matches() {
    assert_matches_location("{ \"dependencies\": { | } }", &[p("dependencies")], true);
    assert_matches_location(
        "{ \"dependencies\": { \"fo| } }",
        &[p("dependencies")],
        true,
    );
    assert_matches_location(
        "{ \"dependencies\": { \"fo|\" } }",
        &[p("dependencies")],
        true,
    );
    assert_matches_location(
        "{ \"dependencies\": { \"fo|\": 1 } }",
        &[p("dependencies")],
        true,
    );
    assert_matches_location(
        "{ \"dependencies\": { \"fo|\": 1 } }",
        &[p("dependencies")],
        true,
    );
    assert_matches_location(
        "{ \"dependencies\": { \"fo\": | } }",
        &[p("dependencies"), p("*")],
        true,
    );
}

/// Not in upstream (`docs/experimental.md`): a value deeper than `MAX_DEPTH` is
/// an empty array or object with an error.
#[test]
fn nesting_limit() {
    let depth = jsonc::MAX_DEPTH + 2;
    let text = "[{\"a\":".repeat(depth) + "1" + &"}]".repeat(depth);
    let (tree, errors) = parse_tree(&text, ParseOptions::default());
    let tree = tree.unwrap();
    let [error] = errors.as_slice() else {
        panic!("{errors:?}");
    };
    assert_eq!(error.error, ParseErrorCode::NestingTooDeep);
    assert_eq!(error.offset, 6 * jsonc::MAX_DEPTH / 2);
    let mut node = tree.root();
    let mut nesting = 1;
    while let Some(&child) = tree[node].children.last() {
        if tree[child].node_type != NodeType::Property {
            nesting += 1;
        }
        node = child;
    }
    assert_eq!(nesting, jsonc::MAX_DEPTH + 1);
    assert_eq!(tree[node].node_type, NodeType::Array);
    assert_eq!(tree[node].end(), text.len() - jsonc::MAX_DEPTH);
}
