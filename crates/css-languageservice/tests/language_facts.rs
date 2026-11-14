//! Compares the language facts with upstream: the calls of the upstream tests,
//! generated calls, `isColorValue` and `getColorValue` on each node of the test
//! texts, entry descriptions and the data manager. The results of the float
//! functions can differ slightly (`near`): the math library of V8 differs
//! from ours and between Node versions. Everything else is exact.
//! Fixture: `scripts/css/trace-facts.mjs`.

use css_languageservice::data::{CssDataProvider, Description, PropertyData};
use css_languageservice::language_facts::CssDataManager;
use css_languageservice::language_facts::colors::{self, Color, Lab, Lch, Xyz};
use css_languageservice::language_facts::entry::{
    HoverSettings, get_entry_description, get_missing_baseline_browsers, text_to_marked_string,
};
use css_languageservice::parser::nodes::{Ast, NodeId};
use css_languageservice::parser::{Dialect, parse_stylesheet};
use serde_json::{Value, json};

fn fixture() -> Value {
    let text = include_str!("fixtures/language_facts.json");
    serde_json::from_str(text).unwrap()
}

/// The number encoding of the fixture: `String(n)`, with `-0`.
fn num(v: &Value) -> f64 {
    let s = v.as_str().unwrap_or_else(|| panic!("number expected: {v}"));
    s.parse().unwrap_or_else(|_| panic!("bad number {s}"))
}

fn enc(x: f64) -> Value {
    if x == 0.0 && x.is_sign_negative() {
        json!("-0")
    } else {
        json!(format!("{x:?}"))
    }
}

/// Replaces each number string with the `enc` form, for an exact comparison.
fn normalize(v: &Value) -> Value {
    match v {
        Value::String(s) => match s.parse::<f64>() {
            Ok(x) if !s.is_empty() => enc(x),
            _ => v.clone(),
        },
        Value::Array(a) => Value::Array(a.iter().map(normalize).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), normalize(v))).collect())
        }
        _ => v.clone(),
    }
}

/// 1 ULP apart, or less than 1e-12 apart: a subtraction such as
/// `500 * (x - y)` makes a 1 ULP difference of `x` larger.
fn near(a: f64, b: f64) -> bool {
    a == b
        || (a.is_finite() && b.is_finite() && (a - b).abs() < 1e-12)
        || (a.is_finite()
            && b.is_finite()
            && a.signum() == b.signum()
            && a.to_bits().abs_diff(b.to_bits()) <= 1)
}

/// Equal, with numbers (normalized strings) compared with `near`.
fn close(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => {
            x == y || matches!((x.parse::<f64>(), y.parse::<f64>()), (Ok(x), Ok(y)) if near(x, y))
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| close(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| close(v, w)))
        }
        _ => a == b,
    }
}

fn color_arg(v: &Value) -> Color {
    Color {
        red: num(&v["red"]),
        green: num(&v["green"]),
        blue: num(&v["blue"]),
        alpha: num(&v["alpha"]),
    }
}

fn lab_arg(v: &Value) -> Lab {
    Lab {
        l: num(&v["l"]),
        a: num(&v["a"]),
        b: num(&v["b"]),
        alpha: v.get("alpha").filter(|a| !a.is_null()).map(num),
    }
}

fn xyz_arg(v: &Value) -> Xyz {
    Xyz {
        x: num(&v["x"]),
        y: num(&v["y"]),
        z: num(&v["z"]),
        alpha: num(&v["alpha"]),
    }
}

fn color_val(c: Option<Color>) -> Value {
    c.map_or(Value::Null, |c| {
        json!({ "red": enc(c.red), "green": enc(c.green), "blue": enc(c.blue), "alpha": enc(c.alpha) })
    })
}

fn opt(x: Option<f64>) -> Value {
    x.map_or(Value::Null, enc)
}

