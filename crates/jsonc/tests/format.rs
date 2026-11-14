//! Port of `node-jsonc-parser/src/test/format.test.ts`.

use jsonc::{FormattingOptions, Range, apply_edits, format};

/// Two `|` characters in `content` mark the range to format.
fn check(
    content: &str,
    expected: &str,
    insert_spaces: bool,
    insert_final_newline: bool,
    keep_lines: bool,
) {
    let mut content = content.to_owned();
    let mut range = None;
    if let (Some(start), Some(end)) = (content.find('|'), content.rfind('|')) {
        content = format!(
            "{}{}{}",
            &content[..start],
            &content[start + 1..end],
            &content[end + 1..]
        );
        // Same as upstream: the range ends one character after the second `|`.
        range = Some(Range {
            offset: start,
            length: end - start,
        });
    }
    let options = FormattingOptions {
        tab_size: 2,
        insert_spaces,
        insert_final_newline,
        eol: "\n".to_owned(),
        keep_lines,
    };
    let edits = format(&content, range, &options);
    assert_eq!(apply_edits(&content, &edits).unwrap(), expected);
}

#[test]
fn object_single_property() {
    check("{\"x\" : 1}", "{\n  \"x\": 1\n}", true, false, false);
}

#[test]
fn object_multiple_properties() {
    check(
        "{\"x\" : 1,  \"y\" : \"foo\", \"z\"  : true}",
        "{\n  \"x\": 1,\n  \"y\": \"foo\",\n  \"z\": true\n}",
        true,
        false,
        false,
    );
}

#[test]
fn object_no_properties() {
    check(
        "{\"x\" : {    },  \"y\" : {}}",
        "{\n  \"x\": {},\n  \"y\": {}\n}",
        true,
        false,
        false,
    );
}

#[test]
fn object_nesting() {
    check(
        "{\"x\" : {  \"y\" : { \"z\"  : { }}, \"a\": true}}",
        "{\n  \"x\": {\n    \"y\": {\n      \"z\": {}\n    },\n    \"a\": true\n  }\n}",
        true,
        false,
        false,
    );
}

#[test]
fn array_single_items() {
    check("[\"[]\"]", "[\n  \"[]\"\n]", true, false, false);
}

#[test]
fn array_multiple_items() {
    check(
        "[true,null,1.2]",
        "[\n  true,\n  null,\n  1.2\n]",
        true,
        false,
        false,
    );
}

#[test]
fn array_no_items() {
    check("[      ]", "[]", true, false, false);
}

#[test]
fn array_nesting() {
    check(
        "[ [], [ [ {} ], \"a\" ]  ]",
        "[\n  [],\n  [\n    [\n      {}\n    ],\n    \"a\"\n  ]\n]",
        true,
        false,
        false,
    );
}

#[test]
fn syntax_errors() {
    check(
        "[ null  1.2 \"Hello\" ]",
        "[\n  null  1.2 \"Hello\"\n]",
        true,
        false,
        false,
    );
}

#[test]
fn syntax_errors_2() {
    check(
        "{\"a\":\"b\"\"c\":\"d\" }",
        "{\n  \"a\": \"b\"\"c\": \"d\"\n}",
        true,
        false,
        false,
    );
}

#[test]
fn empty_lines() {
    check(
        "{\n\"a\": true,\n\n\"b\": true\n}",
        "{\n\t\"a\": true,\n\t\"b\": true\n}",
        false,
        false,
        false,
    );
}

#[test]
fn single_line_comment() {
    check(
        "[ \n//comment\n\"foo\", \"bar\"\n] ",
        "[\n  //comment\n  \"foo\",\n  \"bar\"\n]",
        true,
        false,
        false,
    );
}

#[test]
fn block_line_comment() {
    check(
        "[{\n        /*comment*/     \n\"foo\" : true\n}] ",
        "[\n  {\n    /*comment*/\n    \"foo\": true\n  }\n]",
        true,
        false,
        false,
    );
}

#[test]
fn single_line_comment_on_same_line() {
    check(
        " {  \n        \"a\": {}// comment    \n } ",
        "{\n  \"a\": {} // comment    \n}",
        true,
        false,
        false,
    );
}

#[test]
fn single_line_comment_on_same_line_2() {
    check("{ //comment\n}", "{ //comment\n}", true, false, false);
}

#[test]
fn block_comment_on_same_line() {
    check(
        "{      \"a\": {}, /*comment*/    \n        /*comment*/ \"b\": {},    \n        \"c\": {/*comment*/}    } ",
        "{\n  \"a\": {}, /*comment*/\n  /*comment*/ \"b\": {},\n  \"c\": { /*comment*/}\n}",
        true,
        false,
        false,
    );
}

#[test]
fn block_comment_on_same_line_advanced() {
    check(
        " {       \"d\": [\n             null\n        ] /*comment*/\n        ,\"e\": /*comment*/ [null] }",
        "{\n  \"d\": [\n    null\n  ] /*comment*/,\n  \"e\": /*comment*/ [\n    null\n  ]\n}",
        true,
        false,
        false,
    );
}

#[test]
fn multiple_block_comments_on_same_line() {
    check(
        "{      \"a\": {} /*comment*/, /*comment*/   \n        /*comment*/ \"b\": {}  /*comment*/  } ",
        "{\n  \"a\": {} /*comment*/, /*comment*/\n  /*comment*/ \"b\": {} /*comment*/\n}",
        true,
        false,
        false,
    );
}

