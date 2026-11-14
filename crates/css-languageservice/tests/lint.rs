//! Compares the lint entries with upstream for each `LintVisitor.entries` call
//! of the upstream tests. Fixture: `scripts/css/trace-lint.mjs`.

use css_languageservice::LanguageSettings;
use css_languageservice::data::CssDataProvider;
use css_languageservice::language_facts::CssDataManager;
use css_languageservice::parser::nodes::{Ast, Level, Marker};
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
    let mut failures = 0;
    for c in &cases {
        let input = c["input"].as_str().unwrap();
        let sheet = parse_stylesheet(dialect(c["dialect"].as_str().unwrap()), input);
        for (i, s) in settings.iter().enumerate() {
            let entries = lint_entries(
                &sheet.ast,
                sheet.root,
                LintConfigurationSettings::new(Some(s)),
                &data,
                ALL,
            );
            let actual = encode(&sheet.ast, &entries);
            let expected = &c[format!("e{i}")];
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
