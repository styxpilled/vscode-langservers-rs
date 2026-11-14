//! Compares the scanner with the token streams of the upstream scanner for each
//! source in the upstream tests. Fixture: `scripts/css/trace-scanner.mjs`.

use css_languageservice::parser::Dialect;
use css_languageservice::parser::scanner::{Scanner, Token};
use serde_json::Value;

/// UTF-16 offset of a byte offset. Offsets past the end count one unit each.
fn utf16(src: &str, offset: usize) -> usize {
    let n = offset.min(src.len());
    src[..n].encode_utf16().count() + offset - n
}

fn dump(scanner: &Scanner, src: &str, t: &Token) -> Value {
    let start = utf16(src, t.offset);
    serde_json::json!([
        format!("{:?}", t.kind),
        start,
        utf16(src, t.offset + t.len) - start,
        scanner.text(t)
    ])
}

fn normalize(token: &Value) -> Value {
    let mut token = token.clone();
    if let Some(name) = token.get_mut(0) {
        *name = Value::from(name.as_str().unwrap().to_lowercase());
    }
    token
}

#[test]
fn upstream_token_streams() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/scanner.json"
    ))
    .unwrap();
    let cases: Vec<Value> = serde_json::from_str(&fixture).unwrap();
    let mut failures = 0;
    for case in &cases {
        let src = case["source"].as_str().unwrap();
        let dialect = match case["dialect"].as_str().unwrap() {
            "scss" => Dialect::Scss,
            "less" => Dialect::Less,
            _ => Dialect::Css,
        };
        let mut scanner = Scanner::new(dialect);
        scanner.ignore_whitespace = case["ignoreWhitespace"].as_bool().unwrap();
        scanner.ignore_comment = case["ignoreComment"].as_bool().unwrap();
        scanner.set_source(src);
        let mut tokens = Vec::new();
        loop {
            let t = scanner.scan();
            tokens.push(normalize(&dump(&scanner, src, &t)));
            if t.kind == css_languageservice::parser::scanner::TokenType::Eof {
                break;
            }
        }
        scanner.set_source(src);
        let unquoted = scanner
            .scan_unquoted_string()
            .map_or(Value::Null, |t| normalize(&dump(&scanner, src, &t)));

        let expected: Vec<Value> = case["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .map(normalize)
            .collect();
        let expected_unquoted = normalize(&case["unquoted"]);
        if tokens != expected || unquoted != expected_unquoted {
            failures += 1;
            if failures <= 10 {
                eprintln!(
                    "{} {:?}\n  expected {:?} {}\n  actual   {:?} {}",
                    case["dialect"], src, expected, expected_unquoted, tokens, unquoted
                );
            }
        }
    }
    assert_eq!(failures, 0, "{failures} of {} sources differ", cases.len());
}
