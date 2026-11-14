//! Port of `htmlFormatter.ts`.

use css_languageservice::beautify::BeautifyOptions as CssBeautifyOptions;
use css_languageservice::services::formatter::BraceStyle;
use css_languageservice::utils::is_js_whitespace;
use lsp_types::{Position, Range, TextEdit};
use serde::Deserialize;
use text_document::TextDocument;

use crate::beautify::{
    BeautifyOptions, IndentScripts, Templating, VOID_ELEMENTS, WrapAttributes, html_beautify,
};

/// `templating`: a boolean or a list.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum TemplatingSetting {
    Enabled(bool),
    List(Vec<Templating>),
}

/// `EmbeddedCSSFormatConfiguration`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedCssFormatConfiguration {
    pub newline_between_selectors: Option<bool>,
    pub newline_between_rules: Option<bool>,
    pub space_around_selector_separator: Option<bool>,
    pub brace_style: Option<BraceStyle>,
    pub preserve_new_lines: Option<bool>,
    pub max_preserve_new_lines: Option<i64>,
}

/// `HTMLFormatConfiguration`. The numbers are integers: js-beautify converts
/// them with `parseInt`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HtmlFormatConfiguration {
    pub tab_size: Option<u32>,
    pub insert_spaces: Option<bool>,
    pub indent_empty_lines: Option<bool>,
    pub wrap_line_length: Option<i64>,
    pub unformatted: Option<String>,
    pub content_unformatted: Option<String>,
    pub indent_inner_html: Option<bool>,
    pub wrap_attributes: Option<WrapAttributes>,
    pub wrap_attributes_indent_size: Option<i64>,
    pub preserve_new_lines: Option<bool>,
    pub max_preserve_new_lines: Option<i64>,
    pub indent_handlebars: Option<bool>,
    pub end_with_newline: Option<bool>,
    pub extra_liners: Option<String>,
    pub indent_scripts: Option<IndentScripts>,
    pub templating: Option<TemplatingSetting>,
    pub unformatted_content_delimiter: Option<String>,
    pub css: Option<EmbeddedCssFormatConfiguration>,
}

/// `format`.
pub fn format(
    document: &TextDocument,
    range: Option<Range>,
    options: &HtmlFormatConfiguration,
) -> Vec<TextEdit> {
    let text = document.text();
    let b = text.as_bytes();
    let mut value = text;
    let mut includes_end = true;
    let mut initial_indent_level = 0;
    let tab_size = match options.tab_size {
        None | Some(0) => 4,
        Some(n) => n,
    };
    let range = match range {
        Some(range) => {
            let mut start = document.offset_at(range.start);
            let mut end = document.offset_at(range.end);
            if let Some((s, e)) = expand_range_to_ignore_block(text, start, end) {
                start = s;
                end = e;
            }
            // Include all leading whitespace if at the start of the line,
            // else keep at least one whitespace.
            let mut extended_start = start;
            while extended_start > 0 && matches!(b[extended_start - 1], b' ' | b'\t') {
                extended_start -= 1;
            }
            if extended_start == 0 || matches!(b[extended_start - 1], b'\r' | b'\n') {
                start = extended_start;
            } else if extended_start < start {
                start = extended_start + 1;
            }
            // Include all following whitespace up to the end of the line.
            let mut extended_end = end;
            while extended_end < b.len() && matches!(b[extended_end], b' ' | b'\t') {
                extended_end += 1;
            }
            if extended_end == b.len() || matches!(b[extended_end], b'\r' | b'\n') {
                end = extended_end;
            }
            let range = Range::new(document.position_at(start), document.position_at(end));
            let substring = &text[start.min(end)..start.max(end)];
            // Do not change a range that starts inside a tag.
            let first_half = &text[..start];
            if first_half
                .rfind('<')
                .is_some_and(|lt| !first_half[lt..].contains('>'))
            {
                return vec![TextEdit::new(range, substring.to_owned())];
            }
            includes_end = end == b.len();
            value = substring;
            if start != 0 {
                let start_of_line = document.offset_at(Position::new(range.start.line, 0));
                initial_indent_level = compute_indent_level(text, start_of_line, tab_size);
            }
            range
        }
        None => Range::new(Position::new(0, 0), document.position_at(text.len())),
    };

    let beautify_options = beautify_options(options, tab_size, includes_end);
    let mut result = html_beautify(
        value.trim_start_matches(is_js_whitespace),
        &beautify_options,
    );
    if initial_indent_level > 0 {
        let indent = if options.insert_spaces.unwrap_or(false) {
            " ".repeat((tab_size * initial_indent_level) as usize)
        } else {
            "\t".repeat(initial_indent_level as usize)
        };
        result = result.replace('\n', &format!("\n{indent}"));
        if range.start.character == 0 {
            result.insert_str(0, &indent);
        }
    }
    if result == value {
        return Vec::new();
    }
    vec![TextEdit::new(range, result)]
}

/// `getTagsFormatOption`.
fn tags_option(value: &Option<String>) -> Option<Vec<String>> {
    value.as_ref().map(|list| {
        if list.is_empty() {
            Vec::new()
        } else {
            list.split(',')
                .map(|t| t.trim_matches(is_js_whitespace).to_lowercase())
                .collect()
        }
    })
}

