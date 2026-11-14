//! Ports of the VS Code HTML server tests `folding.test.ts`,
//! `formatting.test.ts`, `selectionRanges.test.ts` and `completions.test.ts`.
//! The JavaScript cases, and `javascriptMode.test.ts`, `hover.test.ts`,
//! `rename.test.ts` and `semanticTokens.test.ts`, are in `mod javascript`.

use std::path::Path;

use css_language_server::fs::RequestService;
use html_languageservice::LanguageService;
use lsp_types::{FoldingRangeKind, Position, Range, TextEdit};
use serde_json::{Value, json};
use text_document::TextDocument;
use vscode_uri::Uri;

use crate::context::WorkspaceContext;
use crate::modes::{Document, LanguageModes};

fn modes() -> LanguageModes {
    LanguageModes::new(
        true,
        true,
        LanguageService::new(true, Vec::new(), None),
        true,
        true,
        RequestService {
            handled_schemas: vec!["file".into()],
        },
    )
}

fn document(uri: &str, text: &str) -> Document {
    Document::new(TextDocument::new(
        uri.parse().unwrap(),
        "html",
        0,
        text.to_owned(),
    ))
}

/// The offset of the first `|` (or `None`) and the text without it.
fn take_cursor(value: &str) -> (Option<usize>, String) {
    match value.find('|') {
        Some(offset) => (
            Some(offset),
            format!("{}{}", &value[..offset], &value[offset + 1..]),
        ),
        None => (None, value.to_owned()),
    }
}

fn assert_folding(lines: &[&str], expected: &[(u32, u32, Option<&str>)], limit: Option<usize>) {
    let document = document("test://foo/bar.html", &lines.join("\n"));
    let mut actual: Vec<(u32, u32, Option<&str>)> = modes()
        .folding_ranges(&document, limit)
        .unwrap()
        .iter()
        .map(|r| {
            let kind = r.kind.as_ref().map(|k| match k {
                FoldingRangeKind::Comment => "comment",
                FoldingRangeKind::Imports => "imports",
                FoldingRangeKind::Region => "region",
            });
            (r.start_line, r.end_line, kind)
        })
        .collect();
    actual.sort_by_key(|r| r.0);
    assert_eq!(actual, expected, "limit {limit:?}");
}

#[test]
fn folding_embedded_css() {
    let input = [
        "<html>",
        "<head>",
        "<style>",
        "  foo {",
        "   display: block;",
        "   color: black;",
        "  }",
        "</style>",
        "</head>",
        "</html>",
    ];
    assert_folding(
        &input,
        &[(0, 8, None), (1, 7, None), (2, 6, None), (3, 5, None)],
        None,
    );
}

#[test]
fn folding_embedded_css_multiple_areas() {
    let input = [
        "<html>",
        "<head style=\"color:red\">",
        "<style>",
        "  /*",
        "    foo: true,",
        "    bar: {}",
        "  */",
        "</style>",
        "<style>",
        "  @keyframes mymove {",
        "    from {top: 0px;}",
        "  }",
        "</style>",
        "</head>",
        "</html>",
    ];
    assert_folding(
        &input,
        &[
            (0, 13, None),
            (1, 12, None),
            (2, 6, None),
            (3, 6, Some("comment")),
            (8, 11, None),
            (9, 10, None),
        ],
        None,
    );
}

#[test]
fn folding_embedded_css_regions() {
    let input = [
        "<html>",
        "<head>",
        "<style>",
        "  /* #region Lalala */",
        "   /*  #region*/",
        "   x = 9;",
        "  /*  #endregion*/",
        "  /* #endregion Lalala*/",
        "</style>",
        "</head>",
        "</html>",
    ];
    assert_folding(
        &input,
        &[
            (0, 9, None),
            (1, 8, None),
            (2, 7, None),
            (3, 7, Some("region")),
            (4, 6, Some("region")),
        ],
        None,
    );
}

#[test]
fn folding_limit() {
    let input = [
        "<div>",
        " <span>",
        "  <b>",
        "  ",
        "  </b>,",
        "  <b>",
        "   <pre>",
        "  ",
        "   </pre>,",
        "   <pre>",
        "  ",
        "   </pre>,",
        "  </b>,",
        "  <b>",
        "  ",
        "  </b>,",
        "  <b>",
        "  ",
        "  </b>",
        " </span>",
        "</div>",
    ];
    let all = [
        (0, 19, None),
        (1, 18, None),
        (2, 3, None),
        (5, 11, None),
        (6, 7, None),
        (9, 10, None),
        (13, 14, None),
        (16, 17, None),
    ];
    let without = |skip: &[u32]| -> Vec<(u32, u32, Option<&str>)> {
        all.iter()
            .filter(|r| !skip.contains(&r.0))
            .copied()
            .collect()
    };
    assert_folding(&input, &all, None);
    assert_folding(&input, &all, Some(8));
    assert_folding(&input, &without(&[9]), Some(7));
    assert_folding(&input, &without(&[6, 9]), Some(6));
    assert_folding(&input, &without(&[6, 9, 16]), Some(5));
    assert_folding(&input, &without(&[6, 9, 13, 16]), Some(4));
    assert_folding(&input, &without(&[5, 6, 9, 13, 16]), Some(3));
    assert_folding(&input, &without(&[2, 5, 6, 9, 13, 16]), Some(2));
    assert_folding(&input, &without(&[1, 2, 5, 6, 9, 13, 16]), Some(1));
}

fn assert_selection(content: &str, expected: &[(usize, &str)]) {
    let (offset, content) = take_cursor(content);
    let document = document("test://foo.html", &content);
    let text = &document.text;
    let position = text.position_at(offset.unwrap());
    let ranges = modes().selection_ranges(&document, &[position]).unwrap();
    assert_eq!(ranges.len(), 1);
    let mut actual = Vec::new();
    let mut current = Some(&ranges[0]);
    while let Some(range) = current {
        let (start, end) = (
            text.offset_at(range.range.start),
            text.offset_at(range.range.end),
        );
        actual.push((start, &content[start..end]));
        current = range.parent.as_deref();
    }
    assert_eq!(actual, expected, "{content}");
}

