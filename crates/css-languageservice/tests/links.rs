//! Compares document links, module resolution and path completion with
//! upstream. The file system and `resolveReference` calls are answered from
//! the recording; a call that upstream did not make fails the test. Fixture:
//! `scripts/css/trace-links.mjs`.

use css_languageservice::parser::{Dialect, parse_stylesheet};
use css_languageservice::services::links::{
    AliasSettings, DocumentContext, FileStat, FileSystemProvider, FileType, LinkResolver,
    get_module_name_from_path,
};
use css_languageservice::services::path_completion::PathCompletionParticipant;
use serde_json::{Value, json};
use text_document::TextDocument;

struct Recorded<'a>(&'a [Value]);

impl Recorded<'_> {
    fn find(&self, f: impl Fn(&Value) -> bool, what: String) -> &Value {
        self.0
            .iter()
            .find(|c| f(c))
            .unwrap_or_else(|| panic!("upstream made no call {what}"))
    }

    fn fs_call(&self, name: &str, uri: &str) -> Option<&Value> {
        let c = self.find(
            |c| c["fn"] == name && c["uri"] == uri,
            format!("{name} {uri}"),
        );
        (c["thrown"] != true).then(|| &c["result"])
    }
}

fn file_type(v: &Value) -> FileType {
    match v.as_u64().unwrap() {
        1 => FileType::File,
        2 => FileType::Directory,
        64 => FileType::SymbolicLink,
        _ => FileType::Unknown,
    }
}

impl FileSystemProvider for Recorded<'_> {
    fn stat(&self, uri: &str) -> Option<FileStat> {
        let r = self.fs_call("stat", uri)?;
        Some(FileStat {
            file_type: file_type(&r["type"]),
            size: r["size"].as_i64().unwrap(),
        })
    }

    fn read_directory(&self, uri: &str) -> Option<Vec<(String, FileType)>> {
        let r = self.fs_call("readDirectory", uri)?;
        Some(
            r.as_array()
                .unwrap()
                .iter()
                .map(|e| (e[0].as_str().unwrap().to_owned(), file_type(&e[1])))
                .collect(),
        )
    }

    fn get_content(&self, uri: &str) -> Option<String> {
        Some(
            self.fs_call("getContent", uri)?
                .as_str()
                .unwrap()
                .to_owned(),
        )
    }
}

impl DocumentContext for Recorded<'_> {
    fn resolve_reference(&self, reference: &str, base_url: &str) -> Option<String> {
        let c = self.find(
            |c| c["fn"] == "resolveReference" && c["ref"] == reference && c["base"] == base_url,
            format!("resolveReference {reference} {base_url}"),
        );
        c["result"].as_str().map(str::to_owned)
    }
}

fn document(d: &Value) -> TextDocument {
    TextDocument::new(
        d["uri"].as_str().unwrap().parse().unwrap(),
        d["languageId"].as_str().unwrap(),
        d["version"].as_i64().unwrap() as i32,
        d["text"].as_str().unwrap().to_owned(),
    )
}

fn dialect(language_id: &str) -> Dialect {
    match language_id {
        "scss" => Dialect::Scss,
        "less" => Dialect::Less,
        _ => Dialect::Css,
    }
}

/// The actual result of a recorded call.
fn run(c: &Value) -> Value {
    let recorded = Recorded(c["calls"].as_array().unwrap());
    let doc = document(&c["document"]);
    match c["fn"].as_str().unwrap() {
        "computeCompletions" => {
            let participant = PathCompletionParticipant {
                literal_completions: serde_json::from_value(c["literal"].clone()).unwrap(),
                import_completions: serde_json::from_value(c["import"].clone()).unwrap(),
            };
            json!(participant.compute_completions(&doc, &recorded, &recorded))
        }
        f => {
            let aliases: Option<AliasSettings> =
                serde_json::from_value(c["aliases"].clone()).unwrap();
            let resolver = LinkResolver {
                dialect: if c["scss"] == true {
                    Dialect::Scss
                } else {
                    Dialect::Css
                },
                file_system: (c["hasFs"] == true).then_some(&recorded as &dyn FileSystemProvider),
                aliases: aliases.as_ref(),
            };
            let sheet = parse_stylesheet(dialect(doc.language_id()), doc.text());
            if f == "findDocumentLinks" {
                json!(resolver.find_document_links(&doc, &sheet, &recorded))
            } else {
                json!(resolver.find_document_links2(&doc, &sheet, &recorded))
            }
        }
    }
}

