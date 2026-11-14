//! Times parse and the services on one HTML document.
//! Usage: `bench <document path> [iterations]`.

use std::time::{Duration, Instant};

use css_languageservice::services::links::DocumentContext;
use html_languageservice::LanguageService;
use html_languageservice::services::formatter::{HtmlFormatConfiguration, format};
use html_languageservice::services::links::find_document_links;
use html_languageservice::services::navigation::{find_document_symbols, find_document_symbols2};
use text_document::TextDocument;

struct Context;

impl DocumentContext for Context {
    fn resolve_reference(&self, reference: &str, base_url: &str) -> Option<String> {
        Some(format!("{base_url}/{reference}"))
    }
}

fn time<T>(label: &str, iterations: u32, mut f: impl FnMut() -> T) -> T {
    let mut result = f();
    let mut best = Duration::MAX;
    for _ in 0..iterations {
        let t = Instant::now();
        result = f();
        best = best.min(t.elapsed());
    }
    println!("{label:>12}: {best:?}");
    result
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("document path");
    let iterations: u32 = args.next().map_or(10, |n| n.parse().unwrap());
    let text = std::fs::read_to_string(&path).unwrap();
    println!("{} KB, {} lines", text.len() / 1000, text.lines().count());
    let doc = TextDocument::new("file:///a.html".parse().unwrap(), "html", 1, text);
    let ls = LanguageService::default();

    let html = time("parse", iterations, || ls.parse_html_document(&doc));
    time("symbols", iterations, || {
        find_document_symbols(&doc, &html).len()
    });
    time("symbols2", iterations, || {
        find_document_symbols2(&doc, &html).len()
    });
    time("folding", iterations, || {
        ls.get_folding_ranges(&doc, None).len()
    });
    time("links", iterations, || {
        find_document_links(&doc, &Context).len()
    });
    time("format", iterations, || {
        format(&doc, None, &HtmlFormatConfiguration::default()).len()
    });
}
