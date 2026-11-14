//! Conversion of the JSON settings with the JS semantics of the upstream server and services.

use css_languageservice::services::formatter::{BraceStyle, CssFormatConfiguration};
use css_languageservice::services::links::AliasSettings;
use css_languageservice::{CompletionSettings, HoverSettings, LanguageSettings};
use serde_json::{Map, Value};

/// JS `value?.[key]`. `None` is `undefined`.
pub fn get<'a>(value: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    value?.as_object()?.get(key)
}

pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// A setting that is read as `value ?? default` and then as a boolean.
pub fn flag(object: &Map<String, Value>, key: &str) -> Option<bool> {
    object
        .get(key)
        .filter(|v| !v.is_null())
        .map(|v| truthy(Some(v)))
}

/// The settings of `CSSValidation.doValidation`: `validate === false` and `lint`.
pub fn validation_settings(value: Option<&Value>) -> Option<LanguageSettings> {
    let object = value?.as_object()?;
    Some(LanguageSettings {
        validate: object.get("validate").and_then(Value::as_bool),
        lint: object.get("lint").and_then(Value::as_object).cloned(),
        ..Default::default()
    })
}

pub fn hover_settings(value: Option<&Value>) -> Option<HoverSettings> {
    let object = value?.as_object()?;
    Some(HoverSettings {
        documentation: object.get("documentation").and_then(Value::as_bool),
        references: object.get("references").and_then(Value::as_bool),
    })
}

pub fn completion_settings(value: Option<&Value>) -> Option<CompletionSettings> {
    let object = value?.as_object()?;
    Some(CompletionSettings {
        trigger_property_value_completion: flag(object, "triggerPropertyValueCompletion"),
        complete_property_with_semicolon: flag(object, "completePropertyWithSemicolon"),
    })
}

/// `importAliases`. Upstream joins other values as paths; here they are dropped.
pub fn alias_settings(value: Option<&Value>) -> Option<AliasSettings> {
    let object = value?.as_object()?;
    Some(
        object
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
            .collect(),
    )
}

/// JS `parseInt(value, 10)` for numbers and strings.
pub fn parse_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => {
            let n = n.as_f64()?;
            let a = n.abs();
            // `String(n)` has an exponent here, so `parseInt` reads only the first digit.
            let a = if a >= 1e21 || (a != 0.0 && a < 1e-6) {
                format!("{a:e}").as_bytes()[0] - b'0'
            } else {
                return Some(n.trunc() as i64);
            };
            Some(if n < 0.0 { -(a as i64) } else { a as i64 })
        }
        Value::String(s) => {
            let s = s.trim_start();
            let (negative, digits) = match s.as_bytes().first() {
                Some(b'-') => (true, &s[1..]),
                Some(b'+') => (false, &s[1..]),
                _ => (false, s),
            };
            let end = digits
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(digits.len());
            if end == 0 {
                return None;
            }
            let n = digits[..end].parse::<i64>().unwrap_or(i64::MAX);
            Some(if negative { -n } else { n })
        }
        _ => None,
    }
}

const BRACE_STYLES: [&str; 5] = [
    "collapse",
    "expand",
    "end-expand",
    "none",
    "preserve-inline",
];

/// The `brace_style` option of `beautify-css.js`: a list of names, the last one wins, and
/// only `expand` is not `collapse`. `Err` where upstream throws.
pub fn brace_style(value: &Value) -> Result<Option<BraceStyle>, String> {
    let names: Vec<Value> = match value {
        Value::Null => return Ok(None),
        Value::Array(a) => a.clone(),
        Value::String(s) => {
            // `split(/[^a-zA-Z0-9_\/\-]+/)`: a run of separators gives no empty item.
            let parts: Vec<&str> = s
                .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '-')))
                .collect();
            let last = parts.len() - 1;
            parts
                .iter()
                .enumerate()
                .filter(|&(i, p)| !p.is_empty() || i == 0 || i == last)
                .map(|(_, p)| Value::from(*p))
                .collect()
        }
        _ => return Ok(None),
    };
    let valid = !names.is_empty()
        && names
            .iter()
            .all(|n| n.as_str().is_some_and(|n| BRACE_STYLES.contains(&n)));
    if !valid {
        let passed = match value {
            Value::String(s) => s.clone(),
            Value::Array(a) => a
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    v => v.to_string(),
                })
                .collect::<Vec<_>>()
                .join(","),
            v => v.to_string(),
        };
        return Err(format!(
            "Invalid Option Value: The option 'brace_style' can contain only the following values:\n{}\nYou passed in: '{passed}'",
            BRACE_STYLES.join(",")
        ));
    }
    Ok(Some(if names.last() == Some(&Value::from("expand")) {
        BraceStyle::Expand
    } else {
        BraceStyle::Collapse
    }))
}

/// The request `FormattingOptions` (with the `css.format.*` settings that VS Code adds) as
/// `CSSFormatConfiguration`. `Err` where upstream throws.
pub fn format_configuration(options: &Value) -> Result<CssFormatConfiguration, String> {
    let empty = Map::new();
    let o = options.as_object().unwrap_or(&empty);
    Ok(CssFormatConfiguration {
        // `tabSize || 4`. A negative size is 4 too (upstream builds broken indents).
        tab_size: o
            .get("tabSize")
            .and_then(parse_int)
            .filter(|&n| n > 0)
            .map(|n| n.min(u32::MAX as i64) as u32),
        insert_spaces: Some(truthy(o.get("insertSpaces"))),
        insert_final_newline: flag(o, "insertFinalNewline"),
        newline_between_selectors: flag(o, "newlineBetweenSelectors"),
        newline_between_rules: flag(o, "newlineBetweenRules"),
        space_around_selector_separator: flag(o, "spaceAroundSelectorSeparator"),
        brace_style: match o.get("braceStyle") {
            Some(v) => brace_style(v)?,
            None => None,
        },
        preserve_new_lines: flag(o, "preserveNewLines"),
        max_preserve_new_lines: o
            .get("maxPreserveNewLines")
            .filter(|v| !v.is_null())
            .and_then(parse_int),
        indent_empty_lines: flag(o, "indentEmptyLines"),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parse_int_as_js() {
        for (value, expected) in [
            (json!(2.5), Some(2)),
            (json!(-2.5), Some(-2)),
            (json!(1e21), Some(1)),
            (json!(3e-7), Some(3)),
            (json!(0.000001), Some(0)),
            (json!(" 12px"), Some(12)),
            (json!("-0x1"), Some(0)),
            (json!("x"), None),
            (json!(true), None),
        ] {
            assert_eq!(parse_int(&value), expected, "{value}");
        }
    }

    #[test]
    fn brace_styles() {
        assert_eq!(
            brace_style(&json!("collapse,expand")),
            Ok(Some(BraceStyle::Expand))
        );
        assert_eq!(
            brace_style(&json!("expand,none")),
            Ok(Some(BraceStyle::Collapse))
        );
        assert_eq!(
            brace_style(&json!("none, ;expand")),
            Ok(Some(BraceStyle::Expand))
        );
        assert_eq!(
            brace_style(&json!(["expand"])),
            Ok(Some(BraceStyle::Expand))
        );
        assert_eq!(brace_style(&json!(3)), Ok(None));
        assert!(brace_style(&json!("expand,")).is_err());
        assert!(brace_style(&json!([])).is_err());
        assert!(brace_style(&json!("bad")).is_err());
    }
}
