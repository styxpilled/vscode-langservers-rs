//! Replays the scanner, parser and feature calls of the upstream tests.
//! Fixture: `scripts/html/trace.mjs`.

use std::sync::Arc;

use css_languageservice::services::links::{
    DocumentContext, FileStat, FileSystemProvider, FileType,
};
use html_languageservice::parser::scanner::{Scanner, ScannerState, TokenType};
use html_languageservice::parser::{HtmlDocument, NodeId};
use html_languageservice::services::completion::{
    CompletionConfiguration, CompletionParticipant, HtmlAttributeValueContext,
};
use html_languageservice::services::{formatter, links, navigation};
use html_languageservice::{HoverSettings, HtmlDataManager, HtmlDataProvider, LanguageService};
use lsp_types::{Position, Uri};
use serde_json::{Value, json};
use text_document::TextDocument;

fn fixture() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/upstream.json"
    ))
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// UTF-16 offset of a byte offset.
fn utf16(src: &str, offset: usize) -> usize {
    src[..offset].encode_utf16().count()
}

/// Byte offset of a UTF-16 offset, `None` inside a surrogate pair.
fn byte_offset(src: &str, utf16_offset: usize) -> Option<usize> {
    let mut units = 0;
    for (i, c) in src.char_indices() {
        if units == utf16_offset {
            return Some(i);
        }
        units += c.len_utf16();
        if units > utf16_offset {
            return None;
        }
    }
    (units == utf16_offset).then_some(src.len())
}

fn state_of(name: &str) -> ScannerState {
    use ScannerState::*;
    [
        WithinContent,
        AfterOpeningStartTag,
        AfterOpeningEndTag,
        WithinDoctype,
        WithinTag,
        WithinEndTag,
        WithinComment,
        WithinScriptContent,
        WithinStyleContent,
        AfterAttributeName,
        BeforeAttributeValue,
    ]
    .into_iter()
    .find(|s| format!("{s:?}") == name)
    .unwrap()
}

#[test]
fn scanner() {
    let f = fixture();
    let mut failures = 0;
    for case in f["scanner"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        if case["splitsSurrogatePair"].as_bool().unwrap() {
            continue;
        }
        let Some(offset) = byte_offset(input, case["initialOffset"].as_u64().unwrap() as usize)
        else {
            continue;
        };
        let mut scanner = Scanner::new(
            input,
            offset,
            state_of(case["initialState"].as_str().unwrap()),
            case["emitPseudoCloseTags"].as_bool().unwrap(),
        );
        let mut tokens = Vec::new();
        loop {
            let t = scanner.scan();
            let start = utf16(input, scanner.token_offset());
            tokens.push(json!([
                format!("{t:?}").to_lowercase(),
                start,
                utf16(input, scanner.token_end()) - start,
                scanner.token_text(),
                scanner.token_error(),
                format!("{:?}", scanner.scanner_state()),
            ]));
            if t == TokenType::Eos {
                break;
            }
        }
        let expected: Vec<Value> = case["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                let mut t = t.clone();
                t[0] = Value::from(t[0].as_str().unwrap().to_lowercase());
                t
            })
            .collect();
        if tokens != expected {
            failures += 1;
            if failures <= 10 {
                eprintln!("{input:?}\n  expected {expected:?}\n  actual   {tokens:?}");
            }
        }
    }
    assert_eq!(failures, 0);
}

fn node_json(doc: &HtmlDocument, text: &str, id: NodeId) -> Value {
    let n = &doc[id];
    let u = |o: usize| utf16(text, o);
    json!({
        "tag": n.tag,
        "start": u(n.start),
        "end": u(n.end),
        "startTagEnd": n.start_tag_end.map(u),
        "endTagStart": n.end_tag_start.map(u),
        "closed": n.closed,
        "attributes": n.attributes.as_ref().map(|a| a.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>()),
        "children": n.children.iter().map(|&c| node_json(doc, text, c)).collect::<Vec<_>>(),
    })
}

fn node_path(doc: &HtmlDocument, mut id: NodeId) -> Value {
    let mut path = Vec::new();
    while !doc.is_document(id) {
        let parent = doc[id].parent.unwrap();
        path.insert(
            0,
            doc[parent].children.iter().position(|&c| c == id).unwrap(),
        );
        id = parent;
    }
    json!(path)
}

