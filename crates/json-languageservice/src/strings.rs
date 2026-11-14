/// Formats a number as JS `String(n)` does.
pub fn js_number_to_string(n: f64) -> String {
    ryu_js::Buffer::new().format(n).to_owned()
}

/// `extendedRegExp`: a `(?i)` prefix sets the `i` flag. Tries the `u` flag first.
pub fn extended_regexp(pattern: &str) -> Option<regress::Regex> {
    let (pattern, flags) = match pattern.strip_prefix("(?i)") {
        Some(p) => (p, "i"),
        None => (pattern, ""),
    };
    regress::Regex::with_flags(pattern, format!("{flags}u").as_str())
        .or_else(|_| regress::Regex::with_flags(pattern, flags))
        .ok()
}

/// `JSON.stringify` with JS number formatting.
pub fn js_stringify(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::Number(n) => n
            .as_f64()
            .map_or_else(|| n.to_string(), js_number_to_string),
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(js_stringify).collect();
            format!("[{}]", items.join(","))
        }
        Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}:{}", Value::from(k.as_str()), js_stringify(v)))
                .collect();
            format!("{{{}}}", entries.join(","))
        }
        _ => value.to_string(),
    }
}

/// `JSON.stringify(value, null, indent)`.
pub fn js_stringify_indent(value: &serde_json::Value, indent: &str) -> String {
    fn write(value: &serde_json::Value, indent: &str, current: &str, out: &mut String) {
        use serde_json::Value;
        let inner = format!("{current}{indent}");
        match value {
            Value::Array(items) if !items.is_empty() => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    out.push_str(&inner);
                    write(item, indent, &inner, out);
                }
                out.push('\n');
                out.push_str(current);
                out.push(']');
            }
            Value::Object(map) if !map.is_empty() => {
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    out.push_str(&inner);
                    out.push_str(&Value::from(k.as_str()).to_string());
                    out.push_str(": ");
                    write(v, indent, &inner, out);
                }
                out.push('\n');
                out.push_str(current);
                out.push('}');
            }
            _ => out.push_str(&js_stringify(value)),
        }
    }
    let mut out = String::new();
    write(value, indent, "", &mut out);
    out
}

/// `utils/json.ts` `stringifyObject`.
pub fn stringify_object(
    value: &serde_json::Value,
    indent: &str,
    literal: &dyn Fn(&serde_json::Value) -> String,
) -> String {
    use serde_json::Value;
    let inner = format!("{indent}\t");
    match value {
        Value::Array(items) => {
            if items.is_empty() {
                return "[]".to_owned();
            }
            let items: Vec<String> = items
                .iter()
                .map(|v| format!("{inner}{}", stringify_object(v, &inner, literal)))
                .collect();
            format!("[\n{}\n{indent}]", items.join(",\n"))
        }
        Value::Object(map) => {
            if map.is_empty() {
                return "{}".to_owned();
            }
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{inner}{}: {}",
                        Value::from(k.as_str()),
                        stringify_object(v, &inner, literal)
                    )
                })
                .collect();
            format!("{{\n{}\n{indent}}}", entries.join(",\n"))
        }
        _ => literal(value),
    }
}

/// JS `String.prototype.trim`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}')
}

/// The longest prefix of `s` with at most `units` UTF-16 code units. JS `substr` can split a
/// surrogate pair. Here the whole character is dropped.
pub fn utf16_prefix(s: &str, units: usize) -> &str {
    let mut count = 0;
    for (i, c) in s.char_indices() {
        count += c.len_utf16();
        if count > units {
            return &s[..i];
        }
    }
    s
}

/// `objects.ts` `equals`. Numbers compare as `f64`.
pub fn js_equals(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| js_equals(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| js_equals(v, w)))
        }
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::js_number_to_string;

    #[test]
    fn matches_js() {
        let cases = [
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (123456789.0, "123456789"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1.5e-7, "1.5e-7"),
            (0.000001, "0.000001"),
            (-0.0, "0"),
            (f64::NAN, "NaN"),
            (f64::NEG_INFINITY, "-Infinity"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
        ];
        for (n, expected) in cases {
            assert_eq!(js_number_to_string(n), expected);
        }
    }
}
