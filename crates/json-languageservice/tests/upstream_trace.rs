//! Replays the schema calls of the upstream `parser.test.ts` and `schema.test.ts`
//! and compares each result with the upstream result.
//!
//! `fixtures/upstream_trace.json` is written by `scripts/trace/run.mjs`, which runs
//! the compiled upstream tests with a recorder. Request responses that are upstream
//! fixture files are read from `upstream/vscode-json-languageservice/src/test/fixtures`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use json_languageservice::schema::compile_json;
use json_languageservice::{
    DocumentLanguageSettings, JsonSchemaService, JsonValidation, ResolvedSchema, SchemaAssociation,
    SchemaConfiguration, SchemaContributions, SchemaDiagnostic, SchemaDraft, SchemaRequestError,
    SeverityLevel, TextDocument, parse, schema_contributions,
};
use lsp_types::DiagnosticSeverity;
use serde_json::{Value, json};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../upstream/vscode-json-languageservice/src/test/fixtures")
}

type Response = Result<String, SchemaRequestError>;

#[derive(Default)]
struct Requests {
    queues: HashMap<String, VecDeque<Response>>,
    last: HashMap<String, Response>,
    misses: Vec<String>,
}

fn response(op: &Value) -> Option<Response> {
    if let Some(content) = op["content"].as_str() {
        return Some(Ok(content.to_owned()));
    }
    if let Some(name) = op["fixture"].as_str() {
        return std::fs::read_to_string(fixtures_dir().join(name))
            .ok()
            .map(Ok);
    }
    if op["error"].is_object() {
        return Some(Err(SchemaRequestError {
            message: op["error"]["message"].as_str().unwrap_or("").to_owned(),
            code: op["error"]["code"].as_i64(),
        }));
    }
    None
}

struct Svc {
    service: JsonSchemaService,
    requests: Arc<Mutex<Requests>>,
}

fn new_service(id: u64, ops: &[Value]) -> Svc {
    let mine = |o: &&Value| o["svc"].as_u64() == Some(id);
    let has_request = !ops
        .iter()
        .filter(mine)
        .any(|o| o["op"] == "noRequestService");
    let has_context = !ops
        .iter()
        .filter(mine)
        .any(|o| o["op"] == "noContextService");
    let mut requests = Requests::default();
    for op in ops.iter().filter(mine).filter(|o| o["op"] == "request") {
        if let Some(r) = response(op) {
            let uri = op["uri"].as_str().unwrap().to_owned();
            requests.queues.entry(uri).or_default().push_back(r);
        }
    }
    let paths: HashMap<(String, String), String> = ops
        .iter()
        .filter(mine)
        .filter(|o| o["op"] == "resolvePath")
        .map(|o| {
            let s = |k: &str| o[k].as_str().unwrap_or("").to_owned();
            ((s("rel"), s("res")), s("out"))
        })
        .collect();
    let requests = Arc::new(Mutex::new(requests));
    let shared = requests.clone();
    let request = move |uri: &str| -> Response {
        let mut r = shared.lock().unwrap();
        let next = r.queues.get_mut(uri).and_then(VecDeque::pop_front);
        match next {
            Some(resp) => {
                r.last.insert(uri.to_owned(), resp.clone());
                resp
            }
            None => match r.last.get(uri) {
                Some(resp) => resp.clone(),
                None => {
                    r.misses.push(uri.to_owned());
                    Err(SchemaRequestError {
                        message: "Resource not found".to_owned(),
                        code: None,
                    })
                }
            },
        }
    };
    let resolve = move |rel: &str, res: &str| -> String {
        if let Some(out) = paths.get(&(rel.to_owned(), res.to_owned())) {
            return out.clone();
        }
        url::Url::parse(res)
            .and_then(|b| b.join(rel))
            .map_or_else(|_| rel.to_owned(), |u| u.to_string())
    };
    let fallbacks: HashMap<String, Option<Vec<String>>> = ops
        .iter()
        .filter(mine)
        .filter(|o| o["op"] == "fallback")
        .map(|o| {
            let out = o["out"].as_array().map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            });
            (o["resource"].as_str().unwrap().to_owned(), out)
        })
        .collect();
    let fallback = (!fallbacks.is_empty())
        .then(|| Box::new(move |resource: &str| fallbacks.get(resource).cloned().flatten()) as _);
    let service = JsonSchemaService::new(
        has_request.then(|| Box::new(request) as _),
        has_context.then(|| Box::new(resolve) as _),
        fallback,
    );
    Svc { service, requests }
}