#[test]
fn parser() {
    let f = fixture();
    let mut failures = 0;
    for case in f["parser"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        let void: Vec<String> = serde_json::from_value(case["voidElements"].clone()).unwrap();
        let doc = HtmlDocument::parse(
            text,
            |t| HtmlDataManager::is_void_element(t, &void),
            case["orphans"].as_bool().unwrap(),
        );
        let roots: Vec<Value> = doc
            .roots()
            .iter()
            .map(|&r| node_json(&doc, text, r))
            .collect();
        let mut ok = Value::from(roots.clone()) == case["roots"];
        for i in 0..=text.encode_utf16().count() {
            let Some(b) = byte_offset(text, i) else {
                continue;
            };
            ok &= node_path(&doc, doc.find_node_before(b)) == case["before"][i];
            ok &= node_path(&doc, doc.find_node_at(b)) == case["at"][i];
        }
        if !ok {
            failures += 1;
            if failures <= 10 {
                eprintln!(
                    "{text:?}\n  expected {}\n  actual   {}",
                    case["roots"],
                    json!(roots)
                );
            }
        }
    }
    assert_eq!(failures, 0);
}

struct RecordedContext<'a>(&'a Value);

impl DocumentContext for RecordedContext<'_> {
    fn resolve_reference(&self, reference: &str, base_url: &str) -> Option<String> {
        let log = self.0.as_array().unwrap();
        let entry = log
            .iter()
            .find(|e| e["ref"] == reference && e["base"] == base_url)
            .unwrap_or_else(|| panic!("unrecorded resolveReference({reference:?}, {base_url:?})"));
        entry["result"].as_str().map(str::to_owned)
    }
}

struct RecordedFs<'a>(&'a Value);

impl FileSystemProvider for RecordedFs<'_> {
    fn stat(&self, _: &str) -> Option<FileStat> {
        unreachable!()
    }

    fn read_directory(&self, uri: &str) -> Option<Vec<(String, FileType)>> {
        let entry = self
            .0
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["uri"] == uri)
            .unwrap_or_else(|| panic!("unrecorded readDirectory({uri:?})"));
        let items = entry["result"].as_array()?;
        Some(
            items
                .iter()
                .map(|i| {
                    let t = match i[1].as_u64().unwrap() {
                        1 => FileType::File,
                        2 => FileType::Directory,
                        64 => FileType::SymbolicLink,
                        _ => FileType::Unknown,
                    };
                    (i[0].as_str().unwrap().to_owned(), t)
                })
                .collect(),
        )
    }

    fn get_content(&self, _: &str) -> Option<String> {
        unreachable!()
    }
}

struct Recorder {
    attribute_value: bool,
    content: bool,
    events: Vec<Value>,
}

impl CompletionParticipant for Recorder {
    fn on_html_attribute_value(&mut self, c: &HtmlAttributeValueContext) {
        if !self.attribute_value {
            return;
        }
        let attributes = c
            .attributes
            .as_ref()
            .map(|a| Value::Object(a.iter().map(|(k, v)| (k.clone(), json!(v))).collect()));
        self.events.push(json!({"attributeValue": {
            "tag": c.tag, "attribute": c.attribute, "value": c.value, "range": c.range, "attributes": attributes,
        }}));
    }

    fn on_html_content(&mut self, position: Position) {
        if self.content {
            self.events.push(json!({"content": {"position": position}}));
        }
    }
}

fn service(call: &Value) -> LanguageService {
    let providers = &call["providers"];
    let custom = providers["custom"]
        .as_array()
        .map(|c| {
            c.iter()
                .map(|p| Arc::new(HtmlDataProvider::new(p["id"].as_str().unwrap(), &p["data"])))
                .collect()
        })
        .unwrap_or_default();
    LanguageService::new(providers["builtIn"].as_bool().unwrap_or(true), custom, None)
}

