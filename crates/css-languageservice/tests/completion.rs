//! Compares completion with upstream. Fixture:
//! `scripts/css/trace-completion.mjs`.

use css_languageservice::data::CssDataProvider;
use css_languageservice::language_facts::CssDataManager;
use css_languageservice::parser::{Dialect, parse_stylesheet};
use css_languageservice::services::completion::{
    Completion, CompletionParticipant, CompletionSettings, ImportPathCompletionContext,
    MixinReferenceCompletionContext, PropertyCompletionContext, PropertyValueCompletionContext,
    UriLiteralCompletionContext,
};
use lsp_types::Position;
use serde_json::{Value, json};
use text_document::TextDocument;

fn dialect(name: &str) -> Dialect {
    match name {
        "scss" => Dialect::Scss,
        "less" => Dialect::Less,
        _ => Dialect::Css,
    }
}

fn document(language_id: &str, text: &str) -> TextDocument {
    TextDocument::new(
        format!("test://test/test.{language_id}").parse().unwrap(),
        language_id,
        0,
        text.to_owned(),
    )
}

#[derive(Default)]
struct Recorder {
    property: Vec<Value>,
    property_value: Vec<Value>,
    uri_literal_value: Vec<Value>,
    import_path: Vec<Value>,
    mixin_reference: Vec<Value>,
}

fn v(x: impl serde::Serialize) -> Value {
    serde_json::to_value(x).unwrap()
}

impl CompletionParticipant for Recorder {
    fn on_css_property(&mut self, c: PropertyCompletionContext) {
        self.property.push(v(c));
    }
    fn on_css_property_value(&mut self, c: PropertyValueCompletionContext) {
        self.property_value.push(v(c));
    }
    fn on_css_uri_literal_value(&mut self, c: UriLiteralCompletionContext) {
        self.uri_literal_value.push(v(c));
    }
    fn on_css_import_path(&mut self, c: ImportPathCompletionContext) {
        self.import_path.push(v(c));
    }
    fn on_css_mixin_reference(&mut self, c: MixinReferenceCompletionContext) {
        self.mixin_reference.push(v(c));
    }
}

impl Recorder {
    fn contexts(self) -> Value {
        json!({
            "property": self.property,
            "propertyValue": self.property_value,
            "uriLiteralValue": self.uri_literal_value,
            "importPath": self.import_path,
            "mixinReference": self.mixin_reference,
        })
    }
}

/// The result and the participant contexts.
fn complete(
    d: Dialect,
    data: &CssDataManager,
    markdown: bool,
    doc: &TextDocument,
    position: Position,
    settings: Option<&CompletionSettings>,
) -> (Value, Value) {
    let sheet = parse_stylesheet(d, doc.text());
    let completion = Completion {
        dialect: d,
        data,
        markdown,
    };
    let mut recorder = Recorder::default();
    let result = completion.do_complete(doc, position, &sheet, settings, &mut [&mut recorder]);
    (v(result), recorder.contexts())
}

/// The full result of a fixture call.
fn expand(fixture: &Value, result: &Value) -> Value {
    let (docs, templates) = (&fixture["docs"], &fixture["templates"]);
    let ranges = &result["ranges"];
    let items: Vec<Value> = result["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            let mut item = templates[pair[0].as_u64().unwrap() as usize].clone();
            if let Some(d) = item.get("documentation").and_then(Value::as_u64) {
                item["documentation"] = docs[d as usize].clone();
            }
            if let Some(r) = pair[1].as_u64() {
                item["textEdit"]["range"] = ranges[r as usize].clone();
            }
            item
        })
        .collect();
    let mut out = result.clone();
    let o = out.as_object_mut().unwrap();
    o.remove("ranges");
    o.insert("items".into(), Value::Array(items));
    out
}

