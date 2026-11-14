//! Compares the formatter with upstream. Fixture:
//! `scripts/css/trace-format.mjs`.

use css_languageservice::services::formatter::{CssFormatConfiguration, format};
use lsp_types::{Range, TextEdit};
use serde_json::Value;
use text_document::TextDocument;

fn run(text: &str, range: &Value, options: &Value) -> Vec<TextEdit> {
    let doc = TextDocument::new(
        "test://test/test.css".parse().unwrap(),
        "css",
        1,
        text.to_owned(),
    );
    let range: Option<Range> = serde_json::from_value(range.clone()).unwrap();
    let options: CssFormatConfiguration = if options.is_null() {
        CssFormatConfiguration::default()
    } else {
        serde_json::from_value(options.clone()).unwrap()
    };
    format(&doc, range, &options)
}

#[test]
fn upstream_calls() {
    let calls: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/formatter.json")).expect("fixture");
    for c in &calls {
        let actual = run(c["text"].as_str().unwrap(), &c["range"], &c["options"]);
        let expected: Vec<TextEdit> = serde_json::from_value(c["result"].clone()).unwrap();
        assert_eq!(actual, expected, "{c}");
    }
}

/// `CSS_FORMAT_SWEEP=<absolute path of .scratch/data/css-format-sweep.json>`.
#[test]
fn upstream_sweep() {
    let Ok(path) = std::env::var("CSS_FORMAT_SWEEP") else {
        return;
    };
    let sweep: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let option_sets = sweep["optionSets"].as_array().unwrap();
    let results = sweep["results"].as_array().unwrap();
    let mut failures = 0;
    for r in results {
        let text = r["text"].as_str().unwrap();
        let options = &option_sets[r["options"].as_u64().unwrap() as usize];
        let edits = run(text, &r["range"], options);
        let e = &edits[0];
        let actual = serde_json::json!([
            e.range.start.line,
            e.range.start.character,
            e.range.end.line,
            e.range.end.character,
            e.new_text
        ]);
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
    assert_eq!(
        failures,
        0,
        "{failures} of {} results differ",
        results.len()
    );
}

/// No panics: ranges from each offset to a later offset of the parser sweep
/// inputs. `CSS_SWEEP_INPUTS=<absolute path of .scratch/data/css-sweep-inputs.json>`.
#[test]
fn robustness() {
    let Ok(path) = std::env::var("CSS_SWEEP_INPUTS") else {
        return;
    };
    let inputs: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).expect("inputs");
    let options = [
        CssFormatConfiguration::default(),
        serde_json::from_value(
            serde_json::json!({ "tabSize": 2, "insertSpaces": true, "braceStyle": "expand" }),
        )
        .unwrap(),
    ];
    for c in inputs.iter().filter(|c| c["fn"] == "_parseStylesheet") {
        let input = c["input"].as_str().unwrap();
        let doc = TextDocument::new(
            "test://test/test.css".parse().unwrap(),
            "css",
            1,
            input.to_owned(),
        );
        let offsets: Vec<usize> = (0..=input.len())
            .filter(|&b| input.is_char_boundary(b))
            .collect();
        for o in &options {
            format(&doc, None, o);
            for (i, &start) in offsets.iter().enumerate() {
                let end = offsets[(i + 3).min(offsets.len() - 1)];
                format(&doc, Some(doc.range(start, end)), o);
            }
        }
    }
}
