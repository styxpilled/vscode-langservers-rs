//! Each feature at each offset of many texts, compared with upstream, and
//! robustness on prefixes of non-ASCII variants of the texts.
//! Sweep: `scripts/html/trace-sweep.mjs`.

use css_languageservice::services::links::{
    DocumentContext, FileStat, FileSystemProvider, FileType,
};
use html_languageservice::LanguageService;
use html_languageservice::language_facts::HtmlDataManager;
use html_languageservice::services::completion::{
    CompletionParticipant, HtmlAttributeValueContext,
};
use html_languageservice::services::{links, navigation};
use html_languageservice::{HoverSettings, HtmlDocument};
use lsp_types::{ClientCapabilities, Position};
use serde_json::{Value, json};
use text_document::TextDocument;

struct Context;

impl DocumentContext for Context {
    fn resolve_reference(&self, reference: &str, base_url: &str) -> Option<String> {
        (reference.encode_utf16().count() % 5 != 4).then(|| format!("{base_url}|{reference}"))
    }
}

struct VirtualFs;

impl FileSystemProvider for VirtualFs {
    fn stat(&self, _: &str) -> Option<FileStat> {
        unreachable!()
    }

    fn read_directory(&self, uri: &str) -> Option<Vec<(String, FileType)>> {
        if uri.encode_utf16().count() % 3 == 2 {
            return Some(Vec::new());
        }
        use FileType::*;
        let entries = [
            ("a.html", File),
            ("b", Directory),
            ("c.css", File),
            (".hidden", File),
            ("img.png", File),
            ("icon.SVG", File),
            ("d.js", File),
            ("v.mp4", File),
            ("s.mp3", File),
            ("t.vtt", File),
            ("é 😀", Directory),
            ("link", SymbolicLink),
        ];
        Some(entries.map(|(n, t)| (n.to_owned(), t)).to_vec())
    }

    fn get_content(&self, _: &str) -> Option<String> {
        unreachable!()
    }
}

#[derive(Default)]
struct Recorder(Vec<Value>);

impl CompletionParticipant for Recorder {
    fn on_html_attribute_value(&mut self, c: &HtmlAttributeValueContext) {
        let attributes = c
            .attributes
            .as_ref()
            .map(|a| Value::Object(a.iter().map(|(k, v)| (k.clone(), json!(v))).collect()));
        self.0.push(json!({"attributeValue": {
            "tag": c.tag, "attribute": c.attribute, "value": c.value, "range": c.range, "attributes": attributes,
        }}));
    }

