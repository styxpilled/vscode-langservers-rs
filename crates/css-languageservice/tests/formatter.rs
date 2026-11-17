//! The style sheet printer: expected output for each construct, and checks
//! that hold for any input (same tokens, same parse, idempotence).

use css_languageservice::parser::nodes::{Ast, NodeId, NodeType};
use css_languageservice::parser::{Dialect, parse_stylesheet};
use css_languageservice::services::formatter::{CssFormatConfiguration, format};
use lsp_types::TextEdit;
use serde_json::{Value, json};
use text_document::TextDocument;

fn document(text: &str) -> TextDocument {
    TextDocument::new(
        "test://test/test.css".parse().unwrap(),
        "css",
        1,
        text.to_owned(),
    )
}

fn edits(
    dialect: Dialect,
    text: &str,
    range: Option<(usize, usize)>,
    options: &CssFormatConfiguration,
) -> (TextDocument, Vec<TextEdit>) {
    let doc = document(text);
    let sheet = parse_stylesheet(dialect, text);
    let range = range.map(|(s, e)| doc.range(s, e));
    let edits = format(&doc, &sheet, dialect, range, options);
    (doc, edits)
}

fn formatted(dialect: Dialect, text: &str, options: &CssFormatConfiguration) -> String {
    let (doc, edits) = edits(dialect, text, None, options);
    doc.apply_edits(&edits)
}

fn options(value: Value) -> CssFormatConfiguration {
    serde_json::from_value(value).unwrap()
}

fn two_spaces() -> CssFormatConfiguration {
    options(json!({ "tabSize": 2, "insertSpaces": true }))
}

#[track_caller]
fn check(dialect: Dialect, input: &str, expected: &str) {
    check_with(dialect, input, &two_spaces(), expected);
}

#[track_caller]
fn check_with(dialect: Dialect, input: &str, options: &CssFormatConfiguration, expected: &str) {
    let actual = formatted(dialect, input, options);
    assert_eq!(actual, expected, "input: {input:?}");
    assert_eq!(
        formatted(dialect, &actual, options),
        actual,
        "not idempotent"
    );
}

#[test]
fn rules() {
    check(
        Dialect::Css,
        ".a,.b>c:is(d,e){color:red!important;margin:0 ! important}",
        ".a,\n.b>c:is(d, e) {\n  color: red !important;\n  margin: 0 !important;\n}",
    );
    check(Dialect::Css, ".a{}", ".a {}");
    check(Dialect::Css, "", "");
    check(
        Dialect::Css,
        ".a{color:red}.b{x:y}",
        ".a {\n  color: red;\n}\n\n.b {\n  x: y;\n}",
    );
    check(
        Dialect::Css,
        "a[href^=\"http:\"]{x:y}a[title=\"a{b}\"]{content:\"{;}\"}",
        "a[href^=\"http:\"] {\n  x: y;\n}\n\na[title=\"a{b}\"] {\n  content: \"{;}\";\n}",
    );
}

#[test]
fn values() {
    check(
        Dialect::Css,
        ".a{background:url(data:image/png;base64,AA) , url( b );width:calc( 1px + 2px )}",
        ".a {\n  background: url(data:image/png;base64,AA), url( b );\n  width: calc(1px + 2px);\n}",
    );
    check(
        Dialect::Css,
        ".a{grid-template-areas:\n    \"a b\"\n        \"c d\";}",
        ".a {\n  grid-template-areas:\n    \"a b\"\n    \"c d\";\n}",
    );
    check(
        Dialect::Css,
        ".a{transition:a 1s,\nb 2s}",
        ".a {\n  transition: a 1s,\n    b 2s;\n}",
    );
    check(
        Dialect::Css,
        ":root{--y:  1px  2px;--z:{a:b};--empty:;}",
        ":root {\n  --y: 1px  2px;\n\n  --z:{a:b};\n\n  --empty:;\n}",
    );
    check(
        Dialect::Css,
        ".a{unicode-range:U+0025-00FF,u+4??}",
        ".a {\n  unicode-range: U+0025-00FF, u+4??;\n}",
    );
}

#[test]
fn at_rules() {
    check(
        Dialect::Css,
        "@media screen and (max-width:100px),print{.a{x:y}}",
        "@media screen and (max-width: 100px), print {\n  .a {\n    x: y;\n  }\n}",
    );
    check(
        Dialect::Css,
        "@import url(\"a.css\") screen,print;@layer a,b;",
        "@import url(\"a.css\") screen, print;\n@layer a, b;",
    );
    check(
        Dialect::Css,
        "@supports (display:grid){.a{x:y}}",
        "@supports (display: grid) {\n  .a {\n    x: y;\n  }\n}",
    );
    check(
        Dialect::Css,
        "@keyframes k{from{opacity:0}50%{opacity:.5}}",
        "@keyframes k {\n  from {\n    opacity: 0;\n  }\n\n  50% {\n    opacity: .5;\n  }\n}",
    );
    check(
        Dialect::Css,
        "@page :first{margin:1in;@top-left{content:\"x\"}}",
        "@page :first {\n  margin: 1in;\n\n  @top-left {\n    content: \"x\";\n  }\n}",
    );
}

