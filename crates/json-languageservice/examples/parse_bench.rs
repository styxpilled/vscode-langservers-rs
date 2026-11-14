//! Times parse and the features that do not use schemas on one document.
//! Usage: `parse_bench <document path>`

use std::time::Instant;

use json_languageservice::{
    FormattingOptions, TextDocument, find_document_symbols2, get_folding_ranges, parse,
};

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let result = f();
    println!("{label}: {:?}", t.elapsed());
    result
}

fn main() {
    let path = std::env::args().nth(1).expect("document path");
    let text = std::fs::read_to_string(&path).unwrap();
    println!("{} MB", text.len() / 1_000_000);
    let doc = time("text document", || {
        TextDocument::new("file:///a.json".parse().unwrap(), "json", 1, text.clone())
    });
    let mut scanner = jsonc::Scanner::new(&text, false);
    time("jsonc scan", || {
        while scanner.scan() != jsonc::SyntaxKind::EOF {}
    });
    time("jsonc visit", || {
        jsonc::visit(&text, &mut Nothing, jsonc::ParseOptions::default())
    });
    let json = time("parse", || parse(&doc, true));
    let mut count = 0;
    json.visit(|_| {
        count += 1;
        true
    });
    println!(
        "{count} nodes of {} bytes",
        size_of::<json_languageservice::AstNode>()
    );
    time("drop parse", || drop(parse(&doc, true)));
    time("folding", || get_folding_ranges(&doc, None).len());
    time("symbols", || {
        find_document_symbols2(&doc, &json, None).len()
    });
    time("format", || {
        json_languageservice::format(&doc, None, &FormattingOptions::default()).len()
    });
}

struct Nothing;
impl jsonc::Visitor for Nothing {}