fn document(doc: &Value) -> (TextDocument, json_languageservice::JsonDocument) {
    let text = TextDocument::new(
        doc["uri"].as_str().unwrap().parse().unwrap(),
        "json",
        0,
        doc["text"].as_str().unwrap().to_owned(),
    );
    let json = parse(&text, doc["comments"].as_bool().unwrap_or(false));
    (text, json)
}

fn draft(v: &Value) -> Option<SchemaDraft> {
    Some(match v.as_u64()? {
        3 => SchemaDraft::V3,
        4 => SchemaDraft::V4,
        6 => SchemaDraft::V6,
        7 => SchemaDraft::V7,
        19 => SchemaDraft::V2019_09,
        20 => SchemaDraft::V2020_12,
        _ => return None,
    })
}

fn severity_level(v: &Value) -> Option<SeverityLevel> {
    match v.as_str()? {
        "error" => Some(SeverityLevel::Error),
        "warning" => Some(SeverityLevel::Warning),
        "ignore" => Some(SeverityLevel::Ignore),
        _ => None,
    }
}

fn schema_diagnostic(d: &SchemaDiagnostic) -> Value {
    let mut v = json!({ "message": d.message, "code": d.code.0 });
    if let Some(info) = &d.related_information {
        v["relatedInformation"] = serde_json::to_value(info).unwrap();
    }
    v
}

fn resolved_json(svc: &JsonSchemaService, r: Option<&ResolvedSchema>, with_schema: bool) -> Value {
    let Some(r) = r else {
        return Value::Null;
    };
    let mut v = json!({
        "errors": r.errors.iter().map(schema_diagnostic).collect::<Vec<_>>(),
        "warnings": r.warnings.iter().map(schema_diagnostic).collect::<Vec<_>>(),
        "schemaDraft": r.schema_draft.map(|d| d as u32),
        "vocabularies": r.active_vocabularies.as_ref().map(|v| v.iter().map(|(k, b)| json!([k, b])).collect::<Vec<_>>()),
    });
    if with_schema {
        v["schema"] = svc.resolved_schema_json(r, &json!("$circular"));
    }
    v
}

