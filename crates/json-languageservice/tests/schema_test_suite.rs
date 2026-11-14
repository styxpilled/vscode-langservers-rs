//! Runs the JSON-Schema-Test-Suite as `jsonSchemaTestSuite.test.ts` does.
//! Needs the suite at `upstream/JSON-Schema-Test-Suite` (commit f6fd52a).

use std::path::{Path, PathBuf};

use json_languageservice::{
    DocumentLanguageSettings, JsonSchemaService, JsonValidation, SchemaDraft, SchemaRequestError,
    TextDocument, parse, schema_contributions,
};
use serde_json::Value;

fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../upstream/JSON-Schema-Test-Suite")
}

const DRAFTS: [(&str, SchemaDraft, &str); 5] = [
    (
        "draft4",
        SchemaDraft::V4,
        "http://json-schema.org/draft-04/schema#",
    ),
    (
        "draft6",
        SchemaDraft::V6,
        "http://json-schema.org/draft-06/schema#",
    ),
    (
        "draft7",
        SchemaDraft::V7,
        "http://json-schema.org/draft-07/schema#",
    ),
    (
        "draft2019-09",
        SchemaDraft::V2019_09,
        "https://json-schema.org/draft/2019-09/schema",
    ),
    (
        "draft2020-12",
        SchemaDraft::V2020_12,
        "https://json-schema.org/draft/2020-12/schema",
    ),
];

fn new_service() -> JsonSchemaService {
    let remotes = suite_dir().join("remotes");
    let draft06 = include_str!("fixtures/draft06-metaschema.json").to_owned();
    let request = move |uri: &str| -> Result<String, SchemaRequestError> {
        if let Some(path) = uri.strip_prefix("http://localhost:1234/") {
            return Ok(
                std::fs::read_to_string(remotes.join(path)).unwrap_or_else(|_| {
                    format!("{{ \"error\": \"Failed to load remote schema: {uri}\" }}")
                }),
            );
        }
        if matches!(
            uri,
            "https://json-schema.org/draft-06/schema"
                | "http://json-schema.org/draft-06/schema#"
                | "http://json-schema.org/draft-06/schema"
        ) {
            return Ok(draft06.clone());
        }
        Ok("{}".to_owned())
    };
    let resolve = |relative: &str, resource: &str| -> String {
        match url::Url::parse(resource).and_then(|base| base.join(relative)) {
            Ok(u) => u.to_string(),
            Err(_) => relative.to_owned(),
        }
    };
    let mut service =
        JsonSchemaService::new(Some(Box::new(request)), Some(Box::new(resolve)), None);
    service.set_schema_contributions(&schema_contributions());
    service
}

/// `JSON.stringify`, which writes whole floats without a fraction.
fn stringify(value: &Value) -> String {
    match value {
        Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap();
            if f.fract() == 0.0 && f.abs() < 1e21 {
                format!("{f:.0}")
            } else {
                n.to_string()
            }
        }
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(stringify).collect::<Vec<_>>().join(",")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!("{}:{}", Value::from(k.as_str()), stringify(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => value.to_string(),
    }
}

#[test]
fn json_schema_test_suite() {
    let tests_dir = suite_dir().join("tests");
    if !tests_dir.exists() {
        eprintln!("skipped: {} not found", tests_dir.display());
        return;
    }
    let mut service = new_service();
    let validation = JsonValidation::default();
    let mut failures = Vec::new();
    let mut count = 0;
    for (draft, schema_draft, schema_uri) in DRAFTS {
        let mut entries: Vec<_> = std::fs::read_dir(tests_dir.join(draft))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries {
            let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
            let groups: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            for group in groups.as_array().unwrap() {
                for test in group["tests"].as_array().unwrap() {
                    count += 1;
                    let text = stringify(&test["data"]);
                    let doc =
                        TextDocument::new("foo://bar/file.json".parse().unwrap(), "json", 0, text);
                    let json_doc = parse(&doc, false);
                    assert!(json_doc.syntax_errors.is_empty());
                    let mut schema = group["schema"].clone();
                    if let Value::Object(map) = &mut schema
                        && !map.contains_key("$schema")
                    {
                        map.insert("$schema".to_owned(), Value::from(schema_uri));
                    }
                    let schema = match schema {
                        Value::Bool(true) => Value::Object(Default::default()),
                        Value::Bool(false) => serde_json::json!({ "not": {} }),
                        s => s,
                    };
                    let settings = DocumentLanguageSettings {
                        schema_draft: Some(schema_draft),
                        ..DocumentLanguageSettings::default()
                    };
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        validation.do_validation(
                            &mut service,
                            &doc,
                            &json_doc,
                            Some(&settings),
                            Some(&schema),
                        )
                    }));
                    let id = format!(
                        "{draft}/{file_name}/{}/{}",
                        group["description"].as_str().unwrap(),
                        test["description"].as_str().unwrap()
                    );
                    match result {
                        Err(_) => failures.push(format!("{id}: panic")),
                        Ok(errors) => {
                            let valid = test["valid"].as_bool().unwrap();
                            if valid && !errors.is_empty() {
                                failures.push(format!(
                                    "{id}: unexpected {:?}",
                                    errors.iter().map(|e| &e.message).collect::<Vec<_>>()
                                ));
                            } else if !valid && errors.is_empty() {
                                failures.push(format!("{id}: expected an error"));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {count} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