#[test]
fn selection_html() {
    assert_selection(
        "<div><p>|a</p></div>",
        &[(8, "a"), (5, "<p>a</p>"), (0, "<div><p>a</p></div>")],
    );
}

#[test]
fn selection_embedded_css() {
    assert_selection(
        "<html><head><style>foo { display: |none; } </style></head></html>",
        &[
            (34, "none"),
            (25, "display: none"),
            (24, " display: none; "),
            (23, "{ display: none; }"),
            (19, "foo { display: none; }"),
            (19, "foo { display: none; } "),
            (12, "<style>foo { display: none; } </style>"),
            (6, "<head><style>foo { display: none; } </style></head>"),
            (
                0,
                "<html><head><style>foo { display: none; } </style></head></html>",
            ),
        ],
    );
}

#[test]
fn selection_embedded_style() {
    assert_selection(
        "<div style=\"color: |red\"></div>",
        &[
            (19, "red"),
            (12, "color: red"),
            (11, "\"color: red\""),
            (5, "style=\"color: red\""),
            (1, "div style=\"color: red\""),
            (0, "<div style=\"color: red\"></div>"),
        ],
    );
}

/// `assertFormat`: `|` marks the start and the end of the range.
fn assert_format(value: &str, expected: &str, settings: Option<&Value>, options: Option<Value>) {
    let (start, value) = take_cursor(value);
    let (start, end, value) = match start {
        Some(start) => {
            let (end, value) = take_cursor(&value);
            (start, end.unwrap(), value)
        }
        None => (0, value.len(), value),
    };
    let document = document("test://test/test.html", &value);
    let text = &document.text;
    let range = Range::new(text.position_at(start), text.position_at(end));
    let options = options.unwrap_or_else(|| json!({ "tabSize": 2, "insertSpaces": true }));
    let edits: Vec<TextEdit> = modes()
        .format(&document, range, &options, settings)
        .unwrap();
    assert_eq!(text.apply_edits(&edits), expected, "{value}");
}

#[test]
fn format_html_only() {
    let expected = "<html>\n\n<body>\n  <p>Hello</p>\n</body>\n\n</html>";
    assert_format(
        "<html><body><p>Hello</p></body></html>",
        expected,
        None,
        None,
    );
    assert_format(
        "|<html><body><p>Hello</p></body></html>|",
        expected,
        None,
        None,
    );
    assert_format(
        "<html>|<body><p>Hello</p></body>|</html>",
        "<html><body>\n  <p>Hello</p>\n</body></html>",
        None,
        None,
    );
}

#[test]
fn format_empty_scripts() {
    assert_format(
        "<html><head><script></script></head></html>",
        "<html>\n\n<head>\n  <script></script>\n</head>\n\n</html>",
        None,
        None,
    );
    // bug 36574
    assert_format(
        "<script src=\"/js/main.js\"> </script>",
        "<script src=\"/js/main.js\"> </script>",
        None,
        None,
    );
}

#[test]
fn format_styles() {
    assert_format(
        "<html><head>\n<style>\n.foo{display:none;}\n</style></head></html>",
        "<html>\n\n<head>\n  <style>\n    .foo {\n      display: none;\n    }\n  </style>\n</head>\n\n</html>",
        None,
        None,
    );
}

#[test]
fn format_end_with_newline() {
    let options =
        || Some(json!({ "tabSize": 2, "insertSpaces": true, "insertFinalNewline": true }));
    let settings = json!({});
    assert_format(
        "<html><body><p>Hello</p></body></html>",
        "<html>\n\n<body>\n  <p>Hello</p>\n</body>\n\n</html>\n",
        Some(&settings),
        options(),
    );
    assert_format(
        "<html>|<body><p>Hello</p></body>|</html>",
        "<html><body>\n  <p>Hello</p>\n</body></html>",
        Some(&settings),
        options(),
    );
    assert_format(
        "<html>|<body><p>Hello</p></body></html>|",
        "<html><body>\n  <p>Hello</p>\n</body>\n\n</html>\n",
        Some(&settings),
        options(),
    );
}

#[test]
fn format_content_unformatted() {
    // #58435
    let settings = json!({ "html": { "format": { "contentUnformatted": "textarea" } } });
    let content = [
        "<html>",
        "",
        "<body>",
        "  <textarea name= \"\" id =\"\" cols=\"30\" rows=\"10\">",
        "  </textarea>",
        "</body>",
        "",
        "</html>",
    ];
    let expected = [
        "<html>",
        "",
        "<body>",
        "  <textarea name=\"\" id=\"\" cols=\"30\" rows=\"10\">",
        "  </textarea>",
        "</body>",
        "",
        "</html>",
    ];
    assert_format(
        &content.join("\n"),
        &expected.join("\n"),
        Some(&settings),
        None,
    );
}

fn fixture(path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::read_to_string(root.join(path))
        .unwrap()
        .replace("\r\n", "\n")
}

/// The expected files are the upstream files. Without the JavaScript mode,
/// the script content is not formatted (`expected-nojs`, from the Node
/// reference without the JavaScript mode).
#[test]
fn format_fixtures() {
    const EXPECTED: &str = if cfg!(feature = "javascript") {
        "expected"
    } else {
        "expected-nojs"
    };
    for (input, expected, options) in [
        (
            "19813.html",
            "19813.html",
            json!({ "tabSize": 2, "insertSpaces": true }),
        ),
        (
            "19813.html",
            "19813-4spaces.html",
            json!({ "tabSize": 4, "insertSpaces": true }),
        ),
        (
            "19813.html",
            "19813-tab.html",
            json!({ "tabSize": 1, "insertSpaces": false }),
        ),
        (
            "21634.html",
            "21634.html",
            json!({ "tabSize": 2, "insertSpaces": true }),
        ),
    ] {
        assert_format(
            &fixture(&format!("inputs/{input}")),
            &fixture(&format!("{EXPECTED}/{expected}")),
            None,
            Some(options),
        );
    }
}