/// `None` for the calls that the branch answers differently (`docs/experimental.md`):
/// where upstream rejects, and path completion where upstream computes the range on one
/// line with two quotes. The recorded contexts can have a value that is not the text
/// at their range; upstream reads the value, the branch the text.
fn expected(c: &Value) -> Option<&Value> {
    let comparable = |context: &Value| {
        let value = context["uriValue"]
            .as_str()
            .or(context["pathValue"].as_str())
            .unwrap();
        let range: lsp_types::Range = serde_json::from_value(context["range"].clone()).unwrap();
        let position: lsp_types::Position =
            serde_json::from_value(context["position"].clone()).unwrap();
        let doc = document(&c["document"]);
        let text = &doc.text()[doc.offset_at(range.start)..doc.offset_at(range.end)];
        // The branch completes only for a cursor between the quotes.
        let (closed, quote) = match value.chars().next() {
            Some(q @ ('\'' | '"')) => (value.len() > 1 && value.ends_with(q), 1),
            _ => (true, 0),
        };
        range.start.line == range.end.line
            && position.line == range.start.line
            && (range.start.character + quote..=range.end.character - quote)
                .contains(&position.character)
            && text == value
            && closed
    };
    let contexts = || {
        c["literal"]
            .as_array()
            .into_iter()
            .chain(c["import"].as_array())
            .flatten()
    };
    (!c["rejected"].is_string() && contexts().all(comparable)).then_some(&c["result"])
}

#[test]
fn upstream_calls() {
    let calls: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/links.json")).expect("fixture");
    let mut skipped = 0;
    for c in &calls {
        match expected(c) {
            Some(expected) => assert_eq!(&run(c), expected, "{c}"),
            None => skipped += 1,
        }
    }
    eprintln!("skipped {skipped} calls");
}

/// `CSS_LINKS_SWEEP=<absolute path of .scratch/data/css-links-sweep.json>`.
#[test]
fn upstream_sweep() {
    let Ok(path) = std::env::var("CSS_LINKS_SWEEP") else {
        return;
    };
    let calls: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut failures = 0;
    let mut skipped = 0;
    for c in &calls {
        let Some(expected) = expected(c) else {
            skipped += 1;
            continue;
        };
        let actual = std::panic::catch_unwind(|| run(c)).unwrap_or(Value::Null);
        if actual != *expected {
            failures += 1;
            if failures <= 10 {
                eprintln!(
                    "{} {} {:?}\nexpected: {}\nactual:   {actual}\ncalls: {}\n",
                    c["fn"], c["document"]["uri"], c["document"]["text"], expected, c["calls"]
                );
            }
        }
    }
    eprintln!("skipped {skipped} calls");
    assert_eq!(failures, 0, "{failures} of {} calls differ", calls.len());
}

#[test]
fn module_name_from_path() {
    for (path, expected) in [
        ("foo", "foo"),
        ("foo/bar", "foo"),
        ("@foo/bar", "@foo/bar"),
        ("@foo/bar/baz", "@foo/bar"),
        ("@foo", "@foo"),
        ("/x", ""),
    ] {
        assert_eq!(get_module_name_from_path(path), expected);
    }
}

/// Answers each call: every path exists, each folder has some entries.
struct Everything;

impl FileSystemProvider for Everything {
    fn stat(&self, _uri: &str) -> Option<FileStat> {
        Some(FileStat {
            file_type: FileType::File,
            size: 1,
        })
    }

    fn read_directory(&self, _uri: &str) -> Option<Vec<(String, FileType)>> {
        Some(vec![
            ("_a b.scss".into(), FileType::File),
            ("dir".into(), FileType::Directory),
            (".x".into(), FileType::File),
        ])
    }

