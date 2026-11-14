use lsp_types::{Range, TextEdit};

use crate::{FormattingOptions, TextDocument};

pub fn format(
    document: &TextDocument,
    range: Option<Range>,
    options: &FormattingOptions,
) -> Vec<TextEdit> {
    let range = range.map(|range| {
        let offset = document.offset_at(range.start);
        jsonc::Range {
            offset,
            length: document.offset_at(range.end).saturating_sub(offset),
        }
    });
    let options = jsonc::FormattingOptions {
        tab_size: options.tab_size as usize,
        insert_spaces: options.insert_spaces,
        insert_final_newline: options.insert_final_newline,
        eol: "\n".to_owned(),
        keep_lines: options.keep_lines,
    };
    jsonc::format(document.text(), range, &options)
        .into_iter()
        .map(|edit| TextEdit {
            range: document.range(edit.offset, edit.offset + edit.length),
            new_text: edit.content,
        })
        .collect()
}
