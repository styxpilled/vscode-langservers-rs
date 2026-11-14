//! `reset_schema` after a failed load, as the server does when schema content arrives.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use json_languageservice::{
    LanguageService, LanguageServiceParams, LanguageSettings, SchemaConfiguration,
    SchemaRequestError, TextDocument, parse,
};

#[test]
fn reset_clears_root_with_id() {
    let content: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    content.lock().unwrap().insert(
        "zed://schemas/root".into(),
        r#"{ "$id": "https://x/root.json", "properties": { "a": { "$ref": "http://x/a.json" } } }"#
            .into(),
    );
    let requests = content.clone();
    let mut ls = LanguageService::new(LanguageServiceParams {
        schema_request_service: Some(Box::new(move |uri| {
            requests
                .lock()
                .unwrap()
                .get(uri)
                .cloned()
                .ok_or_else(|| SchemaRequestError {
                    message: "Loading".into(),
                    code: None,
                })
        })),
        ..Default::default()
    });
    ls.configure(&LanguageSettings {
        validate: Some(true),
        allow_comments: Some(true),
        schemas: vec![SchemaConfiguration {
            uri: "zed://schemas/root".into(),
            file_match: Some(vec!["a.json".into()]),
            ..Default::default()
        }],
    });
    let doc = TextDocument::new(
        "file:///a.json".parse().unwrap(),
        "json",
        1,
        r#"{ "a": 1 }"#.into(),
    );
    let json = parse(&doc, true);
    let messages = |ls: &mut LanguageService| -> Vec<String> {
        ls.do_validation(&doc, &json, None, None)
            .into_iter()
            .map(|d| d.message)
            .collect()
    };
    assert_eq!(
        messages(&mut ls),
        ["Unable to load schema from 'http://x/a.json': Loading."]
    );
    content
        .lock()
        .unwrap()
        .insert("http://x/a.json".into(), r#"{ "type": "string" }"#.into());
    assert!(ls.reset_schema("http://x/a.json"));
    assert_eq!(messages(&mut ls), ["Incorrect type. Expected \"string\"."]);
}