fn run(call: &Value) -> Option<Value> {
    let d = &call["document"];
    let text = d["text"].as_str().unwrap();
    let uri: Uri = d["uri"].as_str().unwrap().parse().unwrap();
    let document = TextDocument::new(uri, d["languageId"].as_str().unwrap(), 0, text.to_owned());
    let ls = service(call);
    let html = ls.parse_html_document(&document);
    let position = || serde_json::from_value::<Position>(call["position"].clone()).unwrap();
    let settings = (!call["settings"].is_null())
        .then(|| CompletionConfiguration::from_json(&call["settings"]));
    fn v<T: serde::Serialize>(x: T) -> Value {
        serde_json::to_value(x).unwrap()
    }
    Some(match call["fn"].as_str().unwrap() {
        "doComplete" => {
            let handlers = &call["participantHandlers"];
            let mut recorder = Recorder {
                attribute_value: handlers["attributeValue"].as_bool().unwrap_or(false),
                content: handlers["content"].as_bool().unwrap_or(false),
                events: Vec::new(),
            };
            let has_participants = call.get("participants").is_some();
            let result = if has_participants {
                ls.do_complete(
                    &document,
                    position(),
                    &html,
                    settings.as_ref(),
                    &mut [&mut recorder],
                )
            } else {
                ls.do_complete(&document, position(), &html, settings.as_ref(), &mut [])
            };
            if has_participants && json!(recorder.events) != call["participants"] {
                return Some(json!({"participants": recorder.events}));
            }
            v(result)
        }
        "doComplete2" => {
            let fs = RecordedFs(&call["readDirectory"]);
            let fs: Option<&dyn FileSystemProvider> =
                call["hasFileSystem"].as_bool().unwrap().then_some(&fs);
            v(ls.do_complete2(
                &document,
                position(),
                &html,
                &RecordedContext(&call["resolveReference"]),
                fs,
                settings.as_ref(),
                &mut [],
            ))
        }
        "doQuoteComplete" => {
            v(ls.do_quote_complete(&document, position(), &html, settings.as_ref()))
        }
        "doTagComplete" => v(ls.do_tag_complete(&document, position(), &html)),
        "doHover" => {
            let options: Option<HoverSettings> =
                serde_json::from_value(call["options"].clone()).unwrap();
            v(ls.do_hover(&document, position(), &html, options))
        }
        "getFoldingRanges" => {
            let limit = call["context"]["rangeLimit"].as_u64().map(|n| n as usize);
            v(ls.get_folding_ranges(&document, limit))
        }
        "getSelectionRanges" => {
            let positions: Vec<Position> =
                serde_json::from_value(call["positions"].clone()).unwrap();
            v(ls.get_selection_ranges(&document, &positions))
        }
        "findDocumentLinks" => v(links::find_document_links(
            &document,
            &RecordedContext(&call["resolveReference"]),
        )),
        "findDocumentHighlights" => v(navigation::find_document_highlights(
            &document,
            position(),
            &html,
        )),
        "findDocumentSymbols" => v(navigation::find_document_symbols(&document, &html)),
        "findDocumentSymbols2" => v(navigation::find_document_symbols2(&document, &html)),
        "doRename" => v(navigation::do_rename(
            &document,
            position(),
            call["newName"].as_str().unwrap(),
            &html,
        )),
        "findMatchingTagPosition" => v(navigation::find_matching_tag_position(
            &document,
            position(),
            &html,
        )),
        "findLinkedEditingRanges" => v(navigation::find_linked_editing_ranges(
            &document,
            position(),
            &html,
        )),
        "format" => {
            let range: Option<lsp_types::Range> =
                serde_json::from_value(call["range"].clone()).unwrap();
            let options = serde_json::from_value(call["options"].clone()).unwrap();
            v(formatter::format(&document, range, &options))
        }
        _ => return None,
    })
}

#[test]
fn features() {
    let f = fixture();
    let mut failures = 0;
    let mut skipped = std::collections::BTreeMap::<String, usize>::new();
    for call in f["calls"].as_array().unwrap() {
        let Some(actual) = run(call) else {
            *skipped
                .entry(call["fn"].as_str().unwrap().to_owned())
                .or_default() += 1;
            continue;
        };
        if actual != call["result"] {
            failures += 1;
            if failures <= 10 {
                let text = call["document"]["text"].as_str().unwrap();
                let s = |v: &Value| {
                    let s = v.to_string();
                    s.chars().take(1500).collect::<String>()
                };
                eprintln!(
                    "{} {:?} {}\n  expected {}\n  actual   {}",
                    call["fn"],
                    text,
                    call["position"],
                    s(&call["result"]),
                    s(&actual)
                );
            }
        }
    }
    eprintln!("skipped {skipped:?}");
    assert_eq!(failures, 0);
}
