//! Times parse and the services on one style sheet.
//! Usage: `bench <style sheet path> [repeat count] [iterations]`. The text is
//! formatted first, then repeated.

use std::time::{Duration, Instant};

use css_languageservice::language_facts::CssDataManager;
use css_languageservice::parser::{Dialect, parse_stylesheet};
use css_languageservice::services::completion::Completion;
use css_languageservice::services::folding::get_folding_ranges;
use css_languageservice::services::formatter::{CssFormatConfiguration, format};
use css_languageservice::services::hover::do_hover;
use css_languageservice::services::navigation::{find_document_colors, find_document_symbols};
use css_languageservice::services::validation::do_validation;
use lsp_types::Position;
use text_document::TextDocument;

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
    let path = args.next().expect("style sheet path");
    let repeat: usize = args.next().map_or(1, |n| n.parse().unwrap());
    let iterations: u32 = args.next().map_or(5, |n| n.parse().unwrap());
    let dialect = match path.rsplit('.').next() {
        Some("scss") => Dialect::Scss,
        Some("less") => Dialect::Less,
        _ => Dialect::Css,
    };
    let uri: lsp_types::Uri = "file:///a.css".parse().unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    let source_doc = TextDocument::new(uri.clone(), "css", 1, source.clone());
    let formatted = match format(&source_doc, None, &CssFormatConfiguration::default()).first() {
        Some(edit) => edit.new_text.clone(),
        None => source,
    };
    let text = formatted.repeat(repeat);
    println!("{} KB, {} lines", text.len() / 1000, text.lines().count());
    let doc = TextDocument::new(uri, "css", 1, text.clone());
    let data = CssDataManager::new(true, &[]);

    let sheet = time("parse", iterations, || parse_stylesheet(dialect, &text));
    println!(
        "{:>12}  {} nodes of {} bytes",
        "",
        sheet.ast.len(),
        size_of::<css_languageservice::parser::nodes::Node>()
    );
    time("validation", iterations, || {
        do_validation(&doc, &sheet, None, &data).len()
    });
    time("symbols", iterations, || {
        find_document_symbols(&doc, &sheet).len()
    });
    time("colors", iterations, || {
        find_document_colors(&doc, &sheet).len()
    });
    time("folding", iterations, || {
        get_folding_ranges(&doc, None).len()
    });
    time("format", iterations, || {
        format(&doc, None, &CssFormatConfiguration::default()).len()
    });

    // Completion and hover at each 97th offset of the first 20 KB.
    let completion = Completion {
        dialect,
        data: &data,
        markdown: true,
    };
    let small_text: String = text.chars().take(20_000).collect();
    let small_doc = TextDocument::new(
        "file:///b.css".parse().unwrap(),
        "css",
        1,
        small_text.clone(),
    );
    let small = parse_stylesheet(dialect, &small_text);
    let positions: Vec<Position> = (0..small_text.len())
        .step_by(97)
        .filter(|&o| small_text.is_char_boundary(o))
        .map(|o| small_doc.position_at(o))
        .collect();
    println!("{:>12}  {} positions", "", positions.len());
    time("completion", iterations, || {
        positions
            .iter()
            .map(|&p| {
                completion
                    .do_complete(&small_doc, p, &small, None, &mut [])
                    .items
                    .len()
            })
            .sum::<usize>()
    });
    time("hover", iterations, || {
        positions
            .iter()
            .filter(|&&p| do_hover(&small_doc, p, &small, &data, true, None).is_some())
            .count()
    });
}
