//! Changes of the `experimental` branch (`docs/experimental.md`).

use css_languageservice::services::links::{
    DocumentContext, FileStat, FileSystemProvider, FileType,
};
use html_languageservice::LanguageService;
use lsp_types::{CompletionTextEdit, HoverContents, MarkedString, Position, Range};
use text_document::TextDocument;

fn document(text: &str) -> TextDocument {
    TextDocument::new(
        "test://test/test.html".parse().unwrap(),
        "html",
        1,
        text.to_owned(),
    )
}

#[test]
fn entity_hover_code_point() {
    let ls = LanguageService::default();
    let doc = document("<p>&Afr;</p>");
    let html = ls.parse_html_document(&doc);
    let hover = ls.do_hover(&doc, Position::new(0, 5), &html, None).unwrap();
    let HoverContents::Scalar(MarkedString::String(s)) = hover.contents else {
        panic!("{hover:?}");
    };
    assert!(s.ends_with("'U+1D504'"), "{s}");
}

#[test]
fn endregion_only_at_start() {
    let ls = LanguageService::default();
    let doc = document("<!-- #region -->\n<p>\n<!-- not endregion -->\n<!-- #endregion -->\n");
    let ranges: Vec<(u32, u32)> = ls
        .get_folding_ranges(&doc, None)
        .into_iter()
        .map(|r| (r.start_line, r.end_line))
        .collect();
    assert_eq!(ranges, [(0, 3)]);
}

struct Context;

impl DocumentContext for Context {
    fn resolve_reference(&self, reference: &str, base_url: &str) -> Option<String> {
        Some(format!("{base_url}|{reference}"))
    }
}

struct Fs;

impl FileSystemProvider for Fs {
    fn stat(&self, _: &str) -> Option<FileStat> {
        None
    }

    fn read_directory(&self, _: &str) -> Option<Vec<(String, FileType)>> {
        Some(vec![("a.html".to_owned(), FileType::File)])
    }

    fn get_content(&self, _: &str) -> Option<String> {
        None
    }
}

fn path_edit_range(text: &str, position: Position) -> Range {
    let ls = LanguageService::default();
    let doc = document(text);
    let html = ls.parse_html_document(&doc);
    let list = ls.do_complete2(&doc, position, &html, &Context, Some(&Fs), None, &mut []);
    let item = list.items.iter().find(|i| i.label == "a.html").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("{item:?}");
    };
    edit.range
}

#[test]
fn path_completion_ranges() {
    let r = |l1, c1, l2, c2| Range::new(Position::new(l1, c1), Position::new(l2, c2));
    // A name on two lines: upstream shifts to a negative character and throws.
    assert_eq!(
        path_edit_range("<img src=\"ab/c\nd\">", Position::new(0, 13)),
        r(0, 13, 1, 1)
    );
    // An empty value: upstream gives a reversed range.
    assert_eq!(
        path_edit_range("<img src=>", Position::new(0, 9)),
        r(0, 9, 0, 9)
    );
    // Unclosed quote: upstream drops the last character.
    assert_eq!(
        path_edit_range("<img src=\"./ab", Position::new(0, 14)),
        r(0, 12, 0, 14)
    );
}

#[test]
fn nesting_limit() {
    use html_languageservice::parser::MAX_DEPTH;
    let text = "<div>".repeat(MAX_DEPTH + 2);
    let html = html_languageservice::HtmlDocument::parse(&text, |_| false, false);
    let depth = |mut id| {
        let mut depth = 1;
        while let Some(parent) = html.parent(id) {
            id = parent;
            depth += 1;
        }
        depth
    };
    let deepest = html.find_node_at(text.len());
    assert_eq!(depth(deepest), MAX_DEPTH);
    assert_eq!(html[deepest].start, 5 * (MAX_DEPTH + 1));
    let closed = html.find_node_at(5 * MAX_DEPTH - 1);
    assert_eq!(depth(closed), MAX_DEPTH);
    assert_eq!(html[closed].end, 5 * MAX_DEPTH);
    assert!(!html[closed].closed);
}
