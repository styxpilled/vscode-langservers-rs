//! Behavior that differs from upstream, and checks for UTF-8 offsets.

use jsonc::{
    FormattingOptions, ParseOptions, Range, ScanError, Scanner, SyntaxKind, format, get_location,
    parse, parse_tree, strip_comments,
};
use serde_json::json;

fn scan_string(text: &str) -> (String, ScanError) {
    let mut scanner = Scanner::new(text, false);
    assert_eq!(scanner.scan(), SyntaxKind::StringLiteral);
    (scanner.token_value().to_owned(), scanner.token_error())
}

#[test]
fn surrogate_pair_escape_is_one_character() {
    assert_eq!(
        scan_string(r#""\ud83d\ude00""#),
        ("😀".to_owned(), ScanError::None)
    );
}

#[test]
fn unpaired_surrogate_escape_is_replacement_character() {
    assert_eq!(
        scan_string(r#""\ud83dx""#),
        ("\u{fffd}x".to_owned(), ScanError::None)
    );
    assert_eq!(
        scan_string(r#""\ude00""#),
        ("\u{fffd}".to_owned(), ScanError::None)
    );
    assert_eq!(
        scan_string(r#""\ud83d\u0041""#),
        ("\u{fffd}A".to_owned(), ScanError::None)
    );
}

#[test]
fn invalid_escape_of_non_ascii_character() {
    assert_eq!(
        scan_string("\"\\éa\""),
        ("a".to_owned(), ScanError::InvalidEscapeCharacter)
    );
}

#[test]
fn comment_value_starts_at_slash() {
    let mut scanner = Scanner::new("1 // c", false);
    scanner.scan();
    scanner.scan();
    assert_eq!(scanner.scan(), SyntaxKind::LineCommentTrivia);
    assert_eq!(scanner.token_value(), "// c");
}

#[test]
fn unclosed_block_comment_ends_at_text_end() {
    let mut scanner = Scanner::new("/*", false);
    assert_eq!(scanner.scan(), SyntaxKind::BlockCommentTrivia);
    assert_eq!(scanner.token_error(), ScanError::UnexpectedEndOfComment);
    assert_eq!(scanner.token_length(), 2);
}

#[test]
fn strip_comments_keeps_length() {
    let text = "{ \"a\": 1 /* x\ny */, // z\n\"b\": 2 }";
    let stripped = strip_comments(text, Some(' '));
    assert_eq!(stripped.len(), text.len());
    assert_eq!(stripped, "{ \"a\": 1     \n    ,     \n\"b\": 2 }");
    assert_eq!(strip_comments(text, None), "{ \"a\": 1 , \n\"b\": 2 }");
}

#[test]
fn large_numbers() {
    let (value, errors) = parse(
        "[1e999, 9007199254740993, -0, 1.5]",
        ParseOptions::default(),
    );
    assert_eq!(errors, []);
    assert_eq!(value, Some(json!([null, 9007199254740992.0, 0, 1.5])));
}

#[test]
fn offsets_are_utf8_bytes() {
    let text = "{\"ä\": \"ö\"}";
    let tree = parse_tree(text, ParseOptions::default()).0.unwrap();
    let root = &tree[tree.root()];
    assert_eq!(root.length, text.len());
    let value = tree
        .find_node_at_location(tree.root(), &["ä".into()])
        .unwrap();
    assert_eq!(tree[value].offset, 7);
    assert_eq!(tree[value].length, 4);
}

#[test]
fn no_panic_at_any_char_boundary() {
    let samples = [
        "{ \"ä\": [\"ö\", 1, true], // ü\n \"€\": { \"𝄞\": null } /* 😀 */ }",
        "[ \"\\é\", é, \"\\u00e9\", /* é",
        "{\r\n\t\"a\" : \"é\r\n\"b\"",
    ];
    let options = FormattingOptions {
        insert_spaces: true,
        tab_size: 2,
        ..Default::default()
    };
    for sample in samples {
        let boundaries: Vec<usize> = sample
            .char_indices()
            .map(|(i, _)| i)
            .chain([sample.len()])
            .collect();
        for &end in &boundaries {
            let text = &sample[..end];
            let _ = parse(text, ParseOptions::default());
            let _ = parse_tree(text, ParseOptions::default());
            let _ = strip_comments(text, Some('*'));
            let _ = format(text, None, &options);
            for &start in boundaries.iter().filter(|&&b| b <= end) {
                let _ = get_location(text, start);
                let range = Range {
                    offset: start,
                    length: end - start,
                };
                let _ = format(text, Some(range), &options);
            }
        }
    }
}
