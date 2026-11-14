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
            match participant.compute_completions(&doc, &recorded, &recorded) {
                Some(list) => json!(list),
                None => json!([]),
            }
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

fn expected(c: &Value) -> Value {
    if c["rejected"].is_string() {
        json!([])
    } else {
        c["result"].clone()
    }
}

#[test]
fn upstream_calls() {
    let calls: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/links.json")).expect("fixture");
    for c in &calls {
        assert_eq!(run(c), expected(c), "{c}");
    }
}

/// `CSS_LINKS_SWEEP=<absolute path of .scratch/data/css-links-sweep.json>`.
#[test]
fn upstream_sweep() {
    let Ok(path) = std::env::var("CSS_LINKS_SWEEP") else {
        return;
    };
    let calls: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut failures = 0;
    for c in &calls {
        let actual = std::panic::catch_unwind(|| run(c)).unwrap_or(Value::Null);
        if actual != expected(c) {
            failures += 1;
            if failures <= 10 {
                eprintln!(
                    "{} {} {:?}\nexpected: {}\nactual:   {actual}\ncalls: {}\n",
                    c["fn"],
                    c["document"]["uri"],
                    c["document"]["text"],
                    expected(c),
                    c["calls"]
                );
            }
        }
    }
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