    fn get_content(&self, _uri: &str) -> Option<String> {
        Some(r#"{"exports": {"./*": {"sass": "./*.scss"}, ".": {"sass": "./i.scss"}}}"#.into())
    }
}

impl DocumentContext for Everything {
    fn resolve_reference(&self, reference: &str, base_url: &str) -> Option<String> {
        Some(format!("{base_url}/../{reference}"))
    }
}

/// No panics: links (each dialect, with aliases) on the parser sweep inputs,
/// and `do_complete2` at the offsets near a link.
/// `CSS_SWEEP_INPUTS=<absolute path of .scratch/data/css-sweep-inputs.json>`.
#[test]
fn robustness() {
    use css_languageservice::language_facts::CssDataManager;
    use css_languageservice::services::completion::Completion;
    let Ok(path) = std::env::var("CSS_SWEEP_INPUTS") else {
        return;
    };
    let inputs: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).expect("inputs");
    let data = CssDataManager::new(true, &[]);
    let aliases: AliasSettings = [("@a/".into(), "x/".into()), ("/".into(), "".into())].into();
    for c in inputs.iter().filter(|c| c["fn"] == "_parseStylesheet") {
        let input = c["input"].as_str().unwrap();
        let d = dialect(c["dialect"].as_str().unwrap_or("css"));
        let doc = TextDocument::new(
            "file:///c:/ws/a/doc.scss".parse().unwrap(),
            if d == Dialect::Scss { "scss" } else { "css" },
            1,
            input.to_owned(),
        );
        let sheet = parse_stylesheet(d, input);
        for dialect in [Dialect::Css, Dialect::Scss] {
            let resolver = LinkResolver {
                dialect,
                file_system: Some(&Everything),
                aliases: Some(&aliases),
            };
            resolver.find_document_links(&doc, &sheet, &Everything);
            resolver.find_document_links2(&doc, &sheet, &Everything);
        }
        let completion = Completion {
            dialect: d,
            data: &data,
            markdown: true,
        };
        // Path completion runs only after `url(` or an import.
        let near_link = |b: usize| {
            let window = &input.as_bytes()[b.saturating_sub(40)..b];
            [&b"url("[..], b"@import", b"@use", b"@forward"]
                .iter()
                .any(|k| window.windows(k.len()).any(|w| w == *k))
        };
        for byte in (0..=input.len()).filter(|&b| input.is_char_boundary(b) && near_link(b)) {
            completion.do_complete2(
                &doc,
                doc.position_at(byte),
                &sheet,
                &Everything,
                Some(&Everything),
                None,
                &mut [],
            );
        }
    }
}

/// A file system with the files of a list, in `file:///c:/ws/`.
struct Files(&'static [(&'static str, &'static str)]);

const ROOT: &str = "file:///c:/ws/";

impl FileSystemProvider for Files {
    fn stat(&self, uri: &str) -> Option<FileStat> {
        let path = uri.strip_prefix(ROOT)?;
        let file_type = if let Some((_, content)) = self.0.iter().find(|(p, _)| *p == path) {
            return Some(FileStat {
                file_type: FileType::File,
                size: content.len() as i64,
            });
        } else if self
            .0
            .iter()
            .any(|(p, _)| p.starts_with(&format!("{path}/")))
        {
            FileType::Directory
        } else {
            FileType::Unknown
        };
        Some(FileStat {
            file_type,
            size: -1,
        })
    }

    fn read_directory(&self, uri: &str) -> Option<Vec<(String, FileType)>> {
        let folder = uri.strip_prefix(ROOT)?;
        let mut entries: Vec<(String, FileType)> = Vec::new();
        for (path, _) in self.0 {
            let Some(rest) = path.strip_prefix(folder) else {
                continue;
            };
            let entry = match rest.split_once('/') {
                Some((name, _)) => (name.to_owned(), FileType::Directory),
                None => (rest.to_owned(), FileType::File),
            };
            if !entries.contains(&entry) {
                entries.push(entry);
            }
        }
        Some(entries)
    }

    fn get_content(&self, uri: &str) -> Option<String> {
        let path = uri.strip_prefix(ROOT)?;
        self.0
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, c)| (*c).to_owned())
    }
}

impl DocumentContext for Files {
    fn resolve_reference(&self, reference: &str, base_url: &str) -> Option<String> {
        if reference.contains("://") {
            return Some(reference.to_owned());
        }
        if let Some(path) = reference.strip_prefix('/') {
            return Some(format!("{ROOT}{path}"));
        }
        let folder = &base_url[..base_url.rfind('/')? + 1];
        Some(format!("{folder}{}", reference.trim_start_matches("./")))
    }
}

