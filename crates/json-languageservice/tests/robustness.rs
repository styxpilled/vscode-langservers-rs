//! Runs each feature on each prefix of malformed text with non-ASCII characters. The features must
//! not panic.

mod common;

use json_languageservice::{
    FormattingOptions, JsonSchemaService, LanguageService, LanguageServiceParams, LanguageSettings,
    SchemaConfiguration, find_document_symbols, find_document_symbols2, find_links, format,
    get_folding_ranges, get_selection_ranges, parse, sort,
};
use lsp_types::DiagnosticSeverity;
use serde_json::json;

const SCHEMA_TEXT: &str = r##"{
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": ["object", "array"],
    "properties": {
        "ä": { "type": "array", "uniqueItems": true, "prefixItems": [{ "const": "ö" }], "items": { "minLength": 2 } },
        "€": { "oneOf": [{ "properties": { "𝄞": { "const": null } } }, { "properties": { "𝄞": { "enum": ["x"] } } }] },
        "a": { "pattern": "^\\p{L}+$", "format": "date-time" },
        "b": { "$ref": "#/$defs/%E0%A4" }
    },
    "patternProperties": { "^[€é]": { "deprecated": true } },
    "propertyNames": { "maxLength": 1 },
    "additionalProperties": { "$dynamicRef": "#node" },
    "items": { "anyOf": [{ "type": "string", "format": "uri" }, { "multipleOf": 0.01 }] },
    "contains": { "type": "number" },
    "$defs": { "node": { "$dynamicAnchor": "node", "unevaluatedProperties": false, "$anchor": "é" } },
    "defaultSnippets": [{ "body": { "ä": ["^${1:ö}"] } }, { "bodyText": "€\n\t𝄞" }],
    "examples": [{ "€": "𝄞" }]
}"##;

#[test]
fn no_panic_on_prefixes() {
    let samples = [
        "{ \"ä\": [\"ö\", 1, true], // ü\n \"€\": { \"𝄞\": null } /* 😀\n */ }",
        "[ \"\\é\", é, \"\\u00e9\", /* é",
        "{\r\n\t\"a\" : \"é\r\n\"b\"\r\n// #region 😀\r\n,\"c\": [1,\r\n2]}\r\n// #endregion",
        "{\"b\": {\"d\": 1, \"c\": [ {\"z\": 0, \"y\": \"€\"} ]}, \"a\": 2 /* x\n*/, }",
        r##"{"$ref": "#é", "a": {"$anchor": "é", "$id": "#€"}, "b": {"$ref": "#/a/$id"}, "c": {"$id": "http://é", "$ref": "http://é#€"}, "d": {"$ref": "http://é#/$id"}}"##,
    ];
    let options = FormattingOptions {
        tab_size: 2,
        insert_spaces: true,
        ..Default::default()
    };
    let mut ls = LanguageService::new(LanguageServiceParams {
        client_capabilities: serde_json::from_value(json!({ "textDocument": { "completion": {
            "completionItem": { "documentationFormat": ["markdown"], "labelDetailsSupport": true }
        } } }))
        .unwrap(),
        ..LanguageServiceParams::default()
    });
    ls.configure(&LanguageSettings {
        schemas: vec![SchemaConfiguration {
            uri: "http://x/s.json".to_owned(),
            file_match: Some(vec!["*.json".to_owned()]),
            schema: Some(serde_json::from_str(SCHEMA_TEXT).unwrap()),
            folder_uri: None,
        }],
        ..LanguageSettings::default()
    });
    let mut service = JsonSchemaService::new(None, None, None);
    let schema = service.resolve_inline_schema(
        &serde_json::from_str(SCHEMA_TEXT).unwrap(),
        "schemaservice://untitled/",
    );
    for sample in samples {
        let ends = sample.char_indices().map(|(i, _)| i).chain([sample.len()]);
        for end in ends {
            let document = common::document("test://test.json", &sample[..end]);
            let json = parse(&document, true);
            get_folding_ranges(&document, None);
            get_folding_ranges(&document, Some(1));
            find_document_symbols(&document, &json, None);
            find_document_symbols2(&document, &json, Some(2));
            format(&document, None, &options);
            sort(&document, &options);
            let positions: Vec<_> = (0..=end).map(|o| document.position_at(o)).collect();
            find_links(&document, &json);
            ls.find_document_colors(&document, &json, Some(1));
            for &position in &positions {
                ls.do_complete(&document, position, &json);
                ls.do_hover(&document, position, &json);
            }
            get_selection_ranges(&document, &positions, &json);
            json.validate(
                &document,
                &schema.schemas,
                schema.root,
                DiagnosticSeverity::WARNING,
                None,
                None,
            );
            for offset in [0, end / 2, end] {
                json.get_matching_schemas(&schema.schemas, schema.root, Some(offset), None, None);
            }
        }
    }
}

/// Loads each prefix of a schema text and references it with `$ref`.
#[test]
fn no_panic_on_schema_prefixes() {
    let ends = SCHEMA_TEXT
        .char_indices()
        .map(|(i, _)| i)
        .chain([SCHEMA_TEXT.len()]);
    let document = common::document(
        "test://test.json",
        r#"{ "ä": ["ö", "ö"], "a": "é", "€x": 1, "b": [] }"#,
    );
    let json = parse(&document, true);
    for end in ends {
        let text = SCHEMA_TEXT[..end].to_owned();
        let request = move |_: &str| Ok(text.clone());
        let mut service = JsonSchemaService::new(Some(Box::new(request)), None, None);
        let schema = json!({
            "allOf": [{ "$ref": "http://x/s.json" }, { "$ref": "http://x/s.json#/properties/€" }]
        });
        let resolved = service.resolve_inline_schema(&schema, "schemaservice://untitled/");
        json.validate(
            &document,
            &resolved.schemas,
            resolved.root,
            DiagnosticSeverity::WARNING,
            None,
            None,
        );
    }
}
