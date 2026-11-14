//! Compares the lint entries with upstream for each `LintVisitor.entries` call
//! of the upstream tests. Fixture: `scripts/css/trace-lint.mjs`.

use css_languageservice::LanguageSettings;
use css_languageservice::data::CssDataProvider;
use css_languageservice::language_facts::CssDataManager;
use css_languageservice::parser::nodes::{Ast, Kind, Level, Marker, NodeId};
use css_languageservice::parser::{Dialect, Parser, parse_stylesheet};
use css_languageservice::services::lint::lint_entries;
use css_languageservice::services::lint_rules::{LintConfigurationSettings, LintSettings};
use css_languageservice::services::validation::do_validation;
use serde_json::{Value, json};
use text_document::TextDocument;

fn utf16(text: &str, offset: isize) -> isize {
    if offset < 0 {
        return offset;
    }
    let n = (offset as usize).min(text.len());
    (text[..n].encode_utf16().count() + offset as usize - n) as isize
}

fn dialect(name: &str) -> Dialect {
    match name {
        "scss" => Dialect::Scss,
        "less" => Dialect::Less,
        _ => Dialect::Css,
    }
}

fn encode(ast: &Ast, entries: &[Marker]) -> Value {
    let text = ast.text();
    let entries: Vec<Value> = entries
        .iter()
        .map(|m| {
            let o = utf16(text, m.offset);
            let l = if m.offset < 0 {
                m.length
            } else {
                utf16(text, m.offset + m.length) - o
            };
            json!([m.rule.id, m.level as u8, m.message, o, l])
        })
        .collect();
    json!(entries)
}

const ALL: u8 = Level::Ignore as u8 | Level::Warning as u8 | Level::Error as u8;

#[test]
fn upstream_lint() {
    let calls: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/lint.json")).expect("fixture");
    for c in &calls {
        let providers: Vec<CssDataProvider> = c["customData"]
            .as_array()
            .unwrap()
            .iter()
            .map(CssDataProvider::new)
            .collect();
        let data = CssDataManager::new(c["builtIn"].as_bool().unwrap(), &providers);
        let input = c["input"].as_str().unwrap();
        let mut parser = Parser::new(dialect(c["dialect"].as_str().unwrap()), input);
        let node = parser
            .internal_parse(c["fn"].as_str().unwrap())
            .expect("parse function")
            .expect("node");
        let settings: LintSettings = serde_json::from_value(c["settings"].clone()).unwrap();
        let filter = c["filter"]
            .as_u64()
            .map_or(Level::Warning as u8 | Level::Error as u8, |f| f as u8);
        let entries = lint_entries(
            &parser.ast,
            node,
            LintConfigurationSettings::new(Some(&settings)),
            &data,
            filter,
        );
        assert_eq!(encode(&parser.ast, &entries), c["entries"], "{c}");
    }
}

/// `docs/experimental.md`: upstream parses a hex color of another length as an
/// error, so the declaration is not checked.
fn has_other_hex_color(ast: &Ast, root: NodeId) -> bool {
    let mut found = false;
    ast.accept(root, &mut |n| {
        found |= ast.kind(n) == Kind::HexColorValue && ![4, 5, 7, 9].contains(&ast.length(n));
        !found
    });
    found
}

/// `docs/experimental.md`: upstream never reports `hexColorLength`.
fn without_hex_color_length(entries: Value) -> Value {
    let mut entries = entries.as_array().unwrap().clone();
    entries.retain(|e| e[0] != "hexColorLength");
    Value::Array(entries)
}

/// `docs/experimental.md`: upstream repeats a duplicate declaration for each
/// other declaration of the same name.
fn duplicates_once(entries: &Value) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for e in entries.as_array().unwrap() {
        if e[0] == "duplicateProperties" && out.last() == Some(e) {
            continue;
        }
        out.push(e.clone());
    }
    Value::Array(out)
}