fn scss(text: &str) -> TextDocument {
    TextDocument::new(
        format!("{ROOT}doc.scss").parse().unwrap(),
        "scss",
        1,
        text.to_owned(),
    )
}

/// `docs/experimental.md`: a link whose resolution throws upstream is left out, and a
/// `package.json` value that is not a string counts as absent. Upstream gives no links.
#[test]
fn links_where_upstream_rejects() {
    let files = Files(&[
        ("node_modules/root-num/package.json", r#"{"sass":5}"#),
        ("node_modules/null-pkg/package.json", "null"),
        (
            "node_modules/obj-entry/package.json",
            r#"{"exports":{".":{"sass":{"x":1}}}}"#,
        ),
        ("node_modules/good/package.json", r#"{"sass":"i.scss"}"#),
        ("node_modules/good/i.scss", ""),
    ]);
    let doc = scss(
        "@import 'pkg:root-num';\n@import 'pkg:null-pkg';\n@import 'pkg:obj-entry';\n\
         @import 'gantry-media://g.png';\n@import 'pkg:good';",
    );
    let resolver = LinkResolver {
        dialect: Dialect::Scss,
        file_system: Some(&files),
        aliases: None,
    };
    let sheet = parse_stylesheet(Dialect::Scss, doc.text());
    let links = resolver.find_document_links2(&doc, &sheet, &files);
    let targets: Vec<&str> = links.iter().map(|l| l.target.as_str()).collect();
    assert_eq!(targets, ["file:///c:/ws/node_modules/good/i.scss"]);
}

/// `docs/experimental.md`: the path completion range comes from offsets. Upstream throws
/// for a value on two lines with a short last line.
#[test]
fn path_completion_offsets() {
    use css_languageservice::services::completion::{
        ImportPathCompletionContext, UriLiteralCompletionContext,
    };
    use lsp_types::{CompletionTextEdit, Position, Range};
    let files = Files(&[("dir/a.css", ""), ("b.css", "")]);
    let complete = |text: &str, value: &str, start: Position, end: Position, cursor: Position| {
        let doc = TextDocument::new(
            format!("{ROOT}doc.css").parse().unwrap(),
            "css",
            1,
            text.to_owned(),
        );
        let participant = PathCompletionParticipant {
            literal_completions: vec![UriLiteralCompletionContext {
                uri_value: value.to_owned(),
                position: cursor,
                range: Range::new(start, end),
            }],
            import_completions: Vec::<ImportPathCompletionContext>::new(),
        };
        let list = participant.compute_completions(&doc, &files, &files);
        list.items
            .into_iter()
            .map(|i| match i.text_edit {
                Some(CompletionTextEdit::Edit(e)) => (i.label, e.range),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>()
    };
    let p = Position::new;
    // `url("dir/\` + newline + `")`: the value is on two lines.
    let items = complete(
        "a { b: url(\"dir/\\\n\") }",
        "\"dir/\\\n\"",
        p(0, 11),
        p(1, 1),
        p(0, 16),
    );
    assert_eq!(items, [("a.css".to_owned(), Range::new(p(0, 16), p(1, 0)))]);
    // An unclosed quote keeps the last character of the value.
    let items = complete("@import 'dir/a", "'dir/a", p(0, 8), p(0, 14), p(0, 14));
    assert_eq!(
        items,
        [("a.css".to_owned(), Range::new(p(0, 13), p(0, 14)))]
    );
    // No paths for a cursor out of the value: the context range is the `u` of
    // `url`, empty after the `)`, or the cursor is at a quote.
    let text = "@import url(\"dir/a\");";
    let value = "\"dir/a\"";
    assert_eq!(complete(text, value, p(0, 8), p(0, 9), p(0, 9)), []);
    assert_eq!(complete(text, value, p(0, 20), p(0, 20), p(0, 20)), []);
    assert_eq!(complete(text, value, p(0, 12), p(0, 19), p(0, 12)), []);
    assert_eq!(complete(text, value, p(0, 12), p(0, 19), p(0, 19)), []);
    assert_eq!(complete(text, value, p(0, 12), p(0, 19), p(0, 18)).len(), 1);
}
