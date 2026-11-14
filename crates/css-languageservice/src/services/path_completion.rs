//! Port of `pathCompletion.ts`.

use lsp_types::{
    Command, CompletionItem, CompletionItemKind, CompletionTextEdit, Position, Range, TextEdit,
};
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
    /// `computeCompletions`. `None` where upstream throws: `Range.create`
    /// rejects a negative character.
    pub fn compute_completions(
        &self,
        document: &TextDocument,
        context: &dyn DocumentContext,
        file_system: &dyn FileSystemProvider,
    ) -> Option<CompletionList> {
        let mut result = CompletionList::default();
        let suggest = |value: &str, position, range| {
            provide_path_suggestions(value, position, range, document, context, file_system)
        };
        for c in &self.literal_completions {
            if matches!(strip_quotes(&c.uri_value), "." | "..") {
                result.is_incomplete = true;
            } else {
                result
                    .items
                    .extend(suggest(&c.uri_value, c.position, c.range)?);
            }
        }
        for c in &self.import_completions {
            if matches!(strip_quotes(&c.path_value), "." | "..") {
                result.is_incomplete = true;
                continue;
            }
            let mut suggestions = suggest(&c.path_value, c.position, c.range)?;
            if document.language_id() == "scss" {
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
        Some(result)
    }
}

fn provide_path_suggestions(
    path_value: &str,
    position: Position,
    range: Range,
    document: &TextDocument,
    context: &dyn DocumentContext,
    file_system: &dyn FileSystemProvider,
) -> Option<Vec<CompletionItem>> {
    let full_value = strip_quotes(path_value);
    let is_value_quoted = path_value.starts_with(['\'', '"']);
    let cursor = position.character as i64 - range.start.character as i64;
    let value_before_cursor = if is_value_quoted {
        js_slice(full_value, 0, cursor - 1)
    } else {
        js_slice(full_value, 0, cursor)
    };
    let full_value_range = if is_value_quoted {
        shift_range(range, 1, -1)?
    } else {
        range
    };
    let replace_range = path_to_replace_range(&value_before_cursor, full_value, full_value_range)?;
    Some(suggestions(
        document,
        context,
        file_system,
        &value_before_cursor,
        replace_range,
    ))
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
        if !is_dir {
            match join_path(&parent_dir, &[&name]) {
                Ok(path) if path == current_doc_uri => continue,
                Ok(_) => {}
                Err(_) => return Vec::new(),
            }
        }
        result.push(create_completion_item(&name, is_dir, replace_range));
    }
    result
}

fn strip_quotes(value: &str) -> &str {
    if value.starts_with(['\'', '"']) {
        let mut chars = value.chars();
        chars.next();
        chars.next_back();
        chars.as_str()
    } else {
        value
    }
}

/// JS `slice(start, end)` on UTF-16 units.
fn js_slice(s: &str, start: i64, end: i64) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    let len = units.len() as i64;
    let clamp = |i: i64| if i < 0 { (len + i).max(0) } else { i.min(len) } as usize;
    let (start, end) = (clamp(start), clamp(end));
    if start >= end {
        return String::new();
    }
    String::from_utf16_lossy(&units[start..end])
}

fn utf16_index(s: &str, byte: usize) -> i64 {
    s[..byte].encode_utf16().count() as i64
}

fn path_to_replace_range(
    value_before_cursor: &str,
    full_value: &str,
    full_value_range: Range,
) -> Option<Range> {
    let Some(last_slash) = value_before_cursor.rfind('/') else {
        return Some(full_value_range);
    };
    // The cursor is in the middle of the value: replace from the last slash
    // before the cursor.
    let last_slash = utf16_index(value_before_cursor, last_slash);
    let value_after_last_slash = js_slice(full_value, last_slash + 1, i64::MAX);
    let start = shift_position(
        full_value_range.end,
        -(value_after_last_slash.encode_utf16().count() as i64),
    )?;
    // Replace up to a space.
    let end = match value_after_last_slash.find(' ') {
        Some(i) => shift_position(start, utf16_index(&value_after_last_slash, i))?,
        None => full_value_range.end,
    };
    Some(Range::new(start, end))
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

/// `None` for a negative character.
fn shift_position(pos: Position, offset: i64) -> Option<Position> {
    let character = u32::try_from(pos.character as i64 + offset).ok()?;
    Some(Position::new(pos.line, character))
}

fn shift_range(range: Range, start_offset: i64, end_offset: i64) -> Option<Range> {
    Some(Range::new(
        shift_position(range.start, start_offset)?,
        shift_position(range.end, end_offset)?,
    ))
}
