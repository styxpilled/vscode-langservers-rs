//! Compares hover and selector printing with upstream. Fixture:
//! `scripts/css/trace-hover.mjs`.

use css_languageservice::HoverSettings;
use css_languageservice::data::CssDataProvider;
use css_languageservice::language_facts::CssDataManager;
use css_languageservice::parser::nodes::{Ast, NodeId, NodeType};
use css_languageservice::parser::{Dialect, Parser, parse_stylesheet};
use css_languageservice::services::hover::{do_hover, supports_markdown};
use css_languageservice::services::selector_printing::{
    ElementId, ElementKind, Elements, SelectorPrinting, selector_to_element, to_element,
};
use lsp_types::{ClientCapabilities, Position};
use serde_json::{Map, Value, json};
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

fn document(language_id: &str, text: &str) -> TextDocument {
    TextDocument::new(
        "test://test/test.css".parse().unwrap(),
        language_id,
        0,
        text.to_owned(),
    )
}

/// The parse of a node and its child index path.
fn locate(n: &Value) -> (Ast, NodeId) {
    let input = n["input"].as_str().unwrap();
    let d = dialect(n["dialect"].as_str().unwrap());
    let (ast, mut node) = match n["fn"].as_str().unwrap() {
        "_parseStylesheet" => {
            let s = parse_stylesheet(d, input);
            (s.ast, s.root)
        }
        f => {
            let mut parser = Parser::new(d, input);
            let node = parser.internal_parse(f).unwrap().unwrap();
            (parser.ast, node)
        }
    };
    for i in n["path"].as_array().unwrap() {
        node = ast.child(node, i.as_u64().unwrap() as usize).unwrap();
    }
    (ast, node)
}

fn dump(elements: &Elements, id: ElementId) -> Value {
    let e = elements.get(id);
    let mut out = Map::new();
    match e.kind {
        ElementKind::Root => _ = out.insert("k".into(), json!("RootElement")),
        ElementKind::Label => _ = out.insert("k".into(), json!("LabelElement")),
        ElementKind::Element => {}
    }
    if let Some(attributes) = &e.attributes {
        let a: Vec<Value> = attributes
            .iter()
            .map(|a| json!([a.name, a.value]))
            .collect();
        out.insert("a".into(), json!(a));
    }
    if let Some(children) = &e.children {
        let c: Vec<Value> = children.iter().map(|&c| dump(elements, c)).collect();
        out.insert("c".into(), json!(c));
    }
    Value::Object(out)
}

fn element_of(ast: &Ast, node: NodeId, selector: bool) -> Value {
    let mut elements = Elements::default();
    if selector {
        selector_to_element(&mut elements, ast, node).map_or(Value::Null, |r| dump(&elements, r))
    } else {
        let e = to_element(&mut elements, ast, node, None);
        dump(&elements, e)
    }
}

#[test]
fn upstream_calls() {
    let calls: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/hover.json")).expect("fixture");
    for c in &calls {
        let actual = match c["fn"].as_str().unwrap() {
            "toElement" | "selectorToElement" => {
                assert!(c["parent"].is_null());
                let (ast, node) = locate(&c["node"]);
                element_of(&ast, node, c["fn"] == "selectorToElement")
            }
            "selectorToMarkedString" | "simpleSelectorToMarkedString" => {
                let (ast, node) = locate(&c["node"]);
                let data = data_manager(c);
                let printing = SelectorPrinting::new(&data);
                let contexts: Vec<String> =
                    serde_json::from_value(c["contexts"].clone()).unwrap_or_default();
                let result = if c["fn"] == "selectorToMarkedString" {
                    printing.selector_to_marked_string(&ast, node, &contexts)
                } else {
                    printing.simple_selector_to_marked_string(&ast, node)
                };
                serde_json::to_value(result).unwrap()
            }
            "doHover" => {
                let input = c["input"].as_str().unwrap();
                let sheet = parse_stylesheet(dialect(c["dialect"].as_str().unwrap()), input);
                let doc = document(c["languageId"].as_str().unwrap(), input);
                let data = data_manager(c);
                let caps: Option<ClientCapabilities> =
                    serde_json::from_value(c["capabilities"].clone()).unwrap();
                let settings: Option<HoverSettings> =
                    serde_json::from_value(c["settings"].clone()).unwrap();
                let position: Position = serde_json::from_value(c["position"].clone()).unwrap();
                let markdown = supports_markdown(caps.as_ref());
                let result = do_hover(&doc, position, &sheet, &data, markdown, settings.as_ref());
                serde_json::to_value(result).unwrap()
            }
            f => panic!("unknown call {f}"),
        };
        assert_eq!(actual, c["result"], "{c}");
    }
}

