//! Port of `vscode-json-languageservice/src/test/formatter.test.ts`.

mod common;

use json_languageservice::{FormattingOptions, format};
use lsp_types::Range;

/// Two `|` characters in `unformatted` mark the range to format.
fn check(unformatted: &str, expected: &str, insert_spaces: bool) {
    let mut text = unformatted.to_owned();
    let mut bounds = None;
    if let (Some(start), Some(end)) = (text.find('|'), text.rfind('|')) {
        text = format!(
            "{}{}{}",
            &text[..start],
            &text[start + 1..end],
            &text[end + 1..]
        );
        bounds = Some((start, end));
    }
    let document = common::document("test://test.json", &text);
    // Same as upstream: the range ends one character after the second `|`.
    let range = bounds
        .map(|(start, end)| Range::new(document.position_at(start), document.position_at(end)));
    let options = FormattingOptions {
        tab_size: 2,
        insert_spaces,
        ..Default::default()
    };
    let edits = format(&document, range, &options);
    assert_eq!(document.apply_edits(&edits), expected);
}

#[test]
fn object_single_property() {
    check("{\"x\" : 1}", "{\n  \"x\": 1\n}", true);
}

#[test]
fn object_multiple_properties() {
    check(
        "{\"x\" : 1,  \"y\" : \"foo\", \"z\"  : true}",
        "{\n  \"x\": 1,\n  \"y\": \"foo\",\n  \"z\": true\n}",
        true,
    );
}

#[test]
fn object_no_properties() {
    check(
        "{\"x\" : {    },  \"y\" : {}}",
        "{\n  \"x\": {},\n  \"y\": {}\n}",
        true,
    );
}

#[test]
fn object_nesting() {
    check(
        "{\"x\" : {  \"y\" : { \"z\"  : { }}, \"a\": true}}",
        "{\n  \"x\": {\n    \"y\": {\n      \"z\": {}\n    },\n    \"a\": true\n  }\n}",
        true,
    );
}

#[test]
fn array_single_items() {
    check("[\"[]\"]", "[\n  \"[]\"\n]", true);
}

#[test]
fn array_multiple_items() {
    check("[true,null,1.2]", "[\n  true,\n  null,\n  1.2\n]", true);
}

#[test]
fn array_no_items() {
    check("[      ]", "[]", true);
}

#[test]
fn array_nesting() {
    check(
        "[ [], [ [ {} ], \"a\" ]  ]",
        "[\n  [],\n  [\n    [\n      {}\n    ],\n    \"a\"\n  ]\n]",
        true,
    );
}

#[test]
fn syntax_errors() {
    check(
        "[ null  1.2 \"Hello\" ]",
        "[\n  null  1.2 \"Hello\"\n]",
        true,
    );
}

#[test]
fn syntax_errors_2() {
    check(
        "{\"a\":\"b\"\"c\":\"d\" }",
        "{\n  \"a\": \"b\"\"c\": \"d\"\n}",
        true,
    );
}

#[test]
fn empty_lines() {
    check(
        "{\n\"a\": true,\n\n\"b\": true\n}",
        "{\n\t\"a\": true,\n\t\"b\": true\n}",
        false,
    );
}

#[test]
fn single_line_comment() {
    check(
        "[ \n//comment\n\"foo\", \"bar\"\n] ",
        "[\n  //comment\n  \"foo\",\n  \"bar\"\n]",
        true,
    );
}

#[test]
fn block_line_comment() {
    check(
        "[{\n        /*comment*/     \n\"foo\" : true\n}] ",
        "[\n  {\n    /*comment*/\n    \"foo\": true\n  }\n]",
        true,
    );
}

#[test]
fn single_line_comment_on_same_line() {
    check(
        " {  \n        \"a\": {}// comment    \n } ",
        "{\n  \"a\": {} // comment    \n}",
        true,
    );
}

#[test]
fn single_line_comment_on_same_line_2() {
    check("{ //comment\n}", "{ //comment\n}", true);
}

#[test]
fn block_comment_on_same_line() {
    check(
        "{      \"a\": {}, /*comment*/    \n        /*comment*/ \"b\": {},    \n        \"c\": {/*comment*/}    } ",
        "{\n  \"a\": {}, /*comment*/\n  /*comment*/ \"b\": {},\n  \"c\": { /*comment*/}\n}",
        true,
    );
}

#[test]
fn block_comment_on_same_line_advanced() {
    check(
        " {       \"d\": [\n             null\n        ] /*comment*/\n        ,\"e\": /*comment*/ [null] }",
        "{\n  \"d\": [\n    null\n  ] /*comment*/,\n  \"e\": /*comment*/ [\n    null\n  ]\n}",
        true,
    );
}

#[test]
fn multiple_block_comments_on_same_line() {
    check(
        "{      \"a\": {} /*comment*/, /*comment*/   \n        /*comment*/ \"b\": {}  /*comment*/  } ",
        "{\n  \"a\": {} /*comment*/, /*comment*/\n  /*comment*/ \"b\": {} /*comment*/\n}",
        true,
    );
}

#[test]
fn multiple_mixed_comments_on_same_line() {
    check(
        "[ /*comment*/  /*comment*/   // comment \n]",
        "[ /*comment*/ /*comment*/ // comment \n]",
        true,
    );
}

#[test]
fn range() {
    check(
        "{ \"a\": {},\n|\"b\": [null, null]|\n} ",
        "{ \"a\": {},\n\"b\": [\n  null,\n  null\n]\n} ",
        true,
    );
}

#[test]
fn range_with_existing_indent() {
    check(
        "{ \"a\": {},\n   |\"b\": [null],\n\"c\": {}\n}|",
        "{ \"a\": {},\n   \"b\": [\n    null\n  ],\n  \"c\": {}\n}",
        true,
    );
}

#[test]
fn range_with_existing_indent_tabs() {
    check(
        "{ \"a\": {},\n|  \"b\": [null],   \n\"c\": {}\n}|    ",
        "{ \"a\": {},\n\t\"b\": [\n\t\tnull\n\t],\n\t\"c\": {}\n}",
        false,
    );
}

#[test]
fn property_range_issue_14623() {
    check(
        "{ |\"a\" :| 1,\n  \"b\": 1\n}",
        "{ \"a\": 1,\n  \"b\": 1\n}",
        false,
    );
}

#[test]
fn block_comment_none_line_breaking_symbols() {
    check(
        "{ \"a\": [ 1\n/* comment */\n, 2\n/* comment */\n]\n/* comment */\n,\n \"b\": true\n/* comment */\n}",
        "{\n  \"a\": [\n    1\n    /* comment */\n    ,\n    2\n    /* comment */\n  ]\n  /* comment */\n  ,\n  \"b\": true\n  /* comment */\n}",
        true,
    );
}

#[test]
fn line_comment_after_none_line_breaking_symbols() {
    check(
        "{ \"a\":\n// comment\nnull,\n \"b\"\n// comment\n: null\n// comment\n}",
        "{\n  \"a\":\n  // comment\n  null,\n  \"b\"\n  // comment\n  : null\n  // comment\n}",
        true,
    );
}

#[test]
fn random_content() {
    check("a 1 b 1 3 true", "a 1 b 1 3 true", true);
}
