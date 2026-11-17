//! `docs/experimental.md`: the color limit counts colors.

mod common;

use json_languageservice::{LanguageService, LanguageSettings, SchemaConfiguration, parse};
use serde_json::json;

#[test]
fn color_limit() {
    let mut ls = LanguageService::new(Default::default());
    ls.configure(&LanguageSettings {
        schemas: vec![SchemaConfiguration {
            uri: "http://x/s.json".to_owned(),
            file_match: Some(vec!["*.json".to_owned()]),
            schema: Some(json!({ "items": { "format": "color-hex" } })),
            folder_uri: None,
        }],
        ..LanguageSettings::default()
    });
    let document = common::document("test://test.json", r##"["x", "#fff", "#000"]"##);
    let json = parse(&document, true);
    let mut colors = |limit| ls.find_document_colors(&document, &json, limit).len();
    assert_eq!(colors(None), 2);
    assert_eq!(colors(Some(0)), 0);
    assert_eq!(colors(Some(1)), 1);
    assert_eq!(colors(Some(2)), 2);
}