fn lab_val(l: Lab) -> Value {
    json!({ "l": enc(l.l), "a": enc(l.a), "b": enc(l.b), "alpha": opt(l.alpha) })
}

fn lch_val(l: Lch) -> Value {
    json!({ "l": enc(l.l), "c": enc(l.c), "h": enc(l.h), "alpha": opt(l.alpha) })
}

fn xyz_val(x: Xyz) -> Value {
    json!({ "x": enc(x.x), "y": enc(x.y), "z": enc(x.z), "alpha": enc(x.alpha) })
}

fn run_call(name: &str, args: &[Value]) -> Value {
    let n = |i: usize| num(&args[i]);
    let alpha = |i: usize| args.get(i).map_or(1.0, num);
    let round = |i: usize| args.get(i).and_then(Value::as_bool).unwrap_or(true);
    match name {
        "hexDigit" => enc(f64::from(colors::hex_digit(n(0) as u32))),
        "colorFromHex" => color_val(colors::color_from_hex(args[0].as_str().unwrap())),
        "colorFrom256RGB" => {
            color_val(Some(colors::color_from_256_rgb(n(0), n(1), n(2), alpha(3))))
        }
        "colorFromHSL" => color_val(Some(colors::color_from_hsl(n(0), n(1), n(2), alpha(3)))),
        "colorFromHWB" => color_val(Some(colors::color_from_hwb(n(0), n(1), n(2), alpha(3)))),
        "colorFromLAB" => color_val(Some(colors::color_from_lab(n(0), n(1), n(2), alpha(3)))),
        "colorFromOKLAB" => color_val(Some(colors::color_from_oklab(n(0), n(1), n(2), alpha(3)))),
        "colorFromLCH" => color_val(Some(colors::color_from_lch(n(0), n(1), n(2), alpha(3)))),
        "colorFromOKLCH" => color_val(Some(colors::color_from_oklch(n(0), n(1), n(2), alpha(3)))),
        "labFromLCH" => lab_val(colors::lab_from_lch(n(0), n(1), n(2), alpha(3))),
        "hslFromColor" => {
            let h = colors::hsl_from_color(color_arg(&args[0]));
            json!({ "h": enc(h.h), "s": enc(h.s), "l": enc(h.l), "a": enc(h.a) })
        }
        "hwbFromColor" => {
            let h = colors::hwb_from_color(color_arg(&args[0]));
            json!({ "h": enc(h.h), "w": enc(h.w), "b": enc(h.b), "a": enc(h.a) })
        }
        "xyzFromLAB" => xyz_val(colors::xyz_from_lab(lab_arg(&args[0]))),
        "xyzFromOKLAB" => xyz_val(colors::xyz_from_oklab(lab_arg(&args[0]))),
        "xyzToRGB" => color_val(Some(colors::xyz_to_rgb(xyz_arg(&args[0])))),
        "RGBtoXYZ" => xyz_val(colors::RGBtoXYZ(color_arg(&args[0]))),
        "XYZtoLAB" => lab_val(colors::XYZtoLAB(xyz_arg(&args[0]), round(1))),
        "XYZtoOKLAB" => lab_val(colors::XYZtoOKLAB(xyz_arg(&args[0]), round(1))),
        "labFromColor" => lab_val(colors::lab_from_color(color_arg(&args[0]), round(1))),
        "oklabFromColor" => lab_val(colors::oklab_from_color(color_arg(&args[0]), round(1))),
        "lchFromColor" => lch_val(colors::lch_from_color(color_arg(&args[0]))),
        "oklchFromColor" => lch_val(colors::oklch_from_color(color_arg(&args[0]))),
        "isColorString" => json!(colors::is_color_string(args[0].as_str().unwrap())),
        "textToMarkedString" => json!(text_to_marked_string(args[0].as_str().unwrap())),
        "getMissingBaselineBrowsers" => {
            let browsers: Option<Vec<String>> = serde_json::from_value(args[0].clone()).unwrap();
            json!(get_missing_baseline_browsers(browsers.as_deref()))
        }
        _ => panic!("unknown function {name}"),
    }
}