    fn on_html_content(&mut self, position: Position) {
        self.0.push(json!({"content": {"position": position}}));
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

fn v<T: serde::Serialize>(x: T) -> Value {
    serde_json::to_value(x).unwrap()
}

fn document(text: &str) -> TextDocument {
    TextDocument::new(
        "test://test/dir/test.html".parse().unwrap(),
        "html",
        1,
        text.to_owned(),
    )
}

fn complete(ls: &LanguageService, doc: &TextDocument, p: Position, html: &HtmlDocument) -> Value {
    let mut recorder = Recorder::default();
    let result = ls.do_complete2(
        doc,
        p,
        html,
        &Context,
        Some(&VirtualFs),
        None,
        &mut [&mut recorder],
    );
    json!({"result": result, "contexts": recorder.0})
}

/// Path completion in a value that is not quoted on one line: the branch takes
/// the replace range from offsets, upstream from `shiftRange(range, 1, -1)`.
fn changed_path_range(doc: &TextDocument, actual: &Value) -> bool {
    actual["contexts"].as_array().is_some_and(|contexts| {
        contexts.iter().any(|c| {
            let c = &c["attributeValue"];
            if !c.is_object()
                || !HtmlDataManager::is_path_attribute(
                    c["tag"].as_str().unwrap(),
                    c["attribute"].as_str().unwrap(),
                )
            {
                return false;
            }
            let value = doc.text_in(serde_json::from_value(c["range"].clone()).unwrap());
            let b = value.as_bytes();
            let quoted =
                b.len() > 1 && matches!((b[0], b[b.len() - 1]), (b'"', b'"') | (b'\'', b'\''));
            !quoted || value.contains('\n')
        })
    })
}

/// The results at a position, in the order of `features` of the sweep.
fn features(
    ls: &LanguageService,
    doc: &TextDocument,
    p: Position,
    html: &HtmlDocument,
) -> [Value; 9] {
    [
        complete(ls, doc, p, html),
        v(ls.do_hover(doc, p, html, None)),
        v(ls.do_quote_complete(doc, p, html, None)),
        v(ls.do_tag_complete(doc, p, html)),
        v(navigation::find_document_highlights(doc, p, html)),
        v(navigation::do_rename(doc, p, "newName", html)),
        v(navigation::find_matching_tag_position(doc, p, html)),
        v(navigation::find_linked_editing_ranges(doc, p, html)),
        v(ls.get_selection_ranges(doc, &[p])),
    ]
}

fn document_features(ls: &LanguageService, doc: &TextDocument, html: &HtmlDocument) -> Value {
    json!({
        "folding": ls.get_folding_ranges(doc, None),
        "folding3": ls.get_folding_ranges(doc, Some(3)),
        "symbols": navigation::find_document_symbols(doc, html),
        "symbols2": navigation::find_document_symbols2(doc, html),
        "links": links::find_document_links(doc, &Context),
    })
}

fn plain_service() -> LanguageService {
    LanguageService::new(true, Vec::new(), Some(&ClientCapabilities::default()))
}

/// `HTML_SWEEP=<absolute path of .scratch/data/html-sweep.json>`.
#[test]
fn upstream_sweep() {
    let Ok(path) = std::env::var("HTML_SWEEP") else {
        return;
    };
    let sweep: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let names = sweep["features"].as_array().unwrap();
    let ls = LanguageService::default();
    let plain = plain_service();
    let (mut total, mut skipped, mut failures) = (0, 0, 0);
    let mut fail = |what: String| {
        failures += 1;
        if failures <= 20 {
            eprintln!("{what}");
        }
    };
    for r in sweep["results"].as_array().unwrap() {
        let text = r["text"].as_str().unwrap();
        let doc = document(text);
        let html = ls.parse_html_document(&doc);
        let actual = document_features(&ls, &doc, &html);
        for (k, expected) in r["document"].as_object().unwrap() {
            total += 1;
            if &actual[k] != expected {
                fail(format!(
                    "{k} {text:?}\n  expected {expected}\n  actual   {}",
                    actual[k]
                ));
            }
        }
        let (offsets, plain_offsets) = (
            r["offsets"].as_array().unwrap(),
            r["plain"].as_array().unwrap(),
        );
        let (mut i, mut j) = (0, 0);
        for byte in (0..=text.len()).filter(|&b| text.is_char_boundary(b)) {
            let offset = text[..byte].encode_utf16().count();
            let p = doc.position_at(byte);
            let mut check = |name: &Value, expected: &Value, actual: &Value| {
                total += 1;
                if expected == "error" || changed_path_range(&doc, actual) {
                    skipped += 1;
                    return;
                }
                if Some(cyrb53(&canonical(actual))) != expected.as_u64() {
                    let s: String = actual.to_string().chars().take(1500).collect();
                    fail(format!("{name} {text:?} at {offset}\n  actual {s}"));
                }
            };
            for (k, actual) in features(&ls, &doc, p, &html).iter().enumerate() {
                check(&names[k], &offsets[i][k], actual);
            }
            i += 1;
            if offset % 7 == 0 {
                let e = &plain_offsets[j];
                check(
                    &json!("plain complete"),
                    &e[0],
                    &complete(&plain, &doc, p, &html),
                );
                let hover = plain.do_hover(
                    &doc,
                    p,
                    &html,
                    Some(HoverSettings {
                        documentation: None,
                        references: Some(false),
                    }),
                );
                check(&json!("plain hover"), &e[1], &v(hover));
                j += 1;
            }
        }
        assert_eq!(i, offsets.len(), "offsets of {text:?}");
    }
    eprintln!("{total} results, {skipped} path completions skipped, {failures} failures");
    assert_eq!(failures, 0);
}

/// Non-ASCII variants of a text: letters replaced with `é` and `😀`, and a
/// lone `<` and `&` added.
fn variants(text: &str) -> Vec<String> {
    let mut a = String::new();
    let mut b = String::new();
    for (i, c) in text.chars().enumerate() {
        a.push(if c.is_ascii_alphabetic() && i % 3 == 0 {
            'é'
        } else {
            c
        });
        b.push_str(match c {
            c if c.is_ascii_alphabetic() && i % 4 == 1 => "😀",
            '"' if i % 2 == 0 => "&😀",
            '>' if i % 5 == 0 => "<😀",
            _ => {
                b.push(c);
                continue;
            }
        });
    }
    vec![a, b]
}

/// No panics: each feature at each offset of each prefix of the non-ASCII
/// variants of the sweep texts.
#[test]
fn robustness() {
    let Ok(path) = std::env::var("HTML_SWEEP") else {
        return;
    };
    let sweep: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let ls = LanguageService::default();
    let plain = plain_service();
    for r in sweep["results"].as_array().unwrap() {
        for text in variants(r["text"].as_str().unwrap()) {
            for (end, _) in text.char_indices().chain([(text.len(), ' ')]) {
                let doc = document(&text[..end]);
                let html = ls.parse_html_document(&doc);
                document_features(&ls, &doc, &html);
                for byte in (0..=end).filter(|&b| text.is_char_boundary(b)) {
                    let p = doc.position_at(byte);
                    features(&ls, &doc, p, &html);
                    if byte % 7 == 0 {
                        plain.do_hover(&doc, p, &html, None);
                    }
                }
            }
        }
    }
}