/// Set `CSS_HOVER_SWEEP` to the sweep file of `trace-hover.mjs`.
#[test]
fn upstream_sweep() {
    let Ok(path) = std::env::var("CSS_HOVER_SWEEP") else {
        return;
    };
    let sweep: Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).expect("sweep");
    let contents = sweep["contents"].as_array().unwrap();
    let data = CssDataManager::new(true, &[]);
    let printing = SelectorPrinting::new(&data);
    let mut failures = 0;
    let mut fail = |what: String| {
        failures += 1;
        if failures <= 10 {
            eprintln!("{what}");
        }
    };
    let results = sweep["results"].as_array().unwrap();
    for r in results {
        let input = r["input"].as_str().unwrap();
        let d = r["dialect"].as_str().unwrap();
        let sheet = parse_stylesheet(dialect(d), input);
        let doc = document(d, input);
        let utf16 = |byte: usize| input[..byte].encode_utf16().count();
        let (hover, plain) = (
            r["hover"].as_array().unwrap(),
            r["plain"].as_array().unwrap(),
        );
        let (mut h, mut p) = (0, 0);
        for byte in (0..=input.len()).filter(|&i| input.is_char_boundary(i)) {
            let offset = utf16(byte);
            let position = doc.position_at(byte);
            let actual = do_hover(&doc, position, &sheet, &data, true, None).map(|x| {
                let range = x.range.unwrap();
                json!([
                    serde_json::to_value(x.contents).unwrap(),
                    utf16(doc.offset_at(range.start)),
                    utf16(doc.offset_at(range.end))
                ])
            });
            let expected = hover[h]
                .as_array()
                .map(|e| json!([contents[e[0].as_u64().unwrap() as usize], e[1], e[2]]));
            if actual != expected {
                fail(format!(
                    "{d} {input:?} hover at {offset}\n  expected {expected:?}\n  actual   {actual:?}"
                ));
            }
            h += 1;
            if offset % 3 == 0 {
                let actual = do_hover(&doc, position, &sheet, &data, false, None)
                    .map(|x| serde_json::to_value(x.contents).unwrap());
                let expected = plain[p].as_u64().map(|i| contents[i as usize].clone());
                if actual != expected {
                    fail(format!(
                        "{d} {input:?} plain hover at {offset}\n  expected {expected:?}\n  actual   {actual:?}"
                    ));
                }
                p += 1;
            }
        }
        assert_eq!((h, p), (hover.len(), plain.len()), "{input:?}");

        let mut selectors = Vec::new();
        sheet.ast.accept(sheet.root, &mut |n| {
            let ty = sheet.ast.node_type(n);
            if ty == NodeType::Selector {
                let marked = printing.selector_to_marked_string(&sheet.ast, n, &["ctx".to_owned()]);
                selectors.push(json!([element_of(&sheet.ast, n, true), marked]));
            }
            if ty == NodeType::SimpleSelector {
                let marked = printing.simple_selector_to_marked_string(&sheet.ast, n);
                selectors.push(json!([element_of(&sheet.ast, n, false), marked]));
            }
            true
        });
        let expected: Vec<Value> = r["selectors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                let s = s.as_array().unwrap();
                json!([
                    contents[s[0].as_u64().unwrap() as usize],
                    contents[s[1].as_u64().unwrap() as usize]
                ])
            })
            .collect();
        if selectors != expected {
            fail(format!(
                "{d} {input:?} selectors\n  expected {expected:?}\n  actual   {selectors:?}"
            ));
        }
    }
    assert_eq!(failures, 0, "{} texts", results.len());
}

/// `docs/differences/css-hover.md`, "Not ported".
#[test]
fn quirks_not_ported() {
    let data = CssDataManager::new(true, &[]);
    let sheet = parse_stylesheet(Dialect::Css, "[o][o] { }");
    let doc = document("css", "[o][o] { }");
    let hover = do_hover(&doc, Position::new(0, 1), &sheet, &data, true, None).unwrap();
    assert_eq!(
        serde_json::to_value(hover.contents).unwrap()[0]["value"],
        "<element o>"
    );
}

fn hover_at(input: &str, offset: usize) -> Option<Value> {
    let data = CssDataManager::new(true, &[]);
    let sheet = parse_stylesheet(Dialect::Css, input);
    let doc = document("css", input);
    do_hover(&doc, doc.position_at(offset), &sheet, &data, true, None)
        .map(|h| serde_json::to_value(h).unwrap())
}

/// `docs/experimental.md`: the name of an unknown at-rule finds its entry.
#[test]
fn unknown_at_rule_by_name() {
    let input = "@view-transition { navigation: auto; }";
    assert!(hover_at("@counter-style x", 2).is_some());
    let hover = hover_at(input, 2).unwrap();
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("view-transition")
    );
    assert_eq!(hover["range"]["end"]["character"], 16);
    assert_eq!(hover_at(input, 20), None);
}

/// `docs/experimental.md`: an element named `…` is not a label.
#[test]
fn element_named_ellipsis() {
    let hover = hover_at(r"\2026 {}", 0).unwrap();
    assert_eq!(hover["contents"][0]["value"], "<\u{2026}>");
}
