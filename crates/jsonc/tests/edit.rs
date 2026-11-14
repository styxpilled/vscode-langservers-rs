//! Port of `node-jsonc-parser/src/test/edit.test.ts`.

use jsonc::{FormattingOptions, ModificationOptions, Segment, modify};
use serde_json::Value;

fn p(name: &str) -> Segment {
    Segment::from(name)
}

fn i(index: usize) -> Segment {
    Segment::from(index)
}

fn formatting_options() -> FormattingOptions {
    FormattingOptions {
        insert_spaces: true,
        tab_size: 2,
        eol: "\n".to_owned(),
        keep_lines: false,
        ..Default::default()
    }
}

fn assert_edit(
    content: &str,
    path: &[Segment],
    value: Option<&str>,
    options: &ModificationOptions,
    expected: &str,
) {
    let value: Option<Value> = value.map(|v| serde_json::from_str(v).unwrap());
    let edits = modify(content, path, value.as_ref(), options).unwrap();
    let mut content = content.to_owned();
    let mut last_edit_offset = content.len();
    for edit in edits.iter().rev() {
        assert!(edit.offset + edit.length <= content.len());
        assert!(last_edit_offset >= edit.offset + edit.length);
        last_edit_offset = edit.offset;
        content.replace_range(edit.offset..edit.offset + edit.length, &edit.content);
    }
    assert_eq!(content, expected);
}

