//! Conversion of the `html.*` JSON settings with the JS semantics of the
//! upstream server, `htmlFormatter.ts` and js-beautify's `Options`.

use css_language_server::settings::{brace_style, get, integer, truthy};
use html_languageservice::HoverSettings;
use html_languageservice::beautify::{IndentScripts, Templating, WrapAttributes};
use html_languageservice::services::completion::CompletionConfiguration;
use html_languageservice::services::formatter::{
    EmbeddedCssFormatConfiguration, HtmlFormatConfiguration, TemplatingSetting,
};
use serde::Deserialize;
use serde_json::{Map, Value};

/// `merge(src, {})`: the own keys of an object.
pub fn merge(src: Option<&Value>) -> Map<String, Value> {
    src.and_then(Value::as_object).cloned().unwrap_or_default()
}

/// The options of `doComplete` (`hide_auto_complete_proposals`) and
/// `doQuoteComplete` in the HTML mode.
pub fn completion_configuration(
    html: Option<&Value>,
    hide_auto_complete_proposals: bool,
) -> CompletionConfiguration {
    let mut options = merge(get(html, "suggest"));
    if hide_auto_complete_proposals {
        options.insert(
            "hideAutoCompleteProposals".into(),
            Value::Bool(get(html, "autoClosingTags") == Some(&Value::Bool(true))),
        );
    }
    let default = Value::from("doublequotes");
    let value = get(get(html, "completion"), "attributeDefaultValue")
        .filter(|v| !v.is_null())
        .unwrap_or(&default);
    options.insert("attributeDefaultValue".into(), value.clone());
    CompletionConfiguration::from_json(&Value::Object(options))
}

/// `settings?.html?.hover`.
pub fn hover_settings(html: Option<&Value>) -> Option<HoverSettings> {
    let hover = get(html, "hover").filter(|v| !v.is_null())?;
    Some(HoverSettings {
        documentation: get(Some(hover), "documentation").and_then(Value::as_bool),
        references: get(Some(hover), "references").and_then(Value::as_bool),
    })
}

/// JS `String(value)` for the values that `+ ',script'` concatenates.
fn js_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Array(a) => a
            .iter()
            .map(|v| match v {
                Value::Null => String::new(),
                v => js_string(v),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        Value::Bool(b) => b.to_string(),
    }
}

/// The `format` of the HTML mode: `html.format` with `contentUnformatted`
/// extended by `script`, then the request options.
pub fn format_settings(html: Option<&Value>, request_options: &Value) -> Map<String, Value> {
    let mut settings = merge(get(html, "format"));
    let content_unformatted = match settings.get("contentUnformatted") {
        Some(v) if truthy(Some(v)) => format!("{},script", js_string(v)),
        _ => "script".to_owned(),
    };
    settings.insert(
        "contentUnformatted".into(),
        Value::from(content_unformatted),
    );
    for (k, v) in merge(Some(request_options)) {
        settings.insert(k, v);
    }
    settings
}

/// `getFormatOption`: the value if it is set and not `null`.
fn option<'a>(o: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    o.get(key).filter(|v| !v.is_null())
}

fn boolean(o: &Map<String, Value>, key: &str) -> Option<bool> {
    option(o, key).map(|v| truthy(Some(v)))
}

/// `_get_number`: the default where the value is not a number.
fn number(o: &Map<String, Value>, key: &str, nan: Option<i64>) -> Option<i64> {
    option(o, key).and_then(|v| integer(v).or(nan))
}

fn string(o: &Map<String, Value>, key: &str) -> Option<String> {
    option(o, key).and_then(Value::as_str).map(str::to_owned)
}

/// `_get_selection`: a name, or a list with one name. Other types give the
/// default. `Err` where js-beautify throws.
fn selection<T: for<'de> Deserialize<'de>>(
    o: &Map<String, Value>,
    key: &str,
    js_key: &str,
) -> Result<Option<T>, String> {
    let name = match option(o, key) {
        Some(Value::String(s)) => Value::from(s.as_str()),
        Some(Value::Array(a)) if a.len() == 1 => a[0].clone(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => {
            return Err(format!(
                "Invalid Option Value: The option '{js_key}' can only be one of the following values"
            ));
        }
        _ => return Ok(None),
    };
    T::deserialize(&name).map(Some).map_err(|_| {
        format!("Invalid Option Value: The option '{js_key}' can only be one of the following values\nYou passed in: '{}'", js_string(&name))
    })
}

/// `getTemplatingFormatOption` and `_get_selection_list`.
fn templating(o: &Map<String, Value>) -> Result<TemplatingSetting, String> {
    match option(o, "templating") {
        Some(Value::Bool(true)) => Ok(TemplatingSetting::Enabled(true)),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                Templating::deserialize(v).map_err(|_| {
                    format!("Invalid Option Value: The option 'templating' can contain only the following values\nYou passed in: '{}'", js_string(&Value::Array(a.clone())))
                })
            })
            .collect::<Result<_, _>>()
            .map(TemplatingSetting::List),
        _ => Ok(TemplatingSetting::Enabled(false)),
    }
}

