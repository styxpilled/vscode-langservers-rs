//! `docs/experimental.md`, "Schema loading": `$ref` is resolved on use.

use std::sync::{Arc, Mutex};

use json_languageservice::{
    LanguageService, LanguageServiceParams, LanguageSettings, SchemaConfiguration, TextDocument,
    parse,
};
use lsp_types::Position;

fn service(schema: &str, requests: Arc<Mutex<Vec<String>>>) -> LanguageService {
    let mut ls = LanguageService::new(LanguageServiceParams {
        schema_request_service: Some(Box::new(move |uri| {
            requests.lock().unwrap().push(uri.to_owned());
            Ok(match uri {
                "http://x/a.json" => r#"{ "type": "string" }"#.to_owned(),
                _ => r#"{ "type": "number" }"#.to_owned(),
            })
        })),
        ..Default::default()
    });
    ls.configure(&LanguageSettings {
        validate: Some(true),
        allow_comments: Some(true),
        schemas: vec![SchemaConfiguration {
            uri: "http://x/root.json".into(),
            file_match: Some(vec!["*.json".into()]),
            schema: Some(serde_json::from_str(schema).unwrap()),
            ..Default::default()
        }],
    });
    ls
}

#[test]
fn external_document_loads_on_use() {
    let requests = Arc::default();
    let schema = r#"{ "properties": {
        "a": { "$ref": "http://x/a.json" },
        "b": { "$ref": "http://x/b.json" }
    } }"#;
    let mut ls = service(schema, Arc::clone(&requests));
    let doc = TextDocument::new(
        "file:///d.json".parse().unwrap(),
        "json",
        1,
        r#"{ "a": 1 }"#.into(),
    );
    let json = parse(&doc, true);
    let messages: Vec<String> = ls
        .do_validation(&doc, &json, None, None)
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert_eq!(messages, ["Incorrect type. Expected \"string\"."]);
    assert_eq!(*requests.lock().unwrap(), ["http://x/a.json"]);
}

#[test]
fn ref_with_annotations_keeps_them() {
    let schema = r##"{
        "properties": { "f": { "anyOf": [
            { "const": "auto", "description": "Auto." },
            { "$ref": "#/$defs/server", "description": "The language server." }
        ] } },
        "$defs": { "server": { "const": "language_server" } }
    }"##;
    let mut ls = service(schema, Arc::default());
    let doc = TextDocument::new(
        "file:///d.json".parse().unwrap(),
        "json",
        1,
        r#"{ "f":  }"#.into(),
    );
    let json = parse(&doc, true);
    let list = ls.do_complete(&doc, Position::new(0, 7), &json);
    let item = list
        .items
        .iter()
        .find(|i| i.label == "\"language_server\"")
        .expect("completion item");
    assert_eq!(
        serde_json::to_value(&item.documentation).unwrap(),
        "The language server."
    );
}