#[test]
fn nesting() {
    check(
        Dialect::Css,
        ".a{color:red;@media (min-width:1px){color:blue}&:hover{x:y}.b &{x:y}> .c{x:y}}",
        ".a {\n  color: red;\n\n  @media (min-width: 1px) {\n    color: blue;\n  }\n\n  &:hover {\n    x: y;\n  }\n\n  .b & {\n    x: y;\n  }\n\n  >.c {\n    x: y;\n  }\n}",
    );
    check(
        Dialect::Scss,
        ".card{a:hover{x:y}}",
        ".card {\n  a:hover {\n    x: y;\n  }\n}",
    );
}

#[test]
fn comments() {
    check(
        Dialect::Scss,
        ".a{\n  // don't { break\n  color:red; // trailing\n  /* b */\n\n\n  x:y /* c */}",
        ".a {\n  // don't { break\n  color: red; // trailing\n  /* b */\n\n\n  x: y; /* c */\n}",
    );
    check(
        Dialect::Css,
        "/* head */\n.a /* c1 */ , .b{x:y}",
        "/* head */\n.a /* c1 */,\n.b {\n  x: y;\n}",
    );
    check(
        Dialect::Scss,
        ".a{x:1px, // one\n2px}",
        ".a {\n  x: 1px, // one\n    2px;\n}",
    );
}

#[test]
fn scss() {
    check(
        Dialect::Scss,
        "@mixin m($a,$b:2){@if $a==1{x:y}@else if $a==2{x:z}@else{x:w}}",
        "@mixin m($a, $b: 2) {\n  @if $a==1 {\n    x: y;\n  } @else if $a==2 {\n    x: z;\n  } @else {\n    x: w;\n  }\n}",
    );
    check(
        Dialect::Scss,
        "$map:(key1:value1,key2:(a:b,c:d));",
        "$map: (key1: value1, key2: (a: b, c: d));",
    );
    check(
        Dialect::Scss,
        "$map: (\n  a: b,\n      c: (\n  d: e\n  )\n);",
        "$map: (\n  a: b,\n  c: (\n    d: e\n  )\n);",
    );
    check(
        Dialect::Scss,
        "@use \"lib\" with ($a:1);.x{@include m($x:1,2){c:d}@include n;font: {family:x;size:2px}}",
        "@use \"lib\" with ($a: 1);\n\n.x {\n  @include m($x: 1, 2) {\n    c: d;\n  }\n\n  @include n;\n\n  font: {\n    family: x;\n    size: 2px;\n  }\n}",
    );
    check(
        Dialect::Scss,
        ".icon-#{$name}{width:1px}#{$sel} .b{prop-#{$p}:1px}",
        ".icon-#{$name} {\n  width: 1px;\n}\n\n#{$sel} .b {\n  prop-#{$p}: 1px;\n}",
    );
    check(
        Dialect::Scss,
        "@each $k,$v in $map{.c-#{$k}{color:$v}}@function f($a){@return $a*2}$x:1!default;",
        "@each $k, $v in $map {\n  .c-#{$k} {\n    color: $v;\n  }\n}\n\n@function f($a) {\n  @return $a*2;\n}\n\n$x: 1 !default;",
    );
}

#[test]
fn less() {
    check(
        Dialect::Less,
        ".m(@a;@b:2) when (@a>1){width:@a}.x{.m(2;@b:3);.y;@r();}",
        ".m(@a; @b: 2) when (@a>1) {\n  width: @a;\n}\n\n.x {\n  .m(2; @b: 3);\n  .y;\n  @r();\n}",
    );
    check(
        Dialect::Less,
        "@detached:{background:red;};@w:10px;.a{width:~\"calc(100% - @{w})\"}",
        "@detached: {\n  background: red;\n};\n\n@w: 10px;\n\n.a {\n  width: ~\"calc(100% - @{w})\";\n}",
    );
}

#[test]
fn errors_keep_source() {
    check(
        Dialect::Css,
        ".a{color:red;;x:y}",
        ".a {\n  color: red;;\n  x: y;\n}",
    );
    check(
        Dialect::Css,
        ".a{x: }  .b{x:y}",
        ".a {
  x:
}

.b {
  x: y;
}",
    );
    check(Dialect::Css, ".a{x:y", ".a{x:y");
}