/// `getCSSFormatOption`.
fn css(o: &Map<String, Value>) -> Result<Option<EmbeddedCssFormatConfiguration>, String> {
    let Some(css) = o.get("css").filter(|v| truthy(Some(v))) else {
        return Ok(None);
    };
    let c = merge(Some(css));
    Ok(Some(EmbeddedCssFormatConfiguration {
        newline_between_selectors: boolean(&c, "newlineBetweenSelectors"),
        newline_between_rules: boolean(&c, "newlineBetweenRules"),
        space_around_selector_separator: boolean(&c, "spaceAroundSelectorSeparator"),
        brace_style: c.get("braceStyle").and_then(brace_style),
        preserve_new_lines: boolean(&c, "preserveNewLines"),
        max_preserve_new_lines: number(&c, "maxPreserveNewLines", Some(32786)),
    }))
}

/// `HTMLFormatConfiguration` from the merged settings. `Err` where upstream
/// throws.
pub fn format_configuration(o: &Map<String, Value>) -> Result<HtmlFormatConfiguration, String> {
    // `tabSize || 4`. A negative size is 4 too (upstream builds broken indents).
    let tab_size = option(o, "tabSize")
        .and_then(integer)
        .filter(|&n| n > 0)
        .map(|n| n.min(u32::MAX as i64) as u32);
    Ok(HtmlFormatConfiguration {
        tab_size,
        insert_spaces: Some(truthy(o.get("insertSpaces"))),
        indent_empty_lines: boolean(o, "indentEmptyLines"),
        wrap_line_length: number(o, "wrapLineLength", Some(0)),
        unformatted: string(o, "unformatted"),
        content_unformatted: string(o, "contentUnformatted"),
        indent_inner_html: boolean(o, "indentInnerHtml"),
        wrap_attributes: selection::<WrapAttributes>(o, "wrapAttributes", "wrap_attributes")?,
        wrap_attributes_indent_size: number(o, "wrapAttributesIndentSize", None),
        preserve_new_lines: boolean(o, "preserveNewLines"),
        max_preserve_new_lines: number(o, "maxPreserveNewLines", Some(32786)),
        indent_handlebars: boolean(o, "indentHandlebars"),
        end_with_newline: boolean(o, "endWithNewline"),
        extra_liners: string(o, "extraLiners"),
        indent_scripts: selection::<IndentScripts>(o, "indentScripts", "indent_scripts")?,
        templating: Some(templating(o)?),
        unformatted_content_delimiter: string(o, "unformattedContentDelimiter"),
        css: css(o)?,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn content_unformatted() {
        let s = |html: Value| format_settings(Some(&html), &json!({"tabSize": 2}));
        assert_eq!(
            s(json!({"format": {"contentUnformatted": "pre"}}))["contentUnformatted"],
            "pre,script"
        );
        assert_eq!(
            s(json!({"format": {"contentUnformatted": ["a", null, 2]}}))["contentUnformatted"],
            "a,,2,script"
        );
        assert_eq!(
            s(json!({"format": {"contentUnformatted": ""}}))["contentUnformatted"],
            "script"
        );
        assert_eq!(s(json!({}))["tabSize"], 2);
    }

    #[test]
    fn format_options() {
        let c = |v: Value| format_configuration(v.as_object().unwrap());
        let config = c(json!({
            "tabSize": "3", "insertSpaces": 1, "wrapLineLength": "x", "wrapAttributes": ["force"],
            "templating": ["erb"], "unformatted": 5, "maxPreserveNewLines": null
        }))
        .unwrap();
        assert_eq!(config.tab_size, Some(3));
        assert_eq!(config.insert_spaces, Some(true));
        assert_eq!(config.wrap_line_length, Some(0));
        assert_eq!(config.wrap_attributes, Some(WrapAttributes::Force));
        assert!(
            matches!(config.templating, Some(TemplatingSetting::List(l)) if l == [Templating::Erb])
        );
        assert_eq!(config.unformatted, None);
        assert_eq!(config.max_preserve_new_lines, None);
        assert!(c(json!({"wrapAttributes": "bad"})).is_err());
        assert!(c(json!({"templating": ["bad"]})).is_err());
        assert!(matches!(
            c(json!({"templating": "all"})).unwrap().templating,
            Some(TemplatingSetting::Enabled(false))
        ));
    }
}