/// Set `CSS_LINT_SWEEP` to the sweep file of `trace-lint.mjs`.
#[test]
fn upstream_sweep() {
    let Ok(path) = std::env::var("CSS_LINT_SWEEP") else {
        return;
    };
    let cases: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).expect("sweep");
    let data = CssDataManager::new(true, &[]);
    let settings: [LintSettings; 2] = [
        LintSettings::new(),
        serde_json::from_value(json!({ "validProperties": ["float", "color", "foo", "width"] }))
            .unwrap(),
    ];
    let (mut failures, mut skipped) = (0, 0);
    for c in &cases {
        let input = c["input"].as_str().unwrap();
        let sheet = parse_stylesheet(dialect(c["dialect"].as_str().unwrap()), input);
        if has_other_hex_color(&sheet.ast, sheet.root) {
            skipped += 1;
            continue;
        }
        for (i, s) in settings.iter().enumerate() {
            let entries = lint_entries(
                &sheet.ast,
                sheet.root,
                LintConfigurationSettings::new(Some(s)),
                &data,
                ALL,
            );
            let actual = without_hex_color_length(encode(&sheet.ast, &entries));
            let expected = &duplicates_once(&c[format!("e{i}")]);
            if &actual != expected {
                failures += 1;
                if failures <= 10 {
                    eprintln!(
                        "{} {input:?} settings {i}\n  expected {expected}\n  actual   {actual}",
                        c["dialect"]
                    );
                }
            }
        }
    }
    eprintln!("{skipped} inputs with hex colors of other lengths skipped");
    assert_eq!(failures, 0, "{} cases", cases.len());
}

/// `CSSValidation.doValidation` calls of the upstream tests and of
/// `trace-lint.mjs` (`fixtures/validation.json`).
#[test]
fn upstream_validation() {
    let calls: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/validation.json")).expect("fixture");
    let uri: lsp_types::Uri = "test://test/test.css".parse().unwrap();
    for c in &calls {
        let providers: Vec<CssDataProvider> = c["customData"]
            .as_array()
            .unwrap()
            .iter()
            .map(CssDataProvider::new)
            .collect();
        let data = CssDataManager::new(c["builtIn"].as_bool().unwrap(), &providers);
        let input = c["input"].as_str().unwrap();
        let document = TextDocument::new(
            uri.clone(),
            c["languageId"].as_str().unwrap(),
            0,
            input.to_owned(),
        );
        let sheet = parse_stylesheet(dialect(c["dialect"].as_str().unwrap()), input);
        let settings: Option<LanguageSettings> =
            serde_json::from_value(c["settings"].clone()).unwrap();
        let result = do_validation(&document, &sheet, settings.as_ref(), &data);
        assert_eq!(serde_json::to_value(&result).unwrap(), c["result"], "{c}");
    }
}

fn messages(d: Dialect, input: &str, lint: Value) -> Vec<String> {
    let data = CssDataManager::new(true, &[]);
    let settings: LanguageSettings = serde_json::from_value(json!({ "lint": lint })).unwrap();
    let sheet = parse_stylesheet(d, input);
    let doc = TextDocument::new(
        "test://t/t.css".parse().unwrap(),
        "css",
        0,
        input.to_owned(),
    );
    do_validation(&doc, &sheet, Some(&settings), &data)
        .iter()
        .map(|x| {
            format!(
                "{}-{} {}",
                x.range.start.character, x.range.end.character, x.message
            )
        })
        .collect()
}

/// `docs/experimental.md`: each duplicate declaration once.
#[test]
fn duplicate_properties_once() {
    let lint = json!({ "duplicateProperties": "warning" });
    let m = messages(
        Dialect::Css,
        "a { color: red; color: blue; color: green }",
        lint,
    );
    assert_eq!(m.len(), 3, "{m:?}");
}

/// `docs/experimental.md`: hex colors of other lengths.
#[test]
fn hex_color_length() {
    let message = "Hex colors must consist of three, four, six or eight hex numbers";
    assert_eq!(
        messages(Dialect::Css, "a { color: #12345 }", json!({})),
        [format!("11-17 {message}")]
    );
    assert_eq!(
        messages(Dialect::Less, "@c: #12345;", json!({})),
        [format!("4-10 {message}")]
    );
    assert!(messages(Dialect::Css, "a { color: #1234 }", json!({})).is_empty());
}

/// `docs/experimental.md`: names are lowercase in all lookups.
#[test]
fn lowercase_names() {
    let m = messages(
        Dialect::Css,
        "a { FOO: 0px }",
        json!({ "zeroUnits": "warning", "validProperties": ["foo"] }),
    );
    assert!(m.is_empty(), "{m:?}");
    let m = messages(
        Dialect::Css,
        "a { -WEBKIT-Transition: none }",
        json!({ "vendorPrefix": "warning" }),
    );
    assert_eq!(
        m,
        ["4-22 Also define the standard property 'transition' for compatibility"]
    );
}