fn check_calls(calls: &[Value]) -> usize {
    let mut failures = 0;
    for call in calls {
        let name = call["fn"].as_str().unwrap();
        let args = call["args"].as_array().unwrap();
        let actual = run_call(name, args);
        let expected = normalize(&call["result"]);
        if !close(&actual, &expected) {
            failures += 1;
            if failures <= 20 {
                eprintln!("{name}({args:?})\n  expected {expected}\n  actual   {actual}");
            }
        }
    }
    failures
}

fn dialect(name: &str) -> Dialect {
    match name {
        "scss" => Dialect::Scss,
        "less" => Dialect::Less,
        _ => Dialect::Css,
    }
}

fn node_results(ast: &Ast, id: NodeId, path: &mut Vec<usize>, out: &mut Vec<Value>) {
    let is_color = colors::is_color_value(ast, id);
    let color = colors::get_color_value(ast, id);
    if is_color || color.is_some() {
        out.push(json!({ "path": path.clone(), "isColor": is_color, "color": color_val(color) }));
    }
    for (i, child) in ast.children(id).enumerate() {
        path.push(i);
        node_results(ast, child, path, out);
        path.pop();
    }
}

fn check_nodes(cases: &[Value]) -> usize {
    let mut failures = 0;
    for case in cases {
        let text = case["text"].as_str().unwrap();
        let sheet = parse_stylesheet(dialect(case["dialect"].as_str().unwrap()), text);
        let mut actual = Vec::new();
        node_results(&sheet.ast, sheet.root, &mut Vec::new(), &mut actual);
        let expected = normalize(&case["nodes"]);
        if Value::Array(actual.clone()) != expected {
            failures += 1;
            if failures <= 20 {
                eprintln!(
                    "{text}\n  expected {expected}\n  actual   {}",
                    Value::Array(actual)
                );
            }
        }
    }
    failures
}

fn check_entries(entries: &[Value]) -> usize {
    let settings = [
        None,
        Some(HoverSettings {
            documentation: Some(false),
            references: None,
        }),
        Some(HoverSettings {
            documentation: None,
            references: Some(false),
        }),
        Some(HoverSettings {
            documentation: Some(false),
            references: Some(false),
        }),
    ];
    let mut failures = 0;
    for case in entries {
        let entry: PropertyData = serde_json::from_value(case["entry"].clone()).unwrap();
        let mut actual = Vec::new();
        for markdown in [true, false] {
            for s in &settings {
                actual.push(match get_entry_description(&entry, markdown, s.as_ref()) {
                    Some(m) => json!([m.kind, m.value]),
                    None => Value::Null,
                });
            }
        }
        let missing = get_missing_baseline_browsers(entry.browsers.as_deref());
        if Value::Array(actual) != case["descriptions"] || missing != case["missing"] {
            failures += 1;
            if failures <= 5 {
                eprintln!("entry {}", case["entry"]);
            }
        }
    }
    failures
}

#[test]
fn upstream_test_calls() {
    let f = fixture();
    let calls = f["testCalls"].as_array().unwrap();
    assert!(calls.len() > 200);
    assert_eq!(check_calls(calls), 0);
}

#[test]
fn generated_calls() {
    let f = fixture();
    assert_eq!(check_calls(f["calls"].as_array().unwrap()), 0);
}

#[test]
fn color_values_of_nodes() {
    let f = fixture();
    assert_eq!(check_nodes(f["nodes"].as_array().unwrap()), 0);
}

#[test]
fn entry_descriptions() {
    let f = fixture();
    assert_eq!(check_entries(f["entries"].as_array().unwrap()), 0);
}