#[test]
fn ignore_directive() {
    check(
        Dialect::Css,
        ".a{x:y}\n/* beautify ignore:start */\n.b{x:y}  .c{x:y}\n/* beautify ignore:end */\n.d{x:y}",
        ".a {\n  x: y;\n}\n\n/* beautify ignore:start */\n.b{x:y}  .c{x:y}\n/* beautify ignore:end */\n.d {\n  x: y;\n}",
    );
    check(
        Dialect::Css,
        ".a{x:y;/* beautify ignore:start */z:w  ;  q:r}",
        ".a {\n  x: y; /* beautify ignore:start */z:w  ;  q:r\n}",
    );
}

#[test]
fn settings() {
    let o = options(json!({
        "tabSize": 2,
        "insertSpaces": true,
        "newlineBetweenSelectors": false,
        "newlineBetweenRules": false,
        "spaceAroundSelectorSeparator": true,
        "braceStyle": "expand",
        "insertFinalNewline": true,
    }));
    check_with(
        Dialect::Scss,
        ".a,.b>.c{x:y;@if $a{x:y}@else{x:z}}.d{x:y}",
        &o,
        ".a, .b > .c\n{\n  x: y;\n  @if $a\n  {\n    x: y;\n  }\n  @else\n  {\n    x: z;\n  }\n}\n.d\n{\n  x: y;\n}\n",
    );
    check_with(
        Dialect::Css,
        ".a{x:y}",
        &CssFormatConfiguration::default(),
        ".a {\n\tx: y;\n}",
    );
    let o = options(
        json!({ "maxPreserveNewLines": 2, "indentEmptyLines": true, "tabSize": 2, "insertSpaces": true }),
    );
    check_with(
        Dialect::Css,
        ".a{x:y;\n\n\n\nz:w}",
        &o,
        ".a {\n  x: y;\n  \n  z: w;\n}",
    );
    let o = options(json!({ "preserveNewLines": false, "tabSize": 2, "insertSpaces": true }));
    check_with(
        Dialect::Css,
        ".a{x:y;\n\n\n\nz:w}",
        &o,
        ".a {\n  x: y;\n  z: w;\n}",
    );
}

#[test]
fn ranges() {
    let text = ".a{color:red}\n.b {\n  x:y;\n  z:w\n}\n.c{q:r}";
    let o = two_spaces();
    // In one declaration: only that declaration changes.
    let start = text.find("x:y").unwrap();
    let (doc, e) = edits(Dialect::Css, text, Some((start + 1, start + 2)), &o);
    assert_eq!(
        doc.apply_edits(&e),
        ".a{color:red}\n.b {\n  x: y;\n  z:w\n}\n.c{q:r}"
    );
    // Over the declarations of `.b`: they, and the missing `;`.
    let end = text.find("z:w").unwrap() + 3;
    let (doc, e) = edits(Dialect::Css, text, Some((start, end)), &o);
    assert_eq!(
        doc.apply_edits(&e),
        ".a{color:red}\n.b {\n  x: y;\n  z: w;\n}\n.c{q:r}"
    );
    // Over two rules.
    let (doc, e) = edits(Dialect::Css, text, Some((0, start)), &o);
    assert_eq!(
        doc.apply_edits(&e),
        ".a {\n  color: red;\n}\n\n.b {\n  x: y;\n  z: w;\n}\n.c{q:r}"
    );
    // Between two rules: nothing.
    let (_, e) = edits(Dialect::Css, ".a{x:y}\n\n.b{x:y}", Some((8, 8)), &o);
    assert!(e.is_empty());
}

/// The text without whitespace and `;`.
fn stripped(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && *c != ';')
        .collect()
}

fn node_types(ast: &Ast, root: NodeId) -> (Vec<u8>, usize) {
    let mut types = Vec::new();
    let mut errors = 0;
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        types.push(ast.node_type(n) as u8);
        errors += ast.issues(n).len();
        // Its value is an expression before `;`, else a token list.
        if ast.node_type(n) == NodeType::CustomPropertyDeclaration {
            continue;
        }
        let mut children: Vec<_> = ast.children(n).collect();
        children.sort_by_key(|&c| std::cmp::Reverse(ast.offset(c)));
        stack.extend(children);
    }
    (types, errors)
}

