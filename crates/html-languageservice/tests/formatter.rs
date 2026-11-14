//! Compares the formatter with upstream on many texts and option sets.
//! Sweep: `scripts/html/trace-format.mjs`.

use html_languageservice::services::formatter::{HtmlFormatConfiguration, format};
use lsp_types::Range;
use serde_json::{Value, json};
use text_document::TextDocument;

/// `HTML_FORMAT_SWEEP=<absolute path of .scratch/data/html-format-sweep.json>`.
#[test]
fn upstream_sweep() {
    let Ok(path) = std::env::var("HTML_FORMAT_SWEEP") else {
        return;
    };
    let sweep: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let option_sets = sweep["optionSets"].as_array().unwrap();
    let results = sweep["results"].as_array().unwrap();
    let (mut failures, mut skipped) = (0, 0);
    for r in results {
        let text = r["text"].as_str().unwrap();
        // Upstream counts line lengths in UTF-16 units.
        if text.chars().any(|c| c.len_utf16() == 2) {
            skipped += 1;
            continue;
        }
        let options = &option_sets[r["options"].as_u64().unwrap() as usize];
        let config: HtmlFormatConfiguration = serde_json::from_value(options.clone()).unwrap();
        let range: Option<Range> = serde_json::from_value(r["range"].clone()).unwrap();
        let doc = TextDocument::new(
            "test://test/test.html".parse().unwrap(),
            "html",
            1,
            text.to_owned(),
        );
        let actual = json!(format(&doc, range, &config));
        if actual != r["result"] {
            failures += 1;
            if failures <= 10 {
                eprintln!(
                    "options {options}, range {}\ninput:    {text:?}\nexpected: {}\nactual:   {actual}\n",
                    r["range"], r["result"]
                );
            }
        }
    }
    eprintln!("skipped {skipped} results with characters outside the BMP");
    assert_eq!(
        failures,
        0,
        "{failures} of {} results differ",
        results.len()
    );
}

#[test]
fn line_length_in_characters() {
    let config: HtmlFormatConfiguration =
        serde_json::from_value(json!({ "wrapAttributes": "force-aligned", "insertSpaces": true, "tabSize": 2, "wrapLineLength": 10 })).unwrap();
    let doc = TextDocument::new(
        "test://test/test.html".parse().unwrap(),
        "html",
        1,
        "<p\u{1F600} a=\"1\" b=\"2\"></p>".to_owned(),
    );
    let edits = format(&doc, None, &config);
    assert_eq!(
        edits[0].new_text,
        "<p\u{1F600} a=\"1\"\n    b=\"2\">
  </p>"
    );
}

/// No panics: each prefix of each sweep text, with the option sets that
/// enable templating.
#[test]
fn robustness() {
    let Ok(path) = std::env::var("HTML_FORMAT_SWEEP") else {
        return;
    };
    let sweep: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let configs: Vec<HtmlFormatConfiguration> = [0, 7, 8, 9, 12]
        .iter()
        .map(|&i| serde_json::from_value(sweep["optionSets"][i].clone()).unwrap())
        .collect();
    let mut texts: Vec<&str> = sweep["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["text"].as_str().unwrap())
        .collect();
    texts.dedup();
    for text in texts {
        for (i, _) in text.char_indices() {
            let doc = TextDocument::new(
                "test://test/test.html".parse().unwrap(),
                "html",
                1,
                text[..i].to_owned(),
            );
            for config in &configs {
                format(&doc, None, config);
            }
        }
    }
}
