//! Compares navigation (without file system), code actions, folding and
//! selection ranges with upstream. Fixture:
//! `scripts/css/trace-navigation.mjs`.

use css_languageservice::data::CssDataProvider;
use css_languageservice::language_facts::CssDataManager;
use css_languageservice::language_facts::colors::Color;
use css_languageservice::parser::{Dialect, Stylesheet, parse_stylesheet};
use css_languageservice::services::code_actions::{do_code_actions, do_code_actions2};
use css_languageservice::services::folding::get_folding_ranges;
use css_languageservice::services::navigation::{
    do_rename, find_definition, find_document_colors, find_document_highlights,
    find_document_symbols, find_references, find_symbol_informations, get_color_presentations,
    prepare_rename,
};
use css_languageservice::services::selection_range::get_selection_ranges;
use lsp_types::{Diagnostic, Position, Range};
use serde_json::{Value, json};
use text_document::TextDocument;

fn dialect(name: &str) -> Dialect {
    match name {
        "scss" => Dialect::Scss,
        "less" => Dialect::Less,
        _ => Dialect::Css,
    }
}

fn data_manager(c: &Value) -> CssDataManager {
    let providers: Vec<CssDataProvider> = c["customData"]
        .as_array()
        .unwrap()
        .iter()
        .map(CssDataProvider::new)
        .collect();
    CssDataManager::new(c["builtIn"].as_bool().unwrap(), &providers)
}

fn document(d: &Value) -> TextDocument {
    TextDocument::new(
        d["uri"].as_str().unwrap().parse().unwrap(),
        d["languageId"].as_str().unwrap(),
        d["version"].as_i64().unwrap() as i32,
        d["text"].as_str().unwrap().to_owned(),
    )
}

fn stylesheet(c: &Value) -> Stylesheet {
    parse_stylesheet(
        dialect(c["dialect"].as_str().unwrap()),
        c["input"].as_str().unwrap(),
    )
}

fn arg<T: serde::de::DeserializeOwned>(c: &Value, name: &str) -> T {
    serde_json::from_value(c[name].clone()).unwrap()
}

fn to_value(v: impl serde::Serialize) -> Value {
    serde_json::to_value(v).unwrap()
}

#[test]
fn upstream_calls() {
    let calls: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/navigation.json")).expect("fixture");
    for c in &calls {
        let actual = match c["fn"].as_str().unwrap() {
            "getColorPresentations" => {
                to_value(get_color_presentations(arg(c, "color"), arg(c, "range")))
            }
            "getFoldingRanges" => {
                let limit = c["context"]["rangeLimit"].as_u64().map(|l| l as usize);
                to_value(get_folding_ranges(&document(&c["document"]), limit))
            }
            f => {
                let doc = document(&c["document"]);
                let sheet = stylesheet(c);
                match f {
                    "findDefinition" => to_value(find_definition(&doc, arg(c, "position"), &sheet)),
                    "findReferences" => to_value(find_references(&doc, arg(c, "position"), &sheet)),
                    "findDocumentHighlights" => {
                        to_value(find_document_highlights(&doc, arg(c, "position"), &sheet))
                    }
                    "prepareRename" => to_value(prepare_rename(&doc, arg(c, "position"), &sheet)),
                    "doRename" => {
                        let name: String = arg(c, "newName");
                        to_value(do_rename(&doc, arg(c, "position"), &name, &sheet))
                    }
                    "findSymbolInformations" => to_value(find_symbol_informations(&doc, &sheet)),
                    "findDocumentSymbols" => to_value(find_document_symbols(&doc, &sheet)),
                    "findDocumentColors" => to_value(find_document_colors(&doc, &sheet)),
                    "getSelectionRanges" => {
                        let positions: Vec<Position> = arg(c, "positions");
                        to_value(get_selection_ranges(&doc, &positions, &sheet))
                    }
                    "doCodeActions" | "doCodeActions2" => {
                        let diagnostics: Vec<Diagnostic> = arg(c, "diagnostics");
                        let data = data_manager(c);
                        if f == "doCodeActions" {
                            to_value(do_code_actions(&doc, &diagnostics, &sheet, &data))
                        } else {
                            to_value(do_code_actions2(&doc, &diagnostics, &sheet, &data))
                        }
                    }
                    f => panic!("unknown call {f}"),
                }
            }
        };
        assert_eq!(actual, c["result"], "{c}");
    }
}

