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

/// An integer setting: a number, or a string with a number. A fraction is cut.
pub fn integer(value: &Value) -> Option<i64> {
    let n = match value {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    n.is_finite().then(|| n.trunc() as i64)
}

const BRACE_STYLES: [&str; 5] = [
    "collapse",
    "expand",
    "end-expand",
    "none",
    "preserve-inline",
];

/// The `brace_style` option of `beautify-css.js`: a list of names, the last known one
/// wins, and only `expand` is not `collapse`.
pub fn brace_style(value: &Value) -> Option<BraceStyle> {
    let names: Vec<&str> = match value {
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        Value::String(s) => s
            .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '-')))
            .collect(),
        _ => return None,
    };
    let last = names.into_iter().rfind(|n| BRACE_STYLES.contains(n))?;
    Some(if last == "expand" {
        BraceStyle::Expand
    } else {
        BraceStyle::Collapse
    })
}

/// The request `FormattingOptions` (with the `css.format.*` settings that VS Code adds) as
/// `CSSFormatConfiguration`.
pub fn format_configuration(options: &Value) -> CssFormatConfiguration {
    let empty = Map::new();
    let o = options.as_object().unwrap_or(&empty);
    CssFormatConfiguration {
        // `tabSize || 4`. A negative size is 4 too (upstream builds broken indents).
        tab_size: o
            .get("tabSize")
            .and_then(integer)
            .filter(|&n| n > 0)
            .map(|n| n.min(u32::MAX as i64) as u32),
        insert_spaces: Some(truthy(o.get("insertSpaces"))),
        insert_final_newline: flag(o, "insertFinalNewline"),
        newline_between_selectors: flag(o, "newlineBetweenSelectors"),
        newline_between_rules: flag(o, "newlineBetweenRules"),
        space_around_selector_separator: flag(o, "spaceAroundSelectorSeparator"),
        brace_style: o.get("braceStyle").and_then(brace_style),
        preserve_new_lines: flag(o, "preserveNewLines"),
        max_preserve_new_lines: o
            .get("maxPreserveNewLines")
            .filter(|v| !v.is_null())
            .and_then(integer),
        indent_empty_lines: flag(o, "indentEmptyLines"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn integers() {
        for (value, expected) in [
            (json!(2.5), Some(2)),
            (json!(-2.5), Some(-2)),
            (json!(1e21), Some(i64::MAX)),
            (json!(" 12 "), Some(12)),
            (json!("12px"), None),
            (json!("x"), None),
            (json!(true), None),
        ] {
            assert_eq!(integer(&value), expected, "{value}");
        }
    }

    #[test]
    fn brace_styles() {
        assert_eq!(
            brace_style(&json!("collapse,expand")),
            Some(BraceStyle::Expand)
        );
        assert_eq!(
            brace_style(&json!("expand,none")),
            Some(BraceStyle::Collapse)
        );
        assert_eq!(
            brace_style(&json!("none, ;expand")),
            Some(BraceStyle::Expand)
        );
        assert_eq!(brace_style(&json!(["expand"])), Some(BraceStyle::Expand));
        assert_eq!(brace_style(&json!("expand,bad")), Some(BraceStyle::Expand));
        assert_eq!(brace_style(&json!(3)), None);
        assert_eq!(brace_style(&json!([])), None);
        assert_eq!(brace_style(&json!("bad")), None);
    }
}