#[test]
fn multiple_mixed_comments_on_same_line() {
    check(
        "[ /*comment*/  /*comment*/   // comment \n]",
        "[ /*comment*/ /*comment*/ // comment \n]",
        true,
        false,
        false,
    );
}

#[test]
fn range() {
    check(
        "{ \"a\": {},\n|\"b\": [null, null]|\n} ",
        "{ \"a\": {},\n\"b\": [\n  null,\n  null\n]\n} ",
        true,
        false,
        false,
    );
}

#[test]
fn range_with_existing_indent() {
    check(
        "{ \"a\": {},\n   |\"b\": [null],\n\"c\": {}\n}|",
        "{ \"a\": {},\n   \"b\": [\n    null\n  ],\n  \"c\": {}\n}",
        true,
        false,
        false,
    );
}

#[test]
fn range_with_existing_indent_tabs() {
    check(
        "{ \"a\": {},\n|  \"b\": [null],   \n\"c\": {}\n}|    ",
        "{ \"a\": {},\n\t\"b\": [\n\t\tnull\n\t],\n\t\"c\": {}\n}",
        false,
        false,
        false,
    );
}

#[test]
fn property_range_issue_14623() {
    check(
        "{ |\"a\" :| 1,\n  \"b\": 1\n}",
        "{ \"a\": 1,\n  \"b\": 1\n}",
        false,
        false,
        false,
    );
}

#[test]
fn block_comment_none_line_breaking_symbols() {
    check(
        "{ \"a\": [ 1\n/* comment */\n, 2\n/* comment */\n]\n/* comment */\n,\n \"b\": true\n/* comment */\n}",
        "{\n  \"a\": [\n    1\n    /* comment */\n    ,\n    2\n    /* comment */\n  ]\n  /* comment */\n  ,\n  \"b\": true\n  /* comment */\n}",
        true,
        false,
        false,
    );
}

#[test]
fn line_comment_after_none_line_breaking_symbols() {
    check(
        "{ \"a\":\n// comment\nnull,\n \"b\"\n// comment\n: null\n// comment\n}",
        "{\n  \"a\":\n  // comment\n  null,\n  \"b\"\n  // comment\n  : null\n  // comment\n}",
        true,
        false,
        false,
    );
}

#[test]
fn line_comment_enforce_line_comment() {
    check(
        "{\"settings\": // This is some text\n{\n\"foo\": 1\n}\n}",
        "{\n  \"settings\": // This is some text\n  {\n    \"foo\": 1\n  }\n}",
        true,
        false,
        false,
    );
}

#[test]
fn random_content() {
    check("a 1 b 1 3 true", "a 1 b 1 3 true", true, false, false);
}

#[test]
fn insertfinalnewline() {
    check("{\n}", "{}\n", true, true, false);
}

#[test]
fn adjust_the_indentation_of_a_one_line_array() {
    check(
        "{ \"array\": [1,2,3]\n}",
        "{ \"array\": [ 1, 2, 3 ]\n}",
        true,
        false,
        true,
    );
}

#[test]
fn adjust_the_indentation_of_a_multi_line_array() {
    check(
        "{\"array\":\n [1,2,\n 3]\n}",
        "{ \"array\":\n  [ 1, 2,\n    3 ]\n}",
        true,
        false,
        true,
    );
}

#[test]
fn adjust_the_identation_of_a_one_line_object() {
    check(
        "{\"settings\": // This is some text\n{\"foo\": 1}\n}",
        "{ \"settings\": // This is some text\n  { \"foo\": 1 }\n}",
        true,
        false,
        true,
    );
}

#[test]
fn multiple_line_breaks_are_kept() {
    check(
        "{\"settings\":\n\n\n\n{\"foo\": 1}\n}",
        "{ \"settings\":\n\n\n\n  { \"foo\": 1 }\n}",
        true,
        false,
        true,
    );
}

#[test]
fn adjusting_multiple_line_breaks_and_a_block_comment_line_breaks_are_kept() {
    check(
        "{\"settings\":\n\n\n{\"foo\": 1} /* this is a multiline\ncomment */\n}",
        "{ \"settings\":\n\n\n  { \"foo\": 1 } /* this is a multiline\ncomment */\n}",
        true,
        false,
        true,
    );
}

#[test]
fn colon_is_kept_on_its_own_line() {
    check(
        "{\"settings\"\n:\n{\"foo\"\n:\n1}\n}",
        "{ \"settings\"\n  :\n  { \"foo\"\n    :\n    1 }\n}",
        true,
        false,
        true,
    );
}

#[test]
fn adjusting_the_indentation_of_a_nested_multi_line_array() {
    check(
        "{\n\n{\n\n\"array\"   : [1, 2\n3, 4]\n}\n}",
        "{\n\n  {\n\n    \"array\": [ 1, 2\n      3, 4 ]\n  }\n}",
        true,
        false,
        true,
    );
}

#[test]
fn adjusting_the_indentation_for_a_series_of_empty_arrays_or_objects() {
    check(
        "{\n\n}\n\n{\n[\n]\n}",
        "{\n\n}\n\n{\n  [\n  ]\n}",
        true,
        false,
        true,
    );
}

#[test]
fn adjusting_the_indentation_for_a_series_of_multiple_empty_lines_at_the_end() {
    check("{\n}\n\n\n", "{\n}\n\n\n", true, false, true);
}

#[test]
fn adjusting_the_indentation_for_comments_on_separate_lines() {
    check(
        "\n\n\n   // comment 1\n\n\n\n  /* comment 2 */\nconst",
        "\n\n\n// comment 1\n\n\n\n/* comment 2 */\nconst",
        true,
        false,
        true,
    );
}
