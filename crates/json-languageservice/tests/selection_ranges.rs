//! Port of `vscode-json-languageservice/src/test/selectionRanges.test.ts`.

mod common;

use json_languageservice::{get_selection_ranges, parse};

fn assert_ranges(content: &str, expected: &[(usize, &str)]) {
    let offset = content.find('|').unwrap();
    let text = content.replacen('|', "", 1);
    let document = common::document("test://foo.json", &text);
    let json = parse(&document, true);
    let ranges = get_selection_ranges(&document, &[document.position_at(offset)], &json);

    let mut actual = Vec::new();
    let mut current = ranges.first();
    while let Some(range) = current {
        actual.push((
            document.offset_at(range.range.start),
            document.text_in(range.range),
        ));
        current = range.parent.as_deref();
    }
    assert_eq!(actual, expected, "{content:?}");
}

#[test]
fn strings() {
    assert_ranges("\"ab|cd\"", &[(1, "abcd"), (0, "\"abcd\"")]);
    assert_ranges("\"|abcd\"", &[(1, "abcd"), (0, "\"abcd\"")]);
    assert_ranges("\"abcd|\"", &[(1, "abcd"), (0, "\"abcd\"")]);
    assert_ranges("|\"abcd\"", &[(0, "\"abcd\"")]);
    assert_ranges("\"abcd\"|", &[(0, "\"abcd\"")]);
}

#[test]
fn bools_numbers_and_nulls() {
    assert_ranges("|true", &[(0, "true")]);
    assert_ranges("false|", &[(0, "false")]);
    assert_ranges("fal|se", &[(0, "false")]);
    assert_ranges("|1", &[(0, "1")]);
    assert_ranges("5677|", &[(0, "5677")]);
    assert_ranges("567.|7", &[(0, "567.7")]);
    assert_ranges("|null", &[(0, "null")]);
    assert_ranges("null|", &[(0, "null")]);
    assert_ranges("nu|ll", &[(0, "null")]);
}

#[test]
fn properties() {
    assert_ranges(
        "{ \"f|oo\": true, \"bar\": \"bit\" }",
        &[
            (3, "foo"),
            (2, "\"foo\""),
            (2, "\"foo\": true"),
            (2, "\"foo\": true,"),
            (1, " \"foo\": true, \"bar\": \"bit\" "),
            (0, "{ \"foo\": true, \"bar\": \"bit\" }"),
        ],
    );
    assert_ranges(
        "{ \"foo\"|: true, \"bar\": \"bit\" }",
        &[
            (2, "\"foo\""),
            (2, "\"foo\": true"),
            (2, "\"foo\": true,"),
            (1, " \"foo\": true, \"bar\": \"bit\" "),
            (0, "{ \"foo\": true, \"bar\": \"bit\" }"),
        ],
    );
    assert_ranges(
        "{ \"foo\":| true, \"bar\": \"bit\" }",
        &[
            (2, "\"foo\": true"),
            (2, "\"foo\": true,"),
            (1, " \"foo\": true, \"bar\": \"bit\" "),
            (0, "{ \"foo\": true, \"bar\": \"bit\" }"),
        ],
    );
    assert_ranges(
        "{ \"foo\": |true, \"bar\": \"bit\" }",
        &[
            (9, "true"),
            (2, "\"foo\": true"),
            (2, "\"foo\": true,"),
            (1, " \"foo\": true, \"bar\": \"bit\" "),
            (0, "{ \"foo\": true, \"bar\": \"bit\" }"),
        ],
    );
    assert_ranges(
        "{ \"foo\": true|, \"bar\": \"bit\" }",
        &[
            (9, "true"),
            (2, "\"foo\": true"),
            (2, "\"foo\": true,"),
            (1, " \"foo\": true, \"bar\": \"bit\" "),
            (0, "{ \"foo\": true, \"bar\": \"bit\" }"),
        ],
    );
    assert_ranges(
        "{ \"foo\": true,| \"bar\": \"bit\" }",
        &[
            (1, " \"foo\": true, \"bar\": \"bit\" "),
            (0, "{ \"foo\": true, \"bar\": \"bit\" }"),
        ],
    );
    assert_ranges(
        "{ \"foo\": true, |\"bar\": \"bit\" }",
        &[
            (15, "\"bar\""),
            (15, "\"bar\": \"bit\""),
            (1, " \"foo\": true, \"bar\": \"bit\" "),
            (0, "{ \"foo\": true, \"bar\": \"bit\" }"),
        ],
    );
    assert_ranges(
        "{ \"foo\": true, \"bar\": \"bit\"| }",
        &[
            (22, "\"bit\""),
            (15, "\"bar\": \"bit\""),
            (1, " \"foo\": true, \"bar\": \"bit\" "),
            (0, "{ \"foo\": true, \"bar\": \"bit\" }"),
        ],
    );
    assert_ranges(
        "{ \"line1\": \"Foo|Bar\" }",
        &[
            (12, "FooBar"),
            (11, "\"FooBar\""),
            (2, "\"line1\": \"FooBar\""),
            (1, " \"line1\": \"FooBar\" "),
            (0, "{ \"line1\": \"FooBar\" }"),
        ],
    );
}

#[test]
fn objects() {
    assert_ranges("|{}", &[(0, "{}")]);
    assert_ranges("{|}", &[(0, "{}")]);
    assert_ranges("{|  }", &[(1, "  "), (0, "{  }")]);
}

#[test]
fn array() {
    assert_ranges("|[[ ], []]", &[(0, "[[ ], []]")]);
    assert_ranges(
        "[|[ ], []]",
        &[(1, "[ ]"), (1, "[ ],"), (1, "[ ], []"), (0, "[[ ], []]")],
    );
    assert_ranges(
        "[[| ], []]",
        &[
            (2, " "),
            (1, "[ ]"),
            (1, "[ ],"),
            (1, "[ ], []"),
            (0, "[[ ], []]"),
        ],
    );
}