/// `(label, kind, result text, command)`.
type Item<'a> = (&'a str, Option<u64>, &'a str, bool);

const FILE: Option<u64> = Some(17);
const FOLDER: Option<u64> = Some(19);

/// `testCompletionFor`.
fn completions(value: &str, uri: &str, folders: Option<&[String]>) -> (String, Vec<Value>) {
    let (offset, value) = take_cursor(value);
    let default_folder = [uri[..uri.rfind('/').unwrap()].to_owned()];
    let context = WorkspaceContext {
        document_uri: uri,
        workspace_folders: folders.unwrap_or(&default_folder),
    };
    let modes = modes();
    let document = document(uri, &value);
    // `positionAt(-1)` without a cursor.
    let position = offset.map_or(Position::new(0, 0), |o| document.text.position_at(o));
    let mode = modes.mode_at_position(&document, position).unwrap();
    let settings = json!({});
    let list = modes
        .complete(mode, &document, position, &context, Some(&settings))
        .unwrap();
    let list: Value = serde_json::from_str(list.get()).unwrap();
    (value, list["items"].as_array().unwrap().clone())
}

fn assert_completions(value: &str, uri: &str, folders: Option<&[String]>, expected: &[Item]) {
    let (value, items) = completions(value, uri, folders);
    let document = TextDocument::new(uri.parse().unwrap(), "html", 0, value.clone());
    for &(label, kind, result_text, command) in expected {
        let matches: Vec<&Value> = items.iter().filter(|i| i["label"] == label).collect();
        assert_eq!(matches.len(), 1, "{label} in {value}");
        let item = matches[0];
        if kind.is_some() {
            assert_eq!(item["kind"].as_u64(), kind, "{label}");
        }
        let edit = &item["textEdit"];
        let range = edit.get("range").or(edit.get("replace")).unwrap();
        let edit = TextEdit::new(
            serde_json::from_value(range.clone()).unwrap(),
            edit["newText"].as_str().unwrap().to_owned(),
        );
        assert_eq!(document.apply_edits(&[edit]), result_text, "{label}");
        if command {
            assert_eq!(
                item["command"],
                json!({ "title": "Suggest", "command": "editor.action.triggerSuggest" })
            );
        }
    }
}

fn fixture_uri(path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pathCompletionFixtures");
    let path = if path.is_empty() {
        root
    } else {
        root.join(path)
    };
    Uri::file(path.to_str().unwrap()).to_string_encoded()
}

