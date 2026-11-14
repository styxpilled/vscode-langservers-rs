//! Each feature of the language modes at each position of each prefix of
//! non-ASCII texts with embedded CSS: no panics. Positions include the middle
//! of surrogate pairs and one character after each line end.

use css_language_server::fs::RequestService;
use html_languageservice::LanguageService;
use lsp_types::{Position, Range};
use serde_json::json;
use text_document::TextDocument;

use crate::context::WorkspaceContext;
use crate::modes::{Document, LanguageModes, Mode};

const TEXTS: &[&str] = &[
    "<html><head><style>a { color: #fab; } /* x */\n@media screen { b:hover { margin: 0 auto } }</style></head>\n<body style=\"color: red; font: x\" class=a>t</body></html>",
    "<div style='background: url(\"a.png\")'><style>\n.c { @x } d{e:f;\n</style><style>{}</style>\n<p>&amp; <img src=b.png></p>",
    "<!DOCTYPE html>\n<a href=\"x\" style=\"\"></a><script>var a = '<style>';</script><style>\n:root { --v: rgb(1, 2, 3) }\n</style\n<b style=color:blue>",
    "<script>
function f(a) { return a.b; }
f(1);</script><script type=\"module\">import x from './y'; <!-- c --> export const z = x</script>
<b onclick=\"f(this)\"><script type=\"text/typescript\">let n: number = 'a'</script>",
    "<style media=\"x\">\r\nh1 { color: hsl(120deg 50% 50%) }\r\n</style>\r\n<!-- <style> --><textarea style=\"a\">x</textarea><ul><li>a<li>b</ul>",
];

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

fn variants(text: &str) -> [String; 2] {
    [
        text.replace('a', "ä").replace('o', "😀"),
        text.replace('e', "\u{FEFF}é").replace('t', "日"),
    ]
}

fn positions(text: &str) -> Vec<Position> {
    let mut positions = Vec::new();
    for (line, content) in text.split('\n').enumerate() {
        let length = content.encode_utf16().count() as u32;
        positions.extend((0..=length + 1).map(|c| Position::new(line as u32, c)));
    }
    positions
}

fn run_all(modes: &LanguageModes, text: &str) {
    let document = Document::new(TextDocument::new(
        "file:///w/test.html".parse().unwrap(),
        "html",
        0,
        text.to_owned(),
    ));
    let folders = ["file:///w".to_owned()];
    let context = WorkspaceContext {
        document_uri: "file:///w/test.html",
        workspace_folders: &folders,
    };
    let settings = json!({ "html": { "format": { "wrapLineLength": 20 } } });
    let settings = Some(&settings);
    let all = Range::new(Position::new(0, 0), Position::new(u32::MAX, 0));
    let positions = positions(text);

    modes.css_validation(&document, settings);
    let _ = modes.folding_ranges(&document, None);
    let _ = modes.folding_ranges(&document, Some(2));
    let _ = modes.selection_ranges(&document, &positions);
    let _ = modes.semantic_tokens(&document, None);
    let _ = modes.format(
        &document,
        all,
        &json!({ "tabSize": 2, "insertSpaces": true }),
        settings,
    );
    for mode in modes.modes_in_document(&document) {
        modes.document_links(mode, &document, &context);
        let _ = modes.document_symbols(mode, &document);
        #[cfg(feature = "javascript")]
        if let Mode::Script(script) = mode {
            let _ = modes.script_validation(script, &document, settings);
        }
        for color in modes.document_colors(mode, &document) {
            modes.color_presentations(mode, &document, color.color, color.range);
        }
    }
    for (i, &position) in positions.iter().enumerate() {
        for kind in ["autoClose", "autoQuote"] {
            modes.auto_insert(&document, position, kind, settings);
        }
        if i % 5 == 0 {
            let options = json!({ "tabSize": 4, "insertFinalNewline": true });
            for range in [
                Range::new(position, all.end),
                Range::new(all.start, position),
            ] {
                let _ = modes.format(&document, range, &options, settings);
            }
        }
        let Some(mode) = modes.mode_at_position(&document, position) else {
            continue;
        };
        if (i % 3 == 0 || mode == Mode::Html)
            && let Ok(list) = modes.complete(mode, &document, position, &context, settings)
        {
            let list: serde_json::Value = serde_json::from_str(list.get()).unwrap();
            if let Some(item) = list["items"].get(0) {
                let _ = modes.resolve(&document, item.clone());
            }
        }
        let _ = modes.hover(mode, &document, position, settings, settings);
        let _ = modes.signature_help(mode, &document, position);
        let _ = modes.document_highlights(mode, &document, position);
        let _ = modes.definition(mode, &document, position);
        let _ = modes.references(mode, &document, position);
        let _ = modes.rename(mode, &document, position, "n");
        modes.linked_editing(mode, &document, position);
    }
}

#[test]
fn robustness() {
    if std::env::var_os("HTML_SERVER_ROBUSTNESS").is_none() {
        return;
    }
    let modes = modes();
    for text in TEXTS {
        for variant in variants(text) {
            for (end, _) in variant.char_indices().chain([(variant.len(), ' ')]) {
                run_all(&modes, &variant[..end]);
            }
        }
    }
}