fn compare(actual: &Value, expected: &Value, what: &str) {
    let labels = |r: &Value| -> Vec<String> {
        r["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(labels(actual), labels(expected), "labels of {what}");
    for (a, e) in actual["items"]
        .as_array()
        .unwrap()
        .iter()
        .zip(expected["items"].as_array().unwrap())
    {
        assert_eq!(a, e, "item of {what}");
    }
    assert_eq!(actual, expected, "{what}");
}

#[test]
fn upstream_calls() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/completion.json")).expect("fixture");
    for c in fixture["calls"].as_array().unwrap() {
        assert!(c.get("error").is_none(), "{c}");
        let input = c["input"].as_str().unwrap();
        let providers: Vec<CssDataProvider> = c["customData"]
            .as_array()
            .unwrap()
            .iter()
            .map(CssDataProvider::new)
            .collect();
        let data = CssDataManager::new(c["builtIn"].as_bool().unwrap(), &providers);
        let settings: Option<CompletionSettings> =
            serde_json::from_value(c["settings"].clone()).unwrap();
        assert_eq!(c["dialect"], c["parser"]);
        let doc = document(c["languageId"].as_str().unwrap(), input);
        let position: Position = serde_json::from_value(c["position"].clone()).unwrap();
        let (result, contexts) = complete(
            dialect(c["dialect"].as_str().unwrap()),
            &data,
            c["markdown"].as_bool().unwrap(),
            &doc,
            position,
            settings.as_ref(),
        );
        let what = format!("{} {input:?} at {position:?}", c["dialect"]);
        compare(&result, &expand(&fixture, &c["result"]), &what);
        assert_eq!(contexts, c["contexts"], "contexts of {what}");
    }
}

/// JSON with sorted keys.
fn canonical(v: &Value) -> String {
    match v {
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            let fields: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", Value::String((*k).clone()), canonical(&o[*k])))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        _ => v.to_string(),
    }
}

/// cyrb53 over UTF-16 code units.
fn cyrb53(s: &str) -> u64 {
    let (mut h1, mut h2) = (0xdeadbeef_u32, 0x41c6ce57_u32);
    for ch in s.encode_utf16() {
        h1 = (h1 ^ ch as u32).wrapping_mul(2654435761);
        h2 = (h2 ^ ch as u32).wrapping_mul(1597334677);
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2246822507);
    h1 ^= (h2 ^ (h2 >> 13)).wrapping_mul(3266489909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2246822507);
    h2 ^= (h1 ^ (h1 >> 13)).wrapping_mul(3266489909);
    4294967296 * (2097151 & h2 as u64) + h1 as u64
}

/// Set `CSS_COMPLETION_SWEEP` to the sweep file of `trace-completion.mjs`.
#[test]
fn upstream_sweep() {
    let Ok(path) = std::env::var("CSS_COMPLETION_SWEEP") else {
        return;
    };
    let sweep: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).expect("sweep");
    let data = CssDataManager::new(true, &[]);
    let (mut failures, mut errors, mut total) = (0, 0, 0);
    for r in &sweep {
        let input = r["input"].as_str().unwrap();
        let d = r["dialect"].as_str().unwrap();
        let doc = document(d, input);
        let (hashes, plain) = (
            r["hashes"].as_array().unwrap(),
            r["plain"].as_array().unwrap(),
        );
        let (mut h, mut p) = (0, 0);
        for byte in (0..=input.len()).filter(|&i| input.is_char_boundary(i)) {
            let offset = input[..byte].encode_utf16().count();
            let position = doc.position_at(byte);
            let mut check = |expected: &Value, markdown: bool| {
                total += 1;
                if expected == "error" {
                    errors += 1;
                    return;
                }
                let (result, contexts) =
                    complete(dialect(d), &data, markdown, &doc, position, None);
                let actual = canonical(&json!({ "result": result, "contexts": contexts }));
                if Some(cyrb53(&actual)) != expected.as_u64() {
                    failures += 1;
                    if failures <= 10 {
                        let short: String = actual.chars().take(1500).collect();
                        eprintln!("{d} {input:?} at {offset} (markdown {markdown})\n  {short}");
                    }
                }
            };
            check(&hashes[h], true);
            h += 1;
            if offset % 7 == 0 {
                check(&plain[p], false);
                p += 1;
            }
        }
    }
    eprintln!("{total} completions, {errors} upstream errors, {failures} failures");
    assert_eq!(failures, 0);
}