fn contributions(v: &Value) -> SchemaContributions {
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    SchemaContributions {
        schemas: v["schemas"].as_object().cloned().unwrap_or_default(),
        schema_associations: v["schemaAssociations"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| SchemaAssociation {
                        pattern: strings(&s["pattern"]),
                        uris: strings(&s["uris"]),
                        folder_uri: s["folderUri"].as_str().map(str::to_owned),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn contains_marker(v: &Value) -> bool {
    match v {
        Value::String(s) => s == "$circular" || s == "$toolarge",
        Value::Array(a) => a.iter().any(contains_marker),
        Value::Object(o) => o.values().any(contains_marker),
        _ => false,
    }
}

/// Replays one upstream test. Returns the differences.
fn replay(ops: &[Value]) -> Vec<String> {
    let mut services: HashMap<u64, Svc> = HashMap::new();
    let mut validations: HashMap<u64, JsonValidation> = HashMap::new();
    let mut diffs = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let kind = op["op"].as_str().unwrap();
        if matches!(
            kind,
            "request" | "resolvePath" | "noRequestService" | "noContextService" | "fallback"
        ) {
            continue;
        }
        if op.get("threw").is_some() || op.get("rejected").is_some() {
            diffs.push(format!("#{i} {kind}: upstream threw, not replayed"));
            continue;
        }
        let expected = op.get("result").cloned().unwrap_or(Value::Null);
        let actual: Value = if kind == "validate" {
            if contains_marker(&op["schema"]) {
                continue;
            }
            let (text, json_doc) = document(&op["doc"]);
            let (schemas, root) = compile_json(&op["schema"]);
            let severity = match op["severity"].as_u64() {
                Some(1) => DiagnosticSeverity::ERROR,
                Some(3) => DiagnosticSeverity::INFORMATION,
                Some(4) => DiagnosticSeverity::HINT,
                _ => DiagnosticSeverity::WARNING,
            };
            let vocabularies = op["vocabularies"].as_array().map(|a| {
                a.iter()
                    .map(|e| (e[0].as_str().unwrap().to_owned(), e[1].as_bool().unwrap()))
                    .collect()
            });
            let result = json_doc.validate(
                &text,
                &schemas,
                root,
                severity,
                draft(&op["draft"]),
                vocabularies.as_ref(),
            );
            serde_json::to_value(result).unwrap()
        } else {
            let svc_id = op["svc"].as_u64().unwrap();
            let svc = services
                .entry(svc_id)
                .or_insert_with(|| new_service(svc_id, ops));
            let service = &mut svc.service;
            match kind {
                "setSchemaContributions" => {
                    let c = if op["builtin"] == true {
                        schema_contributions()
                    } else {
                        contributions(&op["contributions"])
                    };
                    service.set_schema_contributions(&c);
                    Value::Null
                }
                "registerExternalSchema" => {
                    let c = &op["config"];
                    service.register_external_schema(&SchemaConfiguration {
                        uri: c["uri"].as_str().unwrap().to_owned(),
                        file_match: c["fileMatch"].as_array().map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(str::to_owned))
                                .collect()
                        }),
                        schema: c.get("schema").cloned(),
                        folder_uri: c["folderUri"].as_str().map(str::to_owned),
                    });
                    Value::Null
                }
                "clearExternalSchemas" => {
                    service.clear_external_schemas();
                    Value::Null
                }
                "getResolvedSchema" => {
                    let r = service.get_resolved_schema(op["id"].as_str().unwrap());
                    resolved_json(service, r.as_deref(), true)
                }
                "getSchemaForResource" => {
                    let doc = match op["doc"]["schema"].as_str() {
                        Some(s) => Some(json!({ "$schema": s }).to_string()),
                        None if op["doc"].is_object() => Some("{}".to_owned()),
                        None => None,
                    };
                    let doc = doc.map(|text| {
                        let t =
                            TextDocument::new("foo://bar/x.json".parse().unwrap(), "json", 0, text);
                        parse(&t, false)
                    });
                    let r = service
                        .get_schema_for_resource(op["resource"].as_str().unwrap(), doc.as_ref());
                    resolved_json(service, r.as_deref(), true)
                }
                "onResourceChange" => {
                    Value::Bool(service.on_resource_change(op["uri"].as_str().unwrap()))
                }
                "getMatchingSchemas" => {
                    let (text, json_doc) = document(&op["doc"]);
                    let schema = op["schema"].as_object().map(|_| op["schema"].clone());
                    match service.get_matching_schemas(text.uri().as_str(), &json_doc, schema.as_ref()) {
                        Some((_, matching)) => matching
                            .iter()
                            .map(|m| {
                                let n = &json_doc[m.node];
                                json!({ "offset": n.offset(), "length": n.length(), "inverted": m.inverted })
                            })
                            .collect(),
                        None => json!([]),
                    }
                }
                "configureValidation" => {
                    let raw = &op["raw"];
                    if raw.is_object() {
                        let v = validations
                            .entry(op["validation"].as_u64().unwrap())
                            .or_default();
                        let truthy = |v: &Value| {
                            !matches!(v, Value::Null | Value::Bool(false)) && v != "" && v != 0
                        };
                        v.configure(raw["validate"] != false, truthy(&raw["allowComments"]));
                    }
                    Value::Null
                }
                "doValidation" => {
                    let (text, json_doc) = document(&op["doc"]);
                    let s = &op["settings"];
                    let settings = s.is_object().then(|| DocumentLanguageSettings {
                        comments: severity_level(&s["comments"]),
                        trailing_commas: severity_level(&s["trailingCommas"]),
                        schema_validation: severity_level(&s["schemaValidation"]),
                        schema_request: severity_level(&s["schemaRequest"]),
                        schema_draft: draft(&s["schemaDraft"]),
                    });
                    let v = validations
                        .entry(op["validation"].as_u64().unwrap())
                        .or_default();
                    let schema = (!op["schema"].is_null()).then(|| op["schema"].clone());
                    let result = v.do_validation(
                        service,
                        &text,
                        &json_doc,
                        settings.as_ref(),
                        schema.as_ref(),
                    );
                    serde_json::to_value(result).unwrap()
                }
                "getLanguageStatus" => {
                    let (text, json_doc) = document(&op["doc"]);
                    json!({ "schemas": service.get_schema_uris_for_resource(text.uri().as_str(), Some(&json_doc)) })
                }
                other => panic!("unknown op {other}"),
            }
        };
        if !matches_expected(&actual, &expected) {
            diffs.push(format!(
                "#{i} {kind}\n    expected: {}\n    actual:   {}",
                truncate(&expected),
                truncate(&actual)
            ));
        }
    }
    for (id, svc) in &services {
        let misses = &svc.requests.lock().unwrap().misses;
        if !misses.is_empty() {
            diffs.push(format!("service {id}: unrecorded requests {misses:?}"));
        }
    }
    diffs
}