#[test]
fn set_property() {
    assert_edit(
        "{\n  \"x\": \"y\"\n}",
        &[p("x")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": \"bar\"\n}",
    );
    assert_edit(
        "true",
        &[],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "\"bar\"",
    );
    assert_edit(
        "{\n  \"x\": \"y\"\n}",
        &[p("x")],
        Some(r#"{"key":true}"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": {\n    \"key\": true\n  }\n}",
    );
    assert_edit(
        "{\n  \"a\": \"b\",  \"x\": \"y\"\n}",
        &[p("a")],
        Some(r#"null"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"a\": null,  \"x\": \"y\"\n}",
    );
}

#[test]
fn insert_property() {
    assert_edit(
        "{}",
        &[p("foo")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"foo\": \"bar\"\n}",
    );
    assert_edit(
        "{}",
        &[p("foo"), p("foo2")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"foo\": {\n    \"foo2\": \"bar\"\n  }\n}",
    );
    assert_edit(
        "{\n}",
        &[p("foo")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"foo\": \"bar\"\n}",
    );
    assert_edit(
        "  {\n  }",
        &[p("foo")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "  {\n    \"foo\": \"bar\"\n  }",
    );
    assert_edit(
        "{\n  \"x\": \"y\"\n}",
        &[p("foo")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": \"y\",\n  \"foo\": \"bar\"\n}",
    );
    assert_edit(
        "{\n  \"x\": \"y\"\n}",
        &[p("e")],
        Some(r#""null""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": \"y\",\n  \"e\": \"null\"\n}",
    );
    assert_edit(
        "{\n  \"x\": \"y\"\n}",
        &[p("x")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": \"bar\"\n}",
    );
    assert_edit(
        "{\n  \"x\": {\n    \"a\": 1,\n    \"b\": true\n  }\n}\n",
        &[p("x")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": \"bar\"\n}\n",
    );
    assert_edit(
        "{\n  \"x\": {\n    \"a\": 1,\n    \"b\": true\n  }\n}\n",
        &[p("x"), p("b")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": {\n    \"a\": 1,\n    \"b\": \"bar\"\n  }\n}\n",
    );
    assert_edit(
        "{\n  \"x\": {\n    \"a\": 1,\n    \"b\": true\n  }\n}\n",
        &[p("x"), p("c")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            get_insertion_index: Some(&|_: &[&str]| 0),
            ..Default::default()
        },
        "{\n  \"x\": {\n    \"c\": \"bar\",\n    \"a\": 1,\n    \"b\": true\n  }\n}\n",
    );
    assert_edit(
        "{\n  \"x\": {\n    \"a\": 1,\n    \"b\": true\n  }\n}\n",
        &[p("x"), p("c")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            get_insertion_index: Some(&|_: &[&str]| 1),
            ..Default::default()
        },
        "{\n  \"x\": {\n    \"a\": 1,\n    \"c\": \"bar\",\n    \"b\": true\n  }\n}\n",
    );
    assert_edit(
        "{\n  \"x\": {\n    \"a\": 1,\n    \"b\": true\n  }\n}\n",
        &[p("x"), p("c")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            get_insertion_index: Some(&|_: &[&str]| 2),
            ..Default::default()
        },
        "{\n  \"x\": {\n    \"a\": 1,\n    \"b\": true,\n    \"c\": \"bar\"\n  }\n}\n",
    );
    assert_edit(
        "{\n  \"x\": {\n    \"a\": 1,\n    \"b\": true\n  }\n}\n",
        &[p("c")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": {\n    \"a\": 1,\n    \"b\": true\n  },\n  \"c\": \"bar\"\n}\n",
    );
    assert_edit(
        "{\n  \"a\": [\n    {\n    } \n  ]  \n}",
        &[p("foo")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"a\": [\n    {\n    } \n  ],\n  \"foo\": \"bar\"\n}",
    );
    assert_edit(
        "",
        &[p("foo"), i(0)],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"foo\": [\n    \"bar\"\n  ]\n}",
    );
    assert_edit(
        "//comment",
        &[p("foo"), i(0)],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"foo\": [\n    \"bar\"\n  ]\n} //comment",
    );
}

#[test]
fn remove_property() {
    assert_edit(
        "{\n  \"x\": \"y\"\n}",
        &[p("x")],
        None,
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n}",
    );
    assert_edit(
        "{\n  \"x\": \"y\", \"a\": []\n}",
        &[p("x")],
        None,
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"a\": []\n}",
    );
    assert_edit(
        "{\n  \"x\": \"y\", \"a\": []\n}",
        &[p("a")],
        None,
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": \"y\"\n}",
    );
}

#[test]
fn set_item() {
    assert_edit(
        "{\n  \"x\": [1, 2, 3],\n  \"y\": 0\n}",
        &[p("x"), i(0)],
        Some(r#"6"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": [6, 2, 3],\n  \"y\": 0\n}",
    );
    assert_edit(
        "{\n  \"x\": [1, 2, 3],\n  \"y\": 0\n}",
        &[p("x"), i(1)],
        Some(r#"5"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": [1, 5, 3],\n  \"y\": 0\n}",
    );
    assert_edit(
        "{\n  \"x\": [1, 2, 3],\n  \"y\": 0\n}",
        &[p("x"), i(2)],
        Some(r#"4"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": [1, 2, 4],\n  \"y\": 0\n}",
    );
    assert_edit(
        "{\n  \"x\": [1, 2, 3],\n  \"y\": 0\n}",
        &[p("x"), i(3)],
        Some(r#"3"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": [\n    1,\n    2,\n    3,\n    3\n  ],\n  \"y\": 0\n}",
    );
}

#[test]
fn insert_item_at_0_isarrayinsertion_true() {
    assert_edit(
        "[\n  2,\n  3\n]",
        &[i(0)],
        Some(r#"1"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            is_array_insertion: true,
            ..Default::default()
        },
        "[\n  1,\n  2,\n  3\n]",
    );
}

#[test]
fn insert_item_at_0_in_empty_array() {
    assert_edit(
        "[\n]",
        &[i(0)],
        Some(r#"1"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "[\n  1\n]",
    );
}

#[test]
fn insert_item_at_an_index_isarrayinsertion_true() {
    assert_edit(
        "[\n  1,\n  3\n]",
        &[i(1)],
        Some(r#"2"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            is_array_insertion: true,
            ..Default::default()
        },
        "[\n  1,\n  2,\n  3\n]",
    );
}

#[test]
fn insert_item_at_an_index_in_empty_array() {
    assert_edit(
        "[\n]",
        &[i(1)],
        Some(r#"1"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "[\n  1\n]",
    );
}

#[test]
fn insert_item_at_end_index() {
    assert_edit(
        "[\n  1,\n  2\n]",
        &[i(2)],
        Some(r#"3"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "[\n  1,\n  2,\n  3\n]",
    );
}

#[test]
fn insert_item_at_end_to_empty_array() {
    assert_edit(
        "[\n]",
        &[i(usize::MAX)],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "[\n  \"bar\"\n]",
    );
}

#[test]
fn insert_item_at_end() {
    assert_edit(
        "[\n  1,\n  2\n]",
        &[i(usize::MAX)],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "[\n  1,\n  2,\n  \"bar\"\n]",
    );
}

#[test]
fn remove_item_in_array_with_one_item() {
    assert_edit(
        "[\n  1\n]",
        &[i(0)],
        None,
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "[]",
    );
}

#[test]
fn remove_item_in_the_middle_of_the_array() {
    assert_edit(
        "[\n  1,\n  2,\n  3\n]",
        &[i(1)],
        None,
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "[\n  1,\n  3\n]",
    );
}

#[test]
fn remove_last_item_in_the_array() {
    assert_edit(
        "[\n  1,\n  2,\n  \"bar\"\n]",
        &[i(2)],
        None,
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "[\n  1,\n  2\n]",
    );
}

#[test]
fn remove_last_item_in_the_array_if_ends_with_comma() {
    assert_edit(
        "[\n  1,\n  \"foo\",\n  \"bar\",\n]",
        &[i(2)],
        None,
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "[\n  1,\n  \"foo\"\n]",
    );
}

#[test]
fn remove_last_item_in_the_array_if_there_is_a_comment_in_the_beginning() {
    assert_edit(
        "// This is a comment\n[\n  1,\n  \"foo\",\n  \"bar\"\n]",
        &[i(2)],
        None,
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "// This is a comment\n[\n  1,\n  \"foo\"\n]",
    );
}

#[test]
fn remove_last_string_item_in_array_without_formatting() {
    assert_edit(
        "{\"items\":[\"1\",\"2\"]}",
        &[p("items"), i(1)],
        None,
        &ModificationOptions::default(),
        "{\"items\":[\"1\"]}",
    );
}

#[test]
fn remove_last_number_item_in_array_without_formatting() {
    assert_edit(
        "{\"items\":[1,2]}",
        &[p("items"), i(1)],
        None,
        &ModificationOptions::default(),
        "{\"items\":[1]}",
    );
}

#[test]
fn set_property_without_formatting() {
    assert_edit(
        "{\n  \"x\": [1, 2, 3],\n  \"y\": 0\n}",
        &[p("x"), i(0)],
        Some(r#"{"a":1,"b":2}"#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"x\": [{\n      \"a\": 1,\n      \"b\": 2\n    }, 2, 3],\n  \"y\": 0\n}",
    );
    assert_edit(
        "{\n  \"x\": [1, 2, 3],\n  \"y\": 0\n}",
        &[p("x"), i(0)],
        Some(r#"{"a":1,"b":2}"#),
        &ModificationOptions::default(),
        "{\n  \"x\": [{\"a\":1,\"b\":2}, 2, 3],\n  \"y\": 0\n}",
    );
}

#[test]
fn insert_property_when_keeplines_is_true() {
    assert_edit(
        "{}",
        &[p("foo"), p("foo2")],
        Some(r#""bar""#),
        &ModificationOptions {
            formatting_options: Some(formatting_options()),
            ..Default::default()
        },
        "{\n  \"foo\": {\n    \"foo2\": \"bar\"\n  }\n}",
    );
}
