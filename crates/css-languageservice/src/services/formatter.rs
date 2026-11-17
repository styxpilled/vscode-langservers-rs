//! Port of `cssFormatter.ts`.

use lsp_types::{Range, TextEdit};
use serde::Deserialize;
use text_document::TextDocument;

use crate::parser::{Dialect, Stylesheet};
use crate::printer::{PrintOptions, print};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BraceStyle {
    Collapse,
    Expand,
}

/// `CSSFormatConfiguration`. `wrapLineLength` has no effect.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CssFormatConfiguration {
    pub tab_size: Option<u32>,
    pub insert_spaces: Option<bool>,
    pub insert_final_newline: Option<bool>,
    pub newline_between_selectors: Option<bool>,
    pub newline_between_rules: Option<bool>,
    pub space_around_selector_separator: Option<bool>,
    pub brace_style: Option<BraceStyle>,
    pub preserve_new_lines: Option<bool>,
    pub max_preserve_new_lines: Option<i64>,
    pub indent_empty_lines: Option<bool>,
}

/// `format`. Formats the document, or the statements of one block that
/// `range` touches.
pub fn format(
    document: &TextDocument,
    stylesheet: &Stylesheet,
    dialect: Dialect,
    range: Option<Range>,
    options: &CssFormatConfiguration,
) -> Vec<TextEdit> {
    let range = range.map(|r| document.offset_at(r.start)..document.offset_at(r.end));
    let Some((replaced, new_text)) = print(stylesheet, dialect, &print_options(options), range)
    else {
        return Vec::new();
    };
    let range = Range::new(
        document.position_at(replaced.start),
        document.position_at(replaced.end),
    );
    vec![TextEdit::new(range, new_text)]
}

pub fn print_options(options: &CssFormatConfiguration) -> PrintOptions {
    let preserve_newlines = options.preserve_new_lines.unwrap_or(true);
    PrintOptions {
        indent_size: match options.tab_size {
            None | Some(0) => 4,
            Some(n) => n as usize,
        },
        indent_with_tabs: !options.insert_spaces.unwrap_or(false),
        end_with_newline: options.insert_final_newline.unwrap_or(false),
        selector_separator_newline: options.newline_between_selectors.unwrap_or(true),
        newline_between_rules: options.newline_between_rules.unwrap_or(true),
        space_around_combinator: options.space_around_selector_separator.unwrap_or(false),
        brace_style_expand: options.brace_style == Some(BraceStyle::Expand),
        indent_empty_lines: options.indent_empty_lines.unwrap_or(false),
        max_preserve_newlines: if preserve_newlines {
            options
                .max_preserve_new_lines
                .map_or(usize::MAX, |n| n.max(0) as usize)
        } else {
            0
        },
    }
}
