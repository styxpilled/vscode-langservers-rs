//! Port of `pathCompletion.ts`.

use lsp_types::{Command, CompletionItem, CompletionItemKind, CompletionTextEdit, Range, TextEdit};
use text_document::TextDocument;

use crate::services::completion::{
    CompletionList, CompletionParticipant, ImportPathCompletionContext, UriLiteralCompletionContext,
};
use crate::services::links::{DocumentContext, FileSystemProvider, FileType};
use crate::utils::{is_js_whitespace, join_path};

/// `PathCompletionParticipant`.
#[derive(Default)]
pub struct PathCompletionParticipant {
    pub literal_completions: Vec<UriLiteralCompletionContext>,
    pub import_completions: Vec<ImportPathCompletionContext>,
}

impl CompletionParticipant for PathCompletionParticipant {
    fn on_css_uri_literal_value(&mut self, context: UriLiteralCompletionContext) {
        self.literal_completions.push(context);
    }

    fn on_css_import_path(&mut self, context: ImportPathCompletionContext) {
        self.import_completions.push(context);
    }
}

impl PathCompletionParticipant {
    /// `computeCompletions`. Only for a context whose range is its value and a cursor
    /// in the value. Upstream also completes when the cursor is next to the value
    /// (`u|rl("a")`, after the `)`), with a range away from the cursor.
    pub fn compute_completions(
        &self,
        document: &TextDocument,
        context: &dyn DocumentContext,
        file_system: &dyn FileSystemProvider,
    ) -> CompletionList {
        let mut result = CompletionList::default();
        let values = self
            .literal_completions
            .iter()
            .map(|c| (&c.uri_value, c.position, c.range, false))
            .chain(
                self.import_completions
                    .iter()
                    .map(|c| (&c.path_value, c.position, c.range, true)),
            );
        for (value, position, range, import) in values {
            let (start, end) = (
                document.offset_at(range.start),
                document.offset_at(range.end),
            );
            if document.text()[start..end] != *value {
                continue;
            }
            let (start, end) = strip_quotes(value, start, end);
            if matches!(&document.text()[start..end], "." | "..") {
                result.is_incomplete = true;
                continue;
            }
            let cursor = document.offset_at(position);
            if !(start..=end).contains(&cursor) {
                continue;
            }
            let mut suggestions =
                provide_path_suggestions(document, cursor, start, end, context, file_system);
            if import && document.language_id() == "scss" {
                for s in &mut suggestions {
                    if s.label.starts_with('_')
                        && s.label.ends_with(".scss")
                        && let Some(CompletionTextEdit::Edit(edit)) = &mut s.text_edit
                    {
                        edit.new_text = s.label[1..s.label.len() - 5].to_owned();
                    }
                }
            }
            result.items.extend(suggestions);
        }
        result
    }
}

/// `stripQuotes`: the offsets of `value` at `start..end` without its quotes. Upstream
/// takes the range with `shiftRange(range, 1, -1)`, which assumes one line and two
/// quotes, and throws for a negative character.
fn strip_quotes(value: &str, start: usize, end: usize) -> (usize, usize) {
    match value.chars().next() {
        Some(q @ ('\'' | '"')) if value.len() > 1 && value.ends_with(q) => (start + 1, end - 1),
        Some('\'' | '"') => (start + 1, end),
        _ => (start, end),
    }
}

fn provide_path_suggestions(
    document: &TextDocument,
    cursor: usize,
    start: usize,
    end: usize,
    context: &dyn DocumentContext,
    file_system: &dyn FileSystemProvider,
) -> Vec<CompletionItem> {
    let text = document.text();
    let value_before_cursor = &text[start..cursor];
    let full_value = &text[start..end];
    let replace_range = path_to_replace_range(document, value_before_cursor, full_value, end);
    suggestions(
        document,
        context,
        file_system,
        value_before_cursor,
        replace_range,
    )
}

/// `pathToReplaceRange`: the path segment at the cursor, up to a space.
fn path_to_replace_range(
    document: &TextDocument,
    value_before_cursor: &str,
    full_value: &str,
    end: usize,
) -> Range {
    let start = end - full_value.len();
    let Some(last_slash) = value_before_cursor.rfind('/') else {
        return document.range(start, end);
    };
    let after_last_slash = &full_value[last_slash + 1..];
    let start = end - after_last_slash.len();
    let end = after_last_slash
        .find(' ')
        .map_or(end, |space| start + space);
    document.range(start, end)
}

fn suggestions(
    document: &TextDocument,
    context: &dyn DocumentContext,
    file_system: &dyn FileSystemProvider,
    value_before_cursor: &str,
    replace_range: Range,
) -> Vec<CompletionItem> {
    let current_doc_uri = document.uri().as_str();
    // Keep the last slash.
    let value_before_last_slash =
        &value_before_cursor[..value_before_cursor.rfind('/').map_or(0, |i| i + 1)];
    let parent = if value_before_last_slash.is_empty() {
        "."
    } else {
        value_before_last_slash
    };
    let Some(parent_dir) = context
        .resolve_reference(parent, current_doc_uri)
        .filter(|d| !d.is_empty())
    else {
        return Vec::new();
    };
    let Some(infos) = file_system.read_directory(&parent_dir) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for (name, file_type) in infos {
        // Not the names that start with `.`.
        if name.starts_with('.') {
            continue;
        }
        let is_dir = file_type == FileType::Directory;
        // Upstream rejects when the join throws.
        if !is_dir && join_path(&parent_dir, &[&name]).is_ok_and(|p| p == current_doc_uri) {
            continue;
        }
        result.push(create_completion_item(&name, is_dir, replace_range));
    }
    result
}

fn create_completion_item(name: &str, is_dir: bool, replace_range: Range) -> CompletionItem {
    let name = if is_dir {
        escape_path(&format!("{name}/"))
    } else {
        escape_path(name)
    };
    CompletionItem {
        label: name.clone(),
        kind: Some(if is_dir {
            CompletionItemKind::FOLDER
        } else {
            CompletionItemKind::FILE
        }),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(replace_range, name))),
        command: is_dir.then(|| Command {
            title: "Suggest".into(),
            command: "editor.action.triggerSuggest".into(),
            arguments: None,
        }),
        ..Default::default()
    }
}

/// `escapePath`: https://www.w3.org/TR/CSS1/#url.
fn escape_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.chars() {
        if is_js_whitespace(c) || matches!(c, '(' | ')' | ',' | '"' | '\'') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
