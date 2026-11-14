#![allow(dead_code)]

use json_languageservice::{JsonDocument, TextDocument, parse};

pub fn document(uri: &str, text: &str) -> TextDocument {
    TextDocument::new(uri.parse().unwrap(), "json", 0, text.to_owned())
}

pub fn parse_text(text: &str) -> (TextDocument, JsonDocument) {
    let document = document("foo://bar/file.json", text);
    let json = parse(&document, true);
    (document, json)
}