/// The options that `format` passes to `html_beautify`, converted as
/// js-beautify's `Options` does.
fn beautify_options(
    options: &HtmlFormatConfiguration,
    tab_size: u32,
    includes_end: bool,
) -> BeautifyOptions {
    let indent_with_tabs = !options.insert_spaces.unwrap_or(false);
    let mut indent_size = i64::from(tab_size);
    if indent_with_tabs && indent_size == 1 {
        indent_size = 4;
    }
    let preserve_newlines = options.preserve_new_lines.unwrap_or(true);
    let max_preserve_newlines = options.max_preserve_new_lines.unwrap_or(32786);
    let end_with_newline = includes_end && options.end_with_newline.unwrap_or(false);
    let indent_empty_lines = options.indent_empty_lines.unwrap_or(false);
    let templating = match &options.templating {
        Some(TemplatingSetting::Enabled(true)) => vec![Templating::Auto],
        Some(TemplatingSetting::List(list)) if !list.is_empty() => list.clone(),
        _ => vec![Templating::None],
    };
    let templating = if templating == [Templating::Auto] {
        vec![
            Templating::Django,
            Templating::Erb,
            Templating::Handlebars,
            Templating::Php,
        ]
    } else {
        templating
    };
    // `_mergeOpts(options, 'css')`: the fields of `css` replace those of the
    // HTML options, also when they are not set.
    let css = options.css.as_ref();
    let (css_preserve_newlines, css_max_preserve_newlines) = match css {
        Some(css) => (
            css.preserve_new_lines.unwrap_or(true),
            css.max_preserve_new_lines.unwrap_or(32786),
        ),
        None => (preserve_newlines, max_preserve_newlines),
    };
    let css = css.cloned().unwrap_or_default();
    let mut unformatted_content_delimiter = options
        .unformatted_content_delimiter
        .clone()
        .unwrap_or_default();
    for (escape, c) in [("\\r", "\r"), ("\\n", "\n"), ("\\t", "\t")] {
        unformatted_content_delimiter = unformatted_content_delimiter.replacen(escape, c, 1);
    }
    BeautifyOptions {
        indent_size,
        indent_with_tabs,
        end_with_newline,
        preserve_newlines,
        max_preserve_newlines: if preserve_newlines {
            max_preserve_newlines
        } else {
            0
        },
        wrap_line_length: options.wrap_line_length.unwrap_or(120),
        indent_empty_lines,
        templating,
        indent_inner_html: options.indent_inner_html.unwrap_or(false),
        indent_handlebars: options.indent_handlebars.unwrap_or(false),
        wrap_attributes: options.wrap_attributes.unwrap_or_default(),
        wrap_attributes_indent_size: options.wrap_attributes_indent_size.unwrap_or(indent_size),
        extra_liners: tags_option(&options.extra_liners)
            .unwrap_or_else(|| ["head", "body", "/html"].map(String::from).to_vec()),
        unformatted: tags_option(&options.unformatted).unwrap_or_default(),
        content_unformatted: tags_option(&options.content_unformatted)
            .unwrap_or_else(|| ["pre", "textarea"].map(String::from).to_vec()),
        unformatted_content_delimiter,
        indent_scripts: options.indent_scripts.unwrap_or_default(),
        void_elements: VOID_ELEMENTS.map(String::from).to_vec(),
        css: CssBeautifyOptions {
            indent_size,
            indent_with_tabs,
            end_with_newline,
            selector_separator_newline: css.newline_between_selectors.unwrap_or(true),
            newline_between_rules: css.newline_between_rules.unwrap_or(true),
            space_around_combinator: css.space_around_selector_separator.unwrap_or(false),
            brace_style_expand: css.brace_style == Some(BraceStyle::Expand),
            indent_empty_lines,
            max_preserve_newlines: if css_preserve_newlines {
                css_max_preserve_newlines
            } else {
                0
            },
            loose_directives: true,
        },
    }
}

/// The end of the match of `<!--\s*beautify\s+ignore:<kind>\s*-->` at `i`.
fn ignore_marker_end(text: &str, i: usize, kind: &str) -> Option<usize> {
    let ws = |s: &str| s.len() - s.trim_start_matches(is_js_whitespace).len();
    let mut rest = text[i..].strip_prefix("<!--")?;
    rest = rest[ws(rest)..].strip_prefix("beautify")?;
    let n = ws(rest);
    if n == 0 {
        return None;
    }
    rest = rest[n..].strip_prefix("ignore:")?.strip_prefix(kind)?;
    rest = rest[ws(rest)..].strip_prefix("-->")?;
    Some(text.len() - rest.len())
}

/// `expandRangeToIgnoreBlock`.
fn expand_range_to_ignore_block(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let (ignore_start, ignore_start_end) = text
        .match_indices("<!--")
        .take_while(|&(i, _)| i <= start)
        .filter_map(|(i, _)| Some((i, ignore_marker_end(text, i, "start")?)))
        .last()?;
    let block_end = text[ignore_start_end..]
        .match_indices("<!--")
        .find_map(|(i, _)| ignore_marker_end(text, ignore_start_end + i, "end"))?;
    (start >= ignore_start && end <= block_end).then_some((ignore_start, block_end))
}

fn compute_indent_level(content: &str, offset: usize, tab_size: u32) -> u32 {
    let mut n = 0;
    for c in content[offset..].bytes() {
        match c {
            b' ' => n += 1,
            b'\t' => n += tab_size,
            _ => break,
        }
    }
    n / tab_size
}