/// Returns a description of the first broken check.
fn check_invariants(
    dialect: Dialect,
    text: &str,
    o: &CssFormatConfiguration,
) -> Result<(), String> {
    let output = formatted(dialect, text, o);
    if stripped(&output) != stripped(text) {
        return Err(format!("tokens differ: {output:?}"));
    }
    let before = parse_stylesheet(dialect, text);
    let after = parse_stylesheet(dialect, &output);
    // The whitespace at the end of the text is removed. After a parse error,
    // that can change the error recovery.
    let trimmed = parse_stylesheet(dialect, text.trim_end());
    let after_types = node_types(&after.ast, after.root);
    if node_types(&before.ast, before.root) != after_types
        && node_types(&trimmed.ast, trimmed.root) != after_types
    {
        return Err(format!("parse differs: {output:?}"));
    }
    let again = formatted(dialect, &output, o);
    if again != output {
        return Err(format!("not idempotent: {output:?} then {again:?}"));
    }
    let len = text.len();
    for i in 0..4 {
        let mut start = len * i / 4;
        let mut end = (start + len / 3 + 1).min(len);
        while !text.is_char_boundary(start) {
            start -= 1;
        }
        while !text.is_char_boundary(end) {
            end += 1;
        }
        let (doc, e) = edits(dialect, text, Some((start, end)), o);
        let output = doc.apply_edits(&e);
        if stripped(&output) != stripped(text) {
            return Err(format!("range {start}..{end}: tokens differ: {output:?}"));
        }
        let after = parse_stylesheet(dialect, &output);
        if node_types(&before.ast, before.root) != node_types(&after.ast, after.root) {
            return Err(format!("range {start}..{end}: parse differs: {output:?}"));
        }
    }
    Ok(())
}

fn invariant_options() -> [CssFormatConfiguration; 2] {
    [
        two_spaces(),
        options(json!({
            "newlineBetweenSelectors": false,
            "newlineBetweenRules": false,
            "spaceAroundSelectorSeparator": true,
            "braceStyle": "expand",
            "preserveNewLines": false,
        })),
    ]
}

/// The texts of the tests above in each dialect.
#[test]
fn invariants_on_samples() {
    let samples = [
        ".a,.b>c:is(d,e){color:red!important}@media (x:1px),print{.a{x:y}}",
        ".a{background:url(a;b),url( b );--x:{a:b};grid:\n\"a\"\n\"b\"}",
        "$map:(a:(b:c),d:e);.a{@if $a{x:y}@else{x:z}font: {family:x}}",
        ".m(@a;@b:2) when (@a>1){w:@a}@d:{x:y};.x{.m(2;@b:3);.y;@d();}",
        ".a{// c {\nx:y /* d */}/* e */\n\n\n.b{x:1px, // f\n2px}",
        ".a{x: }  .b{x:y}.c{x:y",
    ];
    for dialect in [Dialect::Css, Dialect::Scss, Dialect::Less] {
        for text in samples {
            for o in &invariant_options() {
                if let Err(e) = check_invariants(dialect, text, o) {
                    panic!("{dialect:?} {text:?}: {e}");
                }
            }
        }
    }
}

/// The parser sweep inputs, and the texts of the format sweep of `main`.
/// `CSS_SWEEP_INPUTS=<absolute path of .scratch/data/css-sweep-inputs.json>`,
/// `CSS_FORMAT_SWEEP=<absolute path of .scratch/data/css-format-sweep.json>`.
#[test]
fn invariants_on_sweeps() {
    let mut texts: Vec<(Dialect, String)> = Vec::new();
    if let Ok(path) = std::env::var("CSS_SWEEP_INPUTS") {
        let inputs: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        for c in inputs.iter().filter(|c| c["fn"] == "_parseStylesheet") {
            let dialect = match c["dialect"].as_str().unwrap() {
                "scss" => Dialect::Scss,
                "less" => Dialect::Less,
                _ => Dialect::Css,
            };
            texts.push((dialect, c["input"].as_str().unwrap().to_owned()));
        }
    }
    if let Ok(path) = std::env::var("CSS_FORMAT_SWEEP") {
        let sweep: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut seen = std::collections::HashSet::new();
        for r in sweep["results"].as_array().unwrap() {
            let text = r["text"].as_str().unwrap();
            if seen.insert(text) {
                for dialect in [Dialect::Css, Dialect::Scss, Dialect::Less] {
                    texts.push((dialect, text.to_owned()));
                }
            }
        }
    }
    let mut failures = 0;
    for (dialect, text) in &texts {
        for o in &invariant_options() {
            if let Err(e) = check_invariants(*dialect, text, o) {
                failures += 1;
                if failures <= 20 {
                    eprintln!("{dialect:?} {text:?}\n  {e}\n");
                }
            }
        }
    }
    eprintln!("{} texts", texts.len());
    assert_eq!(failures, 0, "{failures} failures");
}