/// Set `CSS_NAVIGATION_SWEEP` to the sweep file of `trace-navigation.mjs`.
#[test]
fn upstream_sweep() {
    let Ok(path) = std::env::var("CSS_NAVIGATION_SWEEP") else {
        return;
    };
    let sweep: Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).expect("sweep");
    let data = CssDataManager::new(true, &[]);
    let mut failures = 0;
    let mut fail = |what: String| {
        failures += 1;
        if failures <= 20 {
            eprintln!("{what}");
        }
    };
    let results = sweep["results"].as_array().unwrap();
    for r in results {
        let input = r["input"].as_str().unwrap();
        let d = r["dialect"].as_str().unwrap();
        let sheet = parse_stylesheet(dialect(d), input);
        let doc = document(&json!({
            "uri": format!("test://test/test.{d}"), "languageId": d, "version": 1, "text": input
        }));
        let utf16 = |pos: Position| input[..doc.offset_at(pos)].encode_utf16().count();
        let range = |r: Range| json!([utf16(r.start), utf16(r.end)]);
        let mut check = |what: &str, offset: usize, actual: Value, expected: &Value| {
            if actual != *expected {
                fail(format!(
                    "{d} {input:?} {what} at {offset}\n  expected {expected}\n  actual   {actual}"
                ));
            }
        };

        let mut i = 0;
        for byte in (0..=input.len()).filter(|&b| input.is_char_boundary(b)) {
            let offset = input[..byte].encode_utf16().count();
            let position = doc.position_at(byte);
            let highlights: Vec<Value> = find_document_highlights(&doc, position, &sheet)
                .into_iter()
                .map(|h| {
                    let mut r = range(h.range);
                    r.as_array_mut().unwrap().push(to_value(h.kind.unwrap()));
                    r
                })
                .collect();
            check("highlights", offset, json!(highlights), &r["highlights"][i]);
            let definition = find_definition(&doc, position, &sheet).map(|l| range(l.range));
            check(
                "definition",
                offset,
                to_value(definition),
                &r["definition"][i],
            );
            let rename = prepare_rename(&doc, position, &sheet).map(range);
            check("rename", offset, to_value(rename), &r["rename"][i]);
            let mut chain = Vec::new();
            let selection = get_selection_ranges(&doc, &[position], &sheet).remove(0);
            let mut s = Some(&selection);
            while let Some(x) = s {
                chain.extend(range(x.range).as_array().unwrap().clone());
                s = x.parent.as_deref();
            }
            check("selection", offset, json!(chain), &r["selection"][i]);
            i += 1;
        }
        assert_eq!(i, r["highlights"].as_array().unwrap().len(), "{input:?}");

        check(
            "symbols",
            0,
            to_value(find_document_symbols(&doc, &sheet)),
            &r["symbols"],
        );
        let infos = to_value(find_symbol_informations(&doc, &sheet));
        check("symbol infos", 0, infos, &r["symbolInfos"]);
        let colors = find_document_colors(&doc, &sheet);
        let labels: Vec<Vec<String>> = colors
            .iter()
            .map(|c| {
                get_color_presentations(c.color, c.range)
                    .into_iter()
                    .map(|p| p.label)
                    .collect()
            })
            .collect();
        check("colors", 0, to_value(&colors), &r["colors"]);
        check("presentations", 0, to_value(labels), &r["presentations"]);
        check(
            "folding",
            0,
            to_value(get_folding_ranges(&doc, None)),
            &r["folding"],
        );
        let diagnostics: Vec<Diagnostic> = arg(r, "diagnostics");
        let actions = do_code_actions2(&doc, &diagnostics, &sheet, &data);
        check("code actions", 0, to_value(actions), &r["codeActions"]);
    }

    let range = Range::new(Position::new(0, 0), Position::new(0, 1));
    for p in sweep["presentations"].as_array().unwrap() {
        let color: Color = arg(p, "color");
        let labels: Vec<String> = get_color_presentations(color, range)
            .into_iter()
            .map(|p| p.label)
            .collect();
        if to_value(&labels) != p["labels"] {
            fail(format!(
                "presentations of {color:?}\n  expected {}\n  actual   {labels:?}",
                p["labels"]
            ));
        }
    }
    assert_eq!(failures, 0, "{} texts", results.len());
}

/// Runs each feature at each offset of the parser sweep inputs: no panics.
/// Set `CSS_SWEEP_INPUTS` to `.scratch/data/css-sweep-inputs.json`.
#[test]
fn robustness() {
    let Ok(path) = std::env::var("CSS_SWEEP_INPUTS") else {
        return;
    };
    let inputs: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).expect("inputs");
    let data = CssDataManager::new(true, &[]);
    for c in inputs.iter().filter(|c| c["fn"] == "_parseStylesheet") {
        let input = c["input"].as_str().unwrap();
        let d = c["dialect"].as_str().unwrap();
        let sheet = stylesheet(c);
        let doc = document(&json!({
            "uri": "test://test/test.css", "languageId": d, "version": 1, "text": input
        }));
        for byte in (0..=input.len()).filter(|&b| input.is_char_boundary(b)) {
            let position = doc.position_at(byte);
            find_document_highlights(&doc, position, &sheet);
            find_definition(&doc, position, &sheet);
            do_rename(&doc, position, "x", &sheet);
            prepare_rename(&doc, position, &sheet);
            get_selection_ranges(&doc, &[position], &sheet);
        }
        find_document_symbols(&doc, &sheet);
        find_symbol_informations(&doc, &sheet);
        for c in find_document_colors(&doc, &sheet) {
            get_color_presentations(c.color, c.range);
        }
        get_folding_ranges(&doc, None);
        let diagnostics =
            css_languageservice::services::validation::do_validation(&doc, &sheet, None, &data);
        do_code_actions(&doc, &diagnostics, &sheet, &data);
    }
}

/// `docs/differences/css-navigation.md`, "Not ported": upstream suggests
/// `bleed`, `fill` and `flex` for `rule`.
#[test]
fn quirks_not_ported() {
    let data = CssDataManager::new(true, &[]);
    let text = ".a { rule: 1 }";
    let sheet = parse_stylesheet(Dialect::Css, text);
    let doc = document(&json!({
        "uri": "test://test/test.css", "languageId": "css", "version": 0, "text": text
    }));
    let diagnostics =
        css_languageservice::services::validation::do_validation(&doc, &sheet, None, &data);
    let titles: Vec<String> = do_code_actions2(&doc, &diagnostics, &sheet, &data)
        .into_iter()
        .map(|a| a.title)
        .collect();
    assert_eq!(titles, ["Rename to 'flex'", "Rename to 'left'"]);
}