/// Compares with an upstream result. `$toolarge` matches anything.
fn matches_expected(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (_, Value::String(s)) if s == "$toolarge" => true,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| matches_expected(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && b.iter()
                    .all(|(k, v)| a.get(k).is_some_and(|x| matches_expected(x, v)))
        }
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::String(a), Value::String(b)) => without_untitled_ids(a) == without_untitled_ids(b),
        _ => actual == expected,
    }
}

/// Upstream numbers untitled schemas with one counter for all tests.
fn without_untitled_ids(s: &str) -> String {
    const PREFIX: &str = "schemaservice://untitled/";
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(PREFIX) {
        out.push_str(&rest[..i + PREFIX.len()]);
        rest = rest[i + PREFIX.len()..].trim_start_matches(|c: char| c.is_ascii_digit());
    }
    out.push_str(rest);
    out
}

fn truncate(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 2000 {
        format!("{}...", &s[..s.floor_char_boundary(2000)])
    } else {
        s
    }
}

/// Tests that pass one JS object at two places of a schema. JSON input cannot
/// express that, and the server receives only JSON.
const SKIPPED: &[&str] = &[
    "schema / JSON Schema / $dynamicAnchor on a sub-schema shared by reference is registered once",
];

#[test]
fn upstream_trace() {
    let trace: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/upstream_trace.json")).unwrap();
    let has_fixtures = fixtures_dir().exists();
    let mut failures = Vec::new();
    let mut skipped = 0;
    for test in &trace {
        let ops = test["ops"].as_array().unwrap();
        if !has_fixtures && ops.iter().any(|o| o.get("fixture").is_some()) {
            skipped += 1;
            continue;
        }
        let name = test["name"].as_str().unwrap();
        if SKIPPED.contains(&name) {
            continue;
        }
        let result = std::panic::catch_unwind(|| replay(ops));
        match result {
            Ok(diffs) if diffs.is_empty() => {}
            Ok(diffs) => failures.push(format!("{name}\n  {}", diffs.join("\n  "))),
            Err(_) => failures.push(format!("{name}\n  panic")),
        }
    }
    if skipped > 0 {
        eprintln!(
            "skipped {skipped} tests: {} not found",
            fixtures_dir().display()
        );
    }
    assert!(
        failures.is_empty(),
        "{} of {} failed:\n{}",
        failures.len(),
        trace.len(),
        failures.join("\n")
    );
}
