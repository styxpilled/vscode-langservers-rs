//! Port of `pathCompletion.ts`.

use css_languageservice::services::links::{DocumentContext, FileSystemProvider, FileType};
use lsp_types::{
    Command, CompletionItem, CompletionItemKind, CompletionTextEdit, Position, Range, TextEdit,
};
use text_document::TextDocument;

use super::completion::{CompletionList, CompletionParticipant, HtmlAttributeValueContext};
use crate::language_facts::HtmlDataManager;
use crate::utils::utf16_len;

/// `PathCompletionParticipant`.
#[derive(Default)]
pub struct PathCompletionParticipant {
    attribute_completions: Vec<HtmlAttributeValueContext>,
}

impl CompletionParticipant for PathCompletionParticipant {
    fn on_html_attribute_value(&mut self, context: &HtmlAttributeValueContext) {
        if HtmlDataManager::is_path_attribute(&context.tag, &context.attribute) {
            self.attribute_completions.push(context.clone());
        }
    }
}

impl PathCompletionParticipant {
    /// `computeCompletions`. `None` where upstream throws: `Range.create`
    /// rejects a negative character.
    pub fn compute_completions(
        &self,
        document: &TextDocument,
        document_context: &dyn DocumentContext,
        file_system: &dyn FileSystemProvider,
    ) -> Option<CompletionList> {
        let mut result = CompletionList::default();
        for c in &self.attribute_completions {
            let full_value = strip_quotes(document.text_in(c.range));
            if is_completable_path(full_value) {
                if full_value == "." || full_value == ".." {
                    result.is_incomplete = true;
                } else {
                    let replace_range = path_to_replace_range(&c.value, full_value, c.range)?;
                    result.items.extend(provide_path_suggestions(
                        &c.value,
                        replace_range,
                        document,
                        document_context,
                        file_system,
                        c,
                    ));
                }
            }
        }
        Some(result)
    }
}

fn provide_path_suggestions(
    value_before_cursor: &str,
    replace_range: Range,
    document: &TextDocument,
    document_context: &dyn DocumentContext,
    file_system: &dyn FileSystemProvider,
    context: &HtmlAttributeValueContext,
) -> Vec<CompletionItem> {
    let before_last_slash =
        &value_before_cursor[..value_before_cursor.rfind('/').map_or(0, |i| i + 1)];
    let reference = if before_last_slash.is_empty() {
        "."
    } else {
        before_last_slash
    };
    let Some(parent_dir) = document_context.resolve_reference(reference, document.uri().as_str())
    else {
        return Vec::new();
    };
    let Some(infos) = file_system.read_directory(&parent_dir) else {
        return Vec::new();
    };
    let filter = extension_filter(context);
    let mut result = Vec::new();
    for (name, file_type) in infos {
        if name.starts_with('.') {
            continue;
        }
        let is_dir = file_type == FileType::Directory;
        let mut item = completion_item(&name, is_dir, replace_range);
        match filter {
            Some(extensions) if !is_dir => {
                let lower = name.to_lowercase();
                // No filter is exclusive.
                item.sort_text = Some(if extensions.iter().any(|e| lower.ends_with(e)) {
                    format!("0_{name}")
                } else {
                    format!("1_{name}")
                });
                result.push(item);
            }
            _ => result.push(item),
        }
    }
    result
}

/// `getExtensionFilter`: the extensions to sort first.
fn extension_filter(context: &HtmlAttributeValueContext) -> Option<&'static [&'static str]> {
    let (tag, attribute) = (context.tag.as_str(), context.attribute.as_str());
    if tag == "link"
        && attribute == "href"
        && let Some(attributes) = &context.attributes
    {
        let rel = attributes
            .iter()
            .find(|(n, _)| n == "rel")
            .and_then(|(_, v)| v.as_deref());
        match rel {
            Some("stylesheet" | "\"stylesheet\"" | "'stylesheet'") => {
                return Some(&[".css", ".scss", ".sass", ".less"]);
            }
            Some(
                "icon"
                | "\"icon\""
                | "'icon'"
                | "apple-touch-icon"
                | "\"apple-touch-icon\""
                | "'apple-touch-icon'",
            ) => return Some(&[".ico", ".png", ".svg", ".jpg", ".jpeg", ".gif", ".webp"]),
            _ => {}
        }
    }
    if attribute != "src" {
        return None;
    }
    match tag {
        "script" => Some(&[".js", ".mjs", ".cjs", ".ts", ".tsx", ".jsx"]),
        "img" => Some(&[
            ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".bmp", ".ico",
        ]),
        "video" => Some(&[".mp4", ".webm", ".ogg", ".mov", ".avi"]),
        "audio" => Some(&[".mp3", ".wav", ".ogg", ".m4a", ".aac", ".flac"]),
        _ => None,
    }
}

/// `stripQuotes`: `slice(1, -1)` after a leading quote.
fn strip_quotes(full_value: &str) -> &str {
    if full_value.starts_with(['\'', '"']) {
        let inner = &full_value[1..];
        match inner.char_indices().next_back() {
            Some((i, _)) => &inner[..i],
            None => "",
        }
    } else {
        full_value
    }
}

fn is_completable_path(value: &str) -> bool {
    !(value.starts_with("http") || value.starts_with("//"))
}

fn shift_position(pos: Position, offset: i64) -> Option<Position> {
    let character = u32::try_from(i64::from(pos.character) + offset).ok()?;
    Some(Position::new(pos.line, character))
}

/// `pathToReplaceRange`.
fn path_to_replace_range(
    value_before_cursor: &str,
    full_value: &str,
    range: Range,
) -> Option<Range> {
    match value_before_cursor.rfind('/') {
        None => Some(Range::new(
            shift_position(range.start, 1)?,
            shift_position(range.end, -1)?,
        )),
        Some(last_slash) => {
            // The cursor can be in the middle of the value: <script src="./s|rc/test.js">.
            let after_last_slash = full_value.get(last_slash + 1..).unwrap_or("");
            let start = shift_position(range.end, -1 - utf16_len(after_last_slash) as i64)?;
            let end = match after_last_slash.find(' ') {
                Some(ws) => shift_position(start, utf16_len(&after_last_slash[..ws]) as i64)?,
                None => shift_position(range.end, -1)?,
            };
            Some(Range::new(start, end))
        }
    }
}

fn completion_item(name: &str, is_dir: bool, replace_range: Range) -> CompletionItem {
    if is_dir {
        let p = format!("{name}/");
        CompletionItem {
            label: p.clone(),
            kind: Some(CompletionItemKind::FOLDER),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(replace_range, p))),
            command: Some(Command {
                title: "Suggest".to_owned(),
                command: "editor.action.triggerSuggest".to_owned(),
                arguments: None,
            }),
            ..Default::default()
        }
    } else {
        CompletionItem {
            label: name.to_owned(),
            kind: Some(CompletionItemKind::FILE),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(
                replace_range,
                name.to_owned(),
            ))),
            ..Default::default()
        }
    }
}