#[test]
fn data_manager() {
    let f = fixture();
    let m = &f["manager"];
    let manager = CssDataManager::new(true, &[CssDataProvider::new(&m["custom"])]);
    let names = |list: Vec<&str>| json!(list);
    assert_eq!(
        names(
            manager
                .get_properties()
                .iter()
                .map(|e| e.name.as_str())
                .collect()
        ),
        m["properties"]
    );
    assert_eq!(
        names(
            manager
                .get_at_directives()
                .iter()
                .map(|e| e.name.as_str())
                .collect()
        ),
        m["atDirectives"]
    );
    assert_eq!(
        names(
            manager
                .get_pseudo_classes()
                .iter()
                .map(|e| e.name.as_str())
                .collect()
        ),
        m["pseudoClasses"]
    );
    assert_eq!(
        names(
            manager
                .get_pseudo_elements()
                .iter()
                .map(|e| e.name.as_str())
                .collect()
        ),
        m["pseudoElements"]
    );
    for lookup in m["lookups"].as_array().unwrap() {
        let name = lookup[0].as_str().unwrap();
        let description = match manager
            .get_property(name)
            .and_then(|p| p.description.as_ref())
        {
            Some(Description::Text(t)) => json!(t),
            Some(Description::Markup(md)) => json!({ "kind": md.kind, "value": md.value }),
            None => Value::Null,
        };
        let actual = json!([
            name,
            manager.is_known_property(name),
            manager.is_standard_property(name),
            description
        ]);
        assert_eq!(&actual, lookup, "{name}");
    }
}

/// `CSS_FACTS_SWEEP=<absolute path of .scratch/data/css-facts-sweep.json>`.
#[test]
fn upstream_sweep() {
    let Ok(path) = std::env::var("CSS_FACTS_SWEEP") else {
        return;
    };
    let sweep: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let calls = check_calls(sweep["calls"].as_array().unwrap());
    let nodes = check_nodes(sweep["nodes"].as_array().unwrap());
    let entries = check_entries(sweep["entries"].as_array().unwrap());
    assert_eq!((calls, nodes, entries), (0, 0, 0));
}

/// Upstream quirks that are not ported: `docs/differences/css-language-facts.md`.
#[test]
fn quirks_not_ported() {
    let custom =
        json!({ "properties": [{ "name": "10" }, { "name": "2" }, { "name": "constructor" }] });
    let manager = CssDataManager::new(true, &[CssDataProvider::new(&custom)]);
    let names: Vec<&str> = manager
        .get_properties()
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(names[names.len() - 3..], ["10", "2", "constructor"]);
    assert!(manager.get_property("constructor").is_some());
    assert!(!manager.is_known_property("__proto__"));

    let color = |text: &str, selection: &str| {
        let sheet = parse_stylesheet(Dialect::Css, text);
        let offset = text.find(selection).unwrap() as isize;
        let mut node = sheet.ast.get_node_at_offset(sheet.root, offset).unwrap();
        if let Some(p) = sheet.ast.parent(node)
            && sheet.ast.node_type(p) == css_languageservice::parser::nodes::NodeType::Function
        {
            node = p;
        }
        (
            colors::is_color_value(&sheet.ast, node),
            colors::get_color_value(&sheet.ast, node),
        )
    };
    assert_eq!(
        color("a { color: constructor }", "constructor"),
        (false, None)
    );
    assert_eq!(color("a { color: __proto__ }", "__proto__"), (false, None));
    assert!(!color("a { color: h\u{17F}l(0 0% 0%) }", "h\u{17F}l").0);
    let cyan = Some(colors::color_from_256_rgb(0.0, 255.0, 255.0, 1.0));
    assert_eq!(color("a { color: hsl(0.5TURN 100% 50%) }", "hsl").1, cyan);
    assert_eq!(color("a { color: hsl(200Grad 100% 50%) }", "hsl").1, cyan);

    let lab = Lab {
        l: 50.0,
        a: 0.0,
        b: 0.0,
        alpha: Some(0.5),
    };
    assert_eq!(colors::xyz_from_lab(lab).alpha, 0.5);
}