#[test]
fn path_completion() {
    let index = fixture_uri("index.html");
    let about = fixture_uri("about/about.html");
    let workspace = [fixture_uri("")];
    let w = Some(&workspace[..]);
    let root_items = |prefix: &str, suffix: &str| -> [(String, &'static str); 3] {
        ["about/", "index.html", "src/"].map(|l| (format!("{prefix}{l}{suffix}"), l))
    };
    let about_items = |prefix: &str, suffix: &str| -> [(String, &'static str); 3] {
        ["about.css", "about.html", "media/"].map(|l| (format!("{prefix}{l}{suffix}"), l))
    };
    let src_items = |prefix: &str, suffix: &str| -> [(String, &'static str); 2] {
        ["feature.js", "test.js"].map(|l| (format!("{prefix}{l}{suffix}"), l))
    };
    let check = |value: &str, uri: &str, folders: Option<&[String]>, items: &[(String, &str)]| {
        let expected: Vec<Item> = items
            .iter()
            .map(|(text, label)| (*label, None, text.as_str(), false))
            .collect();
        assert_completions(value, uri, folders, &expected);
    };

    // Basics: label, kind, result and command.
    for q in ["\"", "'"] {
        let r = |l: &str| format!("<script src={q}./{l}{q}>");
        let (about_dir, index_file, src_dir) = (r("about/"), r("index.html"), r("src/"));
        assert_completions(
            &r("|"),
            &index,
            None,
            &[
                ("about/", FOLDER, &about_dir, true),
                ("index.html", FILE, &index_file, false),
                ("src/", FOLDER, &src_dir, true),
            ],
        );
    }

    // No completion for remote paths: upstream asserts no items.
    for value in [
        "<script src=\"http:\">",
        "<script src=\"http:/|\">",
        "<script src=\"http://|\">",
        "<script src=\"https:|\">",
        "<script src=\"https:/|\">",
        "<script src=\"https://|\">",
        "<script src=\"//|\">",
    ] {
        let (_, items) = completions(value, &fixture_uri("x.html"), None);
        assert!(
            items
                .iter()
                .all(|i| i["kind"].as_u64() != FILE && i["kind"].as_u64() != FOLDER),
            "{value}"
        );
    }

    // Relative path.
    check(
        "<script src=\"../|\">",
        &about,
        None,
        &root_items("<script src=\"../", "\">"),
    );
    check(
        "<script src=\"../src/|\">",
        &about,
        None,
        &src_items("<script src=\"../src/", "\">"),
    );

    // Absolute path.
    check(
        "<script src=\"/|\">",
        &index,
        None,
        &root_items("<script src=\"/", "\">"),
    );
    check(
        "<script src=\"/src/|\">",
        &about,
        w,
        &src_items("<script src=\"/src/", "\">"),
    );

    // Empty path value.
    check(
        "<script src=\"|\">",
        &index,
        None,
        &root_items("<script src=\"", "\">"),
    );
    check(
        "<script src=\"|\">",
        &about,
        None,
        &about_items("<script src=\"", "\">"),
    );

    // Incomplete path.
    check(
        "<script src=\"/src/f|\">",
        &about,
        w,
        &src_items("<script src=\"/src/", "\">"),
    );
    check(
        "<script src=\"../src/f|\">",
        &about,
        w,
        &src_items("<script src=\"../src/", "\">"),
    );

    // No leading dot or slash.
    check(
        "<script src=\"s|\">",
        &index,
        w,
        &root_items("<script src=\"", "\">"),
    );
    check(
        "<script src=\"src/|\">",
        &index,
        w,
        &src_items("<script src=\"src/", "\">"),
    );
    check(
        "<script src=\"src/f|\">",
        &index,
        w,
        &src_items("<script src=\"src/", "\">"),
    );
    check(
        "<script src=\"s|\">",
        &about,
        w,
        &about_items("<script src=\"", "\">"),
    );
    let icon = [("<script src=\"media/icon.pic\">".to_owned(), "icon.pic")];
    check("<script src=\"media/|\">", &about, w, &icon);
    check("<script src=\"media/f|\">", &about, w, &icon);

    // Completion in the middle of a path.
    check(
        "<script src=\"src/f|eature.js\">",
        &index,
        w,
        &src_items("<script src=\"src/", "\">"),
    );
    check(
        "<script src=\"s|rc/feature.js\">",
        &index,
        w,
        &root_items("<script src=\"", "\">"),
    );
    check("<script src=\"media/f|eature.js\">", &about, w, &icon);
    check(
        "<script src=\"m|edia/feature.js\">",
        &about,
        w,
        &about_items("<script src=\"", "\">"),
    );

    // In the middle of a path, with white space.
    check(
        "<script src=\"./| about/about.html>",
        &index,
        w,
        &root_items("<script src=\"./", " about/about.html>"),
    );
    check(
        "<script src=\"./a|bout /about.html>",
        &index,
        w,
        &root_items("<script src=\"./", " /about.html>"),
    );

    // Files and folders that start with a dot are ignored.
    let (_, items) = completions("<script src=\"./|\"", &index, w);
    assert_eq!(items.len(), 3);
}

/// The JavaScript cases of `javascriptMode.test.ts`, `hover.test.ts`,
/// `rename.test.ts`, `completions.test.ts`, `folding.test.ts`,
/// `formatting.test.ts`, `selectionRanges.test.ts` and
/// `semanticTokens.test.ts`.
#[cfg(feature = "javascript")]
mod javascript {
    use lsp_types::Diagnostic;

    use super::*;
    use crate::javascript::Script;
    use crate::modes::Mode;

    const JS: Mode = Mode::Script(Script::JavaScript);

    fn diagnostics(value: &str) -> Vec<Diagnostic> {
        let document = document("test://test/test.html", value);
        let settings = json!({});
        modes()
            .script_validation(Script::JavaScript, &document, Some(&settings))
            .unwrap()
    }

    fn messages(value: &str) -> Vec<String> {
        diagnostics(value).into_iter().map(|d| d.message).collect()
    }

    #[test]
    fn isolates_classic_and_module_script_declarations() {
        assert_eq!(
            messages(
                "<script>let value = 1;</script><script type=\"module\">let value = 2;</script>"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn isolates_module_var_declarations_from_classic_scripts() {
        assert_eq!(
            messages(
                "<script>let value = 1;</script><script type=\"module\">var value = 2;</script>"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn isolates_declarations_in_separate_module_scripts() {
        assert_eq!(
            messages(
                "<script type=\"module\">var value = 1;</script><script type=\"module\">let value = 2;</script>"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn reports_redeclarations_in_classic_scripts() {
        assert_eq!(
            messages("<script>let value = 1;</script><script>let value = 2;</script>"),
            [
                "Cannot redeclare block-scoped variable 'value'.",
                "Cannot redeclare block-scoped variable 'value'."
            ]
        );
    }

    #[test]
    fn accepts_module_syntax_and_top_level_await() {
        assert_eq!(
            messages(
                "<script type=\"module\">import.meta.url; export const value = await Promise.resolve(1);</script>"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn maps_module_diagnostics_to_the_html_document() {
        let actual: Vec<(Range, String)> =
            diagnostics("<script type=\"module\">\nlet value;\nlet value;\n</script>")
                .into_iter()
                .map(|d| (d.range, d.message))
                .collect();
        let message = "Cannot redeclare block-scoped variable 'value'.".to_owned();
        assert_eq!(
            actual,
            [
                (
                    Range::new(Position::new(1, 4), Position::new(1, 9)),
                    message.clone()
                ),
                (
                    Range::new(Position::new(2, 4), Position::new(2, 9)),
                    message
                )
            ]
        );
    }

    /// `getHoverValue`.
    fn hover_value(value: &str) -> String {
        let (offset, value) = take_cursor(value);
        let document = document("test://test/test.html", &value);
        let position = document.text.position_at(offset.unwrap());
        let modes = modes();
        let mode = modes.mode_at_position(&document, position).unwrap();
        let hover = modes
            .hover(mode, &document, position, None, None)
            .unwrap()
            .expect("expected a hover result");
        match serde_json::to_value(hover.contents).unwrap() {
            Value::String(s) => s,
            Value::Array(a) => a
                .iter()
                .map(|c| c.as_str().or(c["value"].as_str()).unwrap().to_owned())
                .collect::<Vec<_>>()
                .join("\n"),
            c => c["value"].as_str().unwrap().to_owned(),
        }
    }

    /// `/```typescript[\s\S]*name[\s\S]*```/`.
    fn has_signature(value: &str, name: &str) -> bool {
        value.find("```typescript").is_some_and(|start| {
            let rest = &value[start + 13..];
            rest.find(name)
                .is_some_and(|i| rest[i + name.len()..].contains("```"))
        })
    }

    #[test]
    fn javascript_hover_includes_jsdoc_description_for_user_defined_class() {
        let value = hover_value(
            &[
                "<html><head><script>",
                "/**",
                " * A greeter that says hello.",
                " * @param name the person to greet",
                " */",
                "class Greeter {",
                "  constructor(name) { this.name = name; }",
                "}",
                "new Gree|ter(\"world\");",
                "</script></head></html>",
            ]
            .join("\n"),
        );
        assert!(has_signature(&value, "Greeter"), "{value}");
        // Upstream (TypeScript 5): the value also contains "A greeter that says hello.".
        assert_eq!(
            value,
            "```typescript
constructor Greeter(name: any): Greeter
```
"
        );
    }

    #[test]
    fn javascript_hover_includes_jsdoc_description_for_user_defined_method() {
        let value = hover_value(
            &[
                "<html><head><script>",
                "class Greeter {",
                "  /**",
                "   * Returns a friendly greeting.",
                "   */",
                "  sayHello() { return \"hi\"; }",
                "}",
                "new Greeter().sayHe|llo();",
                "</script></head></html>",
            ]
            .join("\n"),
        );
        assert!(has_signature(&value, "sayHello"), "{value}");
        assert!(value.contains("Returns a friendly greeting."), "{value}");
    }

    fn rename(value: &str, new_name: &str) -> (TextDocument, Option<lsp_types::WorkspaceEdit>) {
        let (offset, value) = take_cursor(value);
        let document = document("test://test/test.html", &value);
        let position = document.text.position_at(offset.unwrap());
        let edit = modes().rename(JS, &document, position, new_name).unwrap();
        (document.text, edit)
    }

    /// `testRename`.
    fn assert_rename(input: &[&str], new_name: &str, output: &[&str]) {
        let (text, edit) = rename(&input.join("\n"), new_name);
        let changes = edit.and_then(|e| e.changes).expect("No workspace edits");
        let edits = changes
            .into_iter()
            .find(|(uri, _)| uri.as_str() == text.uri().as_str())
            .map(|(_, edits)| edits)
            .expect("No edits for file");
        assert_eq!(text.apply_edits(&edits), output.join("\n"));
    }

    /// `testNoRename`.
    fn assert_no_rename(input: &[&str], new_name: &str) {
        let (_, edit) = rename(&input.join("\n"), new_name);
        assert!(
            edit.and_then(|e| e.changes).is_none(),
            "Should not rename but rename happened"
        );
    }

    #[test]
    fn rename_variable() {
        assert_rename(
            &[
                "<html>",
                "<head>",
                "<script>",
                "const |a = 2;",
                "const b = a + 2",
                "</script>",
                "</head>",
                "</html>",
            ],
            "h",
            &[
                "<html>",
                "<head>",
                "<script>",
                "const h = 2;",
                "const b = h + 2",
                "</script>",
                "</head>",
                "</html>",
            ],
        );
    }

    #[test]
    fn rename_function() {
        assert_rename(
            &[
                "<html>",
                "<head>",
                "<script>",
                "const name = 'cjg';",
                "function |sayHello(name) {",
                "console.log('hello', name)",
                "}",
                "sayHello(name)",
                "</script>",
                "</head>",
                "</html>",
            ],
            "sayName",
            &[
                "<html>",
                "<head>",
                "<script>",
                "const name = 'cjg';",
                "function sayName(name) {",
                "console.log('hello', name)",
                "}",
                "sayName(name)",
                "</script>",
                "</head>",
                "</html>",
            ],
        );
    }

    #[test]
    fn rename_function_params() {
        assert_rename(
            &[
                "<html>",
                "<head>",
                "<script>",
                "const name = 'cjg';",
                "function sayHello(|name) {",
                "console.log('hello', name)",
                "}",
                "sayHello(name)",
                "</script>",
                "</head>",
                "</html>",
            ],
            "newName",
            &[
                "<html>",
                "<head>",
                "<script>",
                "const name = 'cjg';",
                "function sayHello(newName) {",
                "console.log('hello', newName)",
                "}",
                "sayHello(name)",
                "</script>",
                "</head>",
                "</html>",
            ],
        );
    }

    #[test]
    fn rename_class() {
        assert_rename(
            &[
                "<html>",
                "<head>",
                "<script>",
                "class |Foo {}",
                "const foo = new Foo()",
                "</script>",
                "</head>",
                "</html>",
            ],
            "Bar",
            &[
                "<html>",
                "<head>",
                "<script>",
                "class Bar {}",
                "const foo = new Bar()",
                "</script>",
                "</head>",
                "</html>",
            ],
        );
    }

    #[test]
    fn cannot_rename_literal() {
        assert_no_rename(
            &[
                "<html>",
                "<head>",
                "<script>",
                "const name = |'cjg';",
                "</script>",
                "</head>",
                "</html>",
            ],
            "something",
        );
        assert_no_rename(
            &[
                "<html>",
                "<head>",
                "<script>",
                "const num = |2;",
                "</script>",
                "</head>",
                "</html>",
            ],
            "hhhh",
        );
    }

    #[test]
    fn html_javascript_completions() {
        let uri = "test://test/test.html";
        assert_completions(
            "<html><script>window.|</script></html>",
            uri,
            None,
            &[(
                "location",
                None,
                "<html><script>window.location</script></html>",
                false,
            )],
        );
        // Upstream: `getJSON` after `$.`. No jQuery types, as Zed.
        let (_, items) = completions("<html><script>$.|</script></html>", uri, None);
        assert!(!items.iter().any(|i| i["label"] == "getJSON"));
        assert_completions(
            "<html><script>const x = { a: 1 };</script><script>x.|</script></html>",
            "test://test/test2.html",
            None,
            &[(
                "a",
                None,
                "<html><script>const x = { a: 1 };</script><script>x.a</script></html>",
                false,
            )],
        );
    }

    #[test]
    fn folding_embedded_javascript() {
        let input = [
            "<html>",
            "<head>",
            "<script>",
            "function f() {",
            "}",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_folding(
            &input,
            &[(0, 6, None), (1, 5, None), (2, 4, None), (3, 4, None)],
            None,
        );
    }

    #[test]
    fn folding_embedded_javascript_multiple_areas() {
        let input = [
            "<html>",
            "<head>",
            "<script>",
            "  var x = {",
            "    foo: true,",
            "    bar: {}",
            "  };",
            "</script>",
            "<script>",
            "  test(() => { // hello",
            "    f();",
            "  });",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_folding(
            &input,
            &[
                (0, 13, None),
                (1, 12, None),
                (2, 6, None),
                (3, 6, None),
                (8, 11, None),
                (9, 11, None),
                (9, 11, None),
            ],
            None,
        );
    }

    #[test]
    fn folding_embedded_javascript_incomplete() {
        let input = [
            "<html>",
            "<head>",
            "<script>",
            "  var x = {",
            "</script>",
            "<script>",
            "  });",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_folding(
            &input,
            &[(0, 8, None), (1, 7, None), (2, 3, None), (5, 6, None)],
            None,
        );
    }

    #[test]
    fn folding_embedded_javascript_regions() {
        let input = [
            "<html>",
            "<head>",
            "<script>",
            "  // #region Lalala",
            "   //  #region",
            "   x = 9;",
            "  //  #endregion",
            "  // #endregion Lalala",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_folding(
            &input,
            &[
                (0, 9, None),
                (1, 8, None),
                (2, 7, None),
                (3, 7, Some("region")),
                (4, 6, Some("region")),
            ],
            None,
        );
    }

    #[test]
    fn format_html_and_scripts() {
        for (value, expected) in [
            (
                "<html><head><script>var x=1;</script></head></html>",
                "<html>\n\n<head>\n  <script>var x = 1;</script>\n</head>\n\n</html>",
            ),
            (
                "<html><head><script>\nvar x=2;\n</script></head></html>",
                "<html>\n\n<head>\n  <script>\n    var x = 2;\n  </script>\n</head>\n\n</html>",
            ),
            (
                "<html><head>\n  <script>\nvar x=3;\n</script></head></html>",
                "<html>\n\n<head>\n  <script>\n    var x = 3;\n  </script>\n</head>\n\n</html>",
            ),
            (
                "<html><head>\n  <script>\nvar x=4;\nconsole.log(\"Hi\");\n</script></head></html>",
                "<html>\n\n<head>\n  <script>\n    var x = 4;\n    console.log(\"Hi\");\n  </script>\n</head>\n\n</html>",
            ),
            (
                "<html><head>\n  |<script>\nvar x=5;\n</script>|</head></html>",
                "<html><head>\n  <script>\n    var x = 5;\n  </script></head></html>",
            ),
        ] {
            assert_format(value, expected, None, None);
        }
    }

    #[test]
    fn format_script_end_tag() {
        assert_format(
            "<html>\n<head>\n  <script>\nvar x  =  0;\n</script></head></html>",
            "<html>\n\n<head>\n  <script>\n    var x = 0;\n  </script>\n</head>\n\n</html>",
            None,
            None,
        );
    }

    #[test]
    fn format_html_and_multiple_scripts() {
        assert_format(
            "<html><head>\n<script>\nif(x){\nbar(); }\n</script><script>\nfunction(x){    }\n</script></head></html>",
            "<html>\n\n<head>\n  <script>\n    if (x) {\n      bar();\n    }\n  </script>\n  <script>\n    function(x) { }\n  </script>\n</head>\n\n</html>",
            None,
            None,
        );
    }

    #[test]
    fn format_end_with_newline_script() {
        assert_format(
            "<html><head><script>\nvar x=1;\n</script></head></html>",
            "<html>\n\n<head>\n  <script>\n    var x = 1;\n  </script>\n</head>\n\n</html>\n",
            Some(&json!({})),
            Some(json!({ "tabSize": 2, "insertSpaces": true, "insertFinalNewline": true })),
        );
    }

    #[test]
    fn format_inside_script() {
        assert_format(
            "<html><head>\n  <script>\n|var x=6;|\n</script></head></html>",
            "<html><head>\n  <script>\n  var x = 6;\n</script></head></html>",
            None,
            None,
        );
        assert_format(
            "<html><head>\n  <script>\n|var x=6;\nvar y=  9;|\n</script></head></html>",
            "<html><head>\n  <script>\n  var x = 6;\n  var y = 9;\n</script></head></html>",
            None,
            None,
        );
    }

    #[test]
    fn format_range_after_new_line() {
        assert_format(
            "<html><head>\n  |<script>\nvar x=6;\n</script>\n|</head></html>",
            "<html><head>\n  <script>\n    var x = 6;\n  </script>\n</head></html>",
            None,
            None,
        );
    }

    #[test]
    fn format_bug_48049() {
        assert_format(
            &[
                "<html>",
                "<head>",
                "</head>",
                "",
                "<body>",
                "",
                "    <script>",
                "        function f(x) { }",
                "        f(function () {",
                "        // ",
                "",
                "        console.log(\" vsc crashes on formatting\")",
                "        });",
                "    </script>",
                "",
                "",
                "",
                "        </body>",
                "",
                "</html>",
            ]
            .join("\n"),
            &[
                "<html>",
                "",
                "<head>",
                "</head>",
                "",
                "<body>",
                "",
                "  <script>",
                "    function f(x) { }",
                "    f(function () {",
                "      // ",
                "",
                "      console.log(\" vsc crashes on formatting\")",
                "    });",
                "  </script>",
                "",
                "",
                "",
                "</body>",
                "",
                "</html>",
            ]
            .join("\n"),
            None,
            None,
        );
    }

    #[test]
    fn selection_embedded_javascript() {
        assert_selection(
            "<html><head><script>  function foo() { return ((1|+2)*6) }</script></head></html>",
            &[
                // Upstream (TypeScript 5): (48, "1").
                (49, "+"),
                (48, "1+2"),
                (47, "(1+2)"),
                (47, "(1+2)*6"),
                (46, "((1+2)*6)"),
                (39, "return ((1+2)*6)"),
                (22, "function foo() { return ((1+2)*6) }"),
                (20, "  function foo() { return ((1+2)*6) }"),
                (12, "<script>  function foo() { return ((1+2)*6) }</script>"),
                (
                    6,
                    "<head><script>  function foo() { return ((1+2)*6) }</script></head>",
                ),
                (
                    0,
                    "<html><head><script>  function foo() { return ((1+2)*6) }</script></head></html>",
                ),
            ],
        );
    }

    type Token = (u32, u32, u32, String);

    fn t(line: u32, character: u32, length: u32, classification: &str) -> Token {
        (line, character, length, classification.to_owned())
    }

    /// `assertTokens`.
    fn assert_tokens(lines: &[&str], expected: &[Token], ranges: Option<Vec<Range>>) {
        let document = document("test://foo/bar.html", &lines.join("\n"));
        let modes = modes();
        let legend = modes.semantic_token_legend();
        let types = legend["types"].as_array().unwrap();
        let modifiers = legend["modifiers"].as_array().unwrap();
        let data = modes.semantic_tokens(&document, ranges).unwrap();
        let mut actual = Vec::new();
        let (mut last_line, mut last_character) = (0, 0);
        for c in data.as_chunks::<5>().0 {
            let line = last_line + c[0];
            let character = if c[0] == 0 {
                last_character + c[1]
            } else {
                c[1]
            };
            let mut classification = vec![types[c[3] as usize].as_str().unwrap()];
            classification.extend(
                modifiers
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| c[4] & (1 << i) != 0)
                    .map(|(_, m)| m.as_str().unwrap()),
            );
            actual.push(t(line, character, c[2], &classification.join(".")));
            last_line = line;
            last_character = character;
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn semantic_tokens_variables() {
        let input = [
            "<html>",
            "<head>",
            "<script>",
            "  var x = 9, y1 = [x];",
            "  try {",
            "    for (const s of y1) { x = s }",
            "  } catch (e) {",
            "    throw y1;",
            "  }",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_tokens(
            &input,
            &[
                t(3, 6, 1, "variable.declaration"),
                t(3, 13, 2, "variable.declaration"),
                t(3, 19, 1, "variable"),
                t(5, 15, 1, "variable.declaration.readonly.local"),
                t(5, 20, 2, "variable"),
                t(5, 26, 1, "variable"),
                t(5, 30, 1, "variable.readonly.local"),
                t(6, 11, 1, "variable.declaration.local"),
                t(7, 10, 2, "variable"),
            ],
            None,
        );
    }

    #[test]
    fn semantic_tokens_functions() {
        let input = [
            "<html>",
            "<head>",
            "<script>",
            "  function foo(p1) {",
            "    return foo(Math.abs(p1))",
            "  }",
            "  `/${window.location}`.split(\"/\").forEach(s => foo(s));",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_tokens(
            &input,
            &[
                t(3, 11, 3, "function.declaration"),
                t(3, 15, 2, "parameter.declaration"),
                t(4, 11, 3, "function"),
                t(4, 15, 4, "variable.defaultLibrary"),
                t(4, 20, 3, "method.defaultLibrary"),
                t(4, 24, 2, "parameter"),
                t(6, 6, 6, "variable.defaultLibrary"),
                t(6, 13, 8, "property.defaultLibrary"),
                t(6, 24, 5, "method.defaultLibrary"),
                t(6, 35, 7, "method.defaultLibrary"),
                t(6, 43, 1, "parameter.declaration"),
                t(6, 48, 3, "function"),
                t(6, 52, 1, "parameter"),
            ],
            None,
        );
    }

    #[test]
    fn semantic_tokens_members() {
        let input = [
            "<html>",
            "<head>",
            "<script>",
            "  class A {",
            "    static x = 9;",
            "    f = 9;",
            "    async m() { return A.x + await this.m(); };",
            "    get s() { return this.f; ",
            "    static t() { return new A().f; };",
            "    constructor() {}",
            "  }",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_tokens(
            &input,
            &[
                t(3, 8, 1, "class.declaration"),
                t(4, 11, 1, "property.declaration.static"),
                t(5, 4, 1, "property.declaration"),
                t(6, 10, 1, "method.declaration.async"),
                t(6, 23, 1, "class"),
                t(6, 25, 1, "property.static"),
                t(6, 40, 1, "method.async"),
                t(7, 8, 1, "property.declaration"),
                t(7, 26, 1, "property"),
                t(8, 11, 1, "method.declaration.static"),
                t(8, 28, 1, "class"),
                t(8, 32, 1, "property"),
            ],
            None,
        );
    }

    #[test]
    fn semantic_tokens_interfaces() {
        let input = [
            "<html>",
            "<head>",
            "<script type=\"text/typescript\">",
            "  interface Position { x: number, y: number };",
            "  const p = { x: 1, y: 2 } as Position;",
            "  const foo = (o: Position) => o.x + o.y;",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_tokens(
            &input,
            &[
                t(3, 12, 8, "interface.declaration"),
                t(3, 23, 1, "property.declaration"),
                t(3, 34, 1, "property.declaration"),
                t(4, 8, 1, "variable.declaration.readonly"),
                t(4, 14, 1, "property.declaration"),
                t(4, 20, 1, "property.declaration"),
                t(4, 30, 8, "interface"),
                t(5, 8, 3, "function.declaration.readonly"),
                t(5, 15, 1, "parameter.declaration"),
                t(5, 18, 8, "interface"),
                t(5, 31, 1, "parameter"),
                t(5, 33, 1, "property"),
                t(5, 37, 1, "parameter"),
                t(5, 39, 1, "property"),
            ],
            None,
        );
    }

    #[test]
    fn semantic_tokens_readonly() {
        let input = [
            "<html>",
            "<head>",
            "<script type=\"text/typescript\">",
            "  const f = 9;",
            "  class A { static readonly t = 9; static url: URL; }",
            "  const enum E { A = 9, B = A + 1 }",
            "  console.log(f + A.t + A.url.origin);",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_tokens(
            &input,
            &[
                t(3, 8, 1, "variable.declaration.readonly"),
                t(4, 8, 1, "class.declaration"),
                t(4, 28, 1, "property.declaration.static.readonly"),
                t(4, 42, 3, "property.declaration.static"),
                t(4, 47, 3, "interface.defaultLibrary"),
                t(5, 13, 1, "enum.declaration"),
                t(5, 17, 1, "enumMember.declaration.readonly"),
                t(5, 24, 1, "enumMember.declaration.readonly"),
                t(5, 28, 1, "enumMember.readonly"),
                t(6, 2, 7, "variable.defaultLibrary"),
                t(6, 10, 3, "method.defaultLibrary"),
                t(6, 14, 1, "variable.readonly"),
                t(6, 18, 1, "class"),
                t(6, 20, 1, "property.static.readonly"),
                t(6, 24, 1, "class"),
                t(6, 26, 3, "property.static"),
                t(6, 30, 6, "property.readonly.defaultLibrary"),
            ],
            None,
        );
    }

    #[test]
    fn semantic_tokens_type_aliases_and_type_parameters() {
        let input = [
            "<html>",
            "<head>",
            "<script type=\"text/typescript\">",
            "  type MyMap = Map<string, number>;",
            "  function f<T extends MyMap>(t: T | number) : T { ",
            "    return <T> <unknown> new Map<string, MyMap>();",
            "  }",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_tokens(
            &input,
            &[
                t(3, 7, 5, "type.declaration"),
                t(3, 15, 3, "interface.defaultLibrary"),
                t(4, 11, 1, "function.declaration"),
                t(4, 13, 1, "typeParameter.declaration"),
                t(4, 23, 5, "type"),
                t(4, 30, 1, "parameter.declaration"),
                t(4, 33, 1, "typeParameter"),
                t(4, 47, 1, "typeParameter"),
                t(5, 12, 1, "typeParameter"),
                t(5, 29, 3, "class.defaultLibrary"),
                t(5, 41, 5, "type"),
            ],
            None,
        );
    }

    #[test]
    fn semantic_tokens_ts_and_js() {
        let input = [
            "<html>",
            "<head>",
            "<script type=\"text/typescript\">",
            "  function f<T>(p1: T): T[] { return [ p1 ]; }",
            "</script>",
            "<script>",
            "  window.alert(\"Hello\");",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_tokens(
            &input,
            &[
                t(3, 11, 1, "function.declaration"),
                t(3, 13, 1, "typeParameter.declaration"),
                t(3, 16, 2, "parameter.declaration"),
                t(3, 20, 1, "typeParameter"),
                t(3, 24, 1, "typeParameter"),
                t(3, 39, 2, "parameter"),
                t(6, 2, 6, "variable.defaultLibrary"),
                t(6, 9, 5, "method.defaultLibrary"),
            ],
            None,
        );
    }

    #[test]
    fn semantic_tokens_ranges() {
        let input = [
            "<html>",
            "<head>",
            "<script>",
            "  window.alert(\"Hello\");",
            "</script>",
            "<script>",
            "  window.alert(\"World\");",
            "</script>",
            "</head>",
            "</html>",
        ];
        assert_tokens(
            &input,
            &[
                t(3, 2, 6, "variable.defaultLibrary"),
                t(3, 9, 5, "method.defaultLibrary"),
            ],
            Some(vec![Range::new(Position::new(2, 0), Position::new(4, 0))]),
        );
        assert_tokens(
            &input,
            &[t(6, 2, 6, "variable.defaultLibrary")],
            Some(vec![Range::new(Position::new(6, 2), Position::new(6, 8))]),
        );
    }

    /// Not upstream tests: the changes of the tsrs results.
    fn at(value: &str) -> (Document, Position) {
        let (offset, text) = take_cursor(value);
        let document = document("test://test/test.html", &text);
        let position = document.text.position_at(offset.unwrap());
        (document, position)
    }

    #[test]
    fn highlights_are_write_or_text() {
        let (document, position) = at("<script>var |x = 1; x = 2; f(x);</script>");
        let kinds: Vec<_> = modes()
            .document_highlights(JS, &document, position)
            .unwrap()
            .iter()
            .map(|h| h.kind.map(|k| format!("{k:?}")))
            .collect();
        let (write, text) = (Some("Write".to_owned()), Some("Text".to_owned()));
        assert_eq!(kinds, [write.clone(), write, text]);
    }

    #[test]
    fn references_only_in_document() {
        let (document, position) = at("<script>Math.|max(1, 2); Math.max(3, 4);</script>");
        let references = modes().references(JS, &document, position).unwrap();
        assert_eq!(references.len(), 2);
        assert!(
            references
                .iter()
                .all(|r| r.uri.as_str() == "test://test/test.html")
        );
    }

    #[test]
    fn completion_without_insert_text() {
        let (document, position) = at("<script>var o = { 'a-b': 1, c: 2 }; o.|</script>");
        let folders = Vec::new();
        let context = WorkspaceContext {
            document_uri: "test://test/test.html",
            workspace_folders: &folders,
        };
        let list = modes()
            .complete(JS, &document, position, &context, None)
            .unwrap();
        let list: Value = serde_json::from_str(list.get()).unwrap();
        let labels: Vec<_> = list["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap().to_owned())
            .collect();
        // `a-b` needs the insert text `["a-b"]`; `o` is a JavaScript identifier entry.
        assert_eq!(labels, ["c", "o"], "{list}");
    }

    #[test]
    fn definitions() {
        let definition = |value: &str| {
            let (document, position) = at(value);
            modes()
                .definition(JS, &document, position)
                .unwrap()
                .unwrap()
        };
        assert_eq!(
            definition(
                "<script>var x = 1;
|x;</script>"
            ),
            json!([{
                "uri": "test://test/test.html",
                "range": { "start": { "line": 0, "character": 12 }, "end": { "line": 0, "character": 13 } }
            }])
        );
        assert_eq!(definition("<script>|1;</script>"), json!([]));
        let lib = definition("<script>Math.|PI;</script>");
        let uri = lib[0]["uri"].as_str().unwrap();
        assert_eq!(uri, "html-server://javascript/libs/lib.es5.d.ts");
        let content = modes().text_document_content(uri).unwrap();
        let line = lib[0]["range"]["start"]["line"].as_u64().unwrap() as usize;
        assert!(content.lines().nth(line).unwrap().contains("PI"));
        assert_eq!(definition("<script>|$;</script>"), json!([]));
    }
}
