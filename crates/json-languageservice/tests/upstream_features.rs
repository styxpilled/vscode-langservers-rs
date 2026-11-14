//! Replays the calls of the upstream `completion.test.ts`, `hover.test.ts`, `links.test.ts` and
//! the color tests of `documentSymbols.test.ts`, and compares each result with the upstream
//! result.
//!
//! `fixtures/upstream_features.json` is written by `scripts/trace/ls-run.mjs`.

use std::collections::{HashMap, VecDeque};

use json_languageservice::{
    LanguageService, LanguageServiceParams, LanguageSettings, SchemaConfiguration,
    SchemaRequestError, TextDocument, find_links, parse,
};
use lsp_types::Position;
use serde_json::Value;

fn settings(raw: &Value) -> LanguageSettings {
    let schemas = raw["schemas"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| SchemaConfiguration {
            uri: s["uri"].as_str().unwrap_or_default().to_owned(),
            file_match: s["fileMatch"].as_array().map(|f| {
                f.iter()
                    .filter_map(|p| p.as_str().map(str::to_owned))
                    .collect()
            }),
            schema: s.get("schema").filter(|s| !s.is_null()).cloned(),
            folder_uri: s["folderUri"].as_str().map(str::to_owned),
        })
        .collect();
    LanguageSettings {
        validate: raw["validate"].as_bool(),
        allow_comments: raw["allowComments"].as_bool(),
        schemas,
    }
}

fn new_service(id: u64, ops: &[Value]) -> LanguageService {
    let new = ops
        .iter()
        .find(|o| o["op"] == "new" && o["svc"] == id)
        .unwrap();
    let mut responses: HashMap<String, VecDeque<Result<String, SchemaRequestError>>> =
        HashMap::new();
    for op in ops
        .iter()
        .filter(|o| o["op"] == "request" && o["svc"] == id)
    {
        let response = match op["content"].as_str() {
            Some(content) => Ok(content.to_owned()),
            None => Err(SchemaRequestError {
                message: op["error"]["message"].as_str().unwrap_or("").to_owned(),
                code: None,
            }),
        };
        responses
            .entry(op["uri"].as_str().unwrap().to_owned())
            .or_default()
            .push_back(response);
    }
    let request = move |uri: &str| {
        responses
            .get_mut(uri)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| {
                Err(SchemaRequestError {
                    message: "Resource not found".to_owned(),
                    code: None,
                })
            })
    };
    LanguageService::new(LanguageServiceParams {
        schema_request_service: (new["requestService"] == true)
            .then(|| Box::new(request) as json_languageservice::SchemaRequestService),
        client_capabilities: serde_json::from_value(new["clientCapabilities"].clone())
            .unwrap_or_default(),
        ..LanguageServiceParams::default()
    })
}

#[test]
fn upstream_features() {
    replay(include_str!("fixtures/upstream_features.json"));
}

/// Replays a trace of `scripts/trace/ls-sweep.mjs`, which runs completion and hover at each
/// offset. The trace is large, so it is not a fixture. Set `JSON_LS_SWEEP` to its path.
#[test]
fn upstream_sweep() {
    if let Ok(path) = std::env::var("JSON_LS_SWEEP") {
        replay(&std::fs::read_to_string(path).unwrap());
    }
}

fn replay(trace: &str) {
    let trace: Vec<Value> = serde_json::from_str(trace).unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    let mut skipped = Vec::new();
    for test in &trace {
        let name = test["name"].as_str().unwrap();
        let ops = test["ops"].as_array().unwrap();
        // Contributions are a JS extension point. The server does not use them.
        if ops.iter().any(|o| o["contributions"] == true) {
            skipped.push(name);
            continue;
        }
        let mut services: HashMap<u64, LanguageService> = HashMap::new();
        for op in ops {
            let id = op["svc"].as_u64().unwrap();
            let kind = op["op"].as_str().unwrap();
            if kind == "new" {
                services.insert(id, new_service(id, ops));
                continue;
            }
            let ls = services.get_mut(&id).unwrap();
            if kind == "configure" {
                ls.configure(&settings(&op["settings"]));
                continue;
            }
            if kind == "request" {
                continue;
            }
            let document = TextDocument::new(
                op["doc"]["uri"].as_str().unwrap().parse().unwrap(),
                "json",
                0,
                op["doc"]["text"].as_str().unwrap().to_owned(),
            );
            let json = parse(&document, true);
            let position =
                || -> Position { serde_json::from_value(op["args"][0].clone()).unwrap() };
            let actual = match kind {
                "complete" => serde_json::to_value(ls.do_complete(&document, position(), &json)),
                "hover" => serde_json::to_value(ls.do_hover(&document, position(), &json)),
                "links" => serde_json::to_value(find_links(&document, &json)),
                "colors" => serde_json::to_value(ls.find_document_colors(&document, &json, None)),
                _ => panic!("unknown op {kind}"),
            }
            .unwrap();
            checked += 1;
            if actual != op["result"] {
                failures.push(format!(
                    "{name}: {kind} {:?} at {}\n  expected: {}\n  actual:   {}",
                    op["doc"]["text"].as_str().unwrap(),
                    op["args"],
                    op["result"],
                    actual
                ));
            }
        }
    }
    eprintln!("checked {checked} calls, skipped {skipped:?}");
    assert!(
        failures.is_empty(),
        "{} of {checked} calls differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
