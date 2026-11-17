//! Small completion lists for clients with `completionList.itemDefaults` and
//! `completionItem/resolve` of `documentation`, `detail` or `command`.

use lsp_types::{
    Command, CompletionItem, CompletionTextEdit, Documentation, InsertTextFormat, Range,
};
use serde::Serialize;
use serde_json::{Value, json};

/// What the client reads from `itemDefaults` and resolves later.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompletionSupport {
    pub edit_range: bool,
    pub insert_text_format: bool,
    pub resolve_documentation: bool,
    pub resolve_detail: bool,
    pub resolve_command: bool,
}

impl CompletionSupport {
    pub fn from_capabilities(capabilities: &Value) -> Self {
        let completion = &capabilities["textDocument"]["completion"];
        let contains =
            |list: &Value, name: &str| list.as_array().is_some_and(|a| a.iter().any(|v| v == name));
        let defaults = &completion["completionList"]["itemDefaults"];
        let resolve = &completion["completionItem"]["resolveSupport"]["properties"];
        CompletionSupport {
            edit_range: contains(defaults, "editRange"),
            insert_text_format: contains(defaults, "insertTextFormat"),
            resolve_documentation: contains(resolve, "documentation"),
            resolve_detail: contains(resolve, "detail"),
            resolve_command: contains(resolve, "command"),
        }
    }

    pub fn resolves(&self) -> bool {
        self.resolve_documentation || self.resolve_detail || self.resolve_command
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactList {
    is_incomplete: bool,
    #[serde(skip_serializing_if = "Defaults::is_empty")]
    item_defaults: Defaults,
    items: Vec<CompactItem>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Defaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    edit_range: Option<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    insert_text_format: Option<InsertTextFormat>,
}

impl Defaults {
    fn is_empty(&self) -> bool {
        self.edit_range.is_none() && self.insert_text_format.is_none()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompactItem {
    #[serde(flatten)]
    item: CompletionItem,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_edit_text: Option<String>,
}

/// The resolvable properties of the last list, for `completionItem/resolve`.
#[derive(Default)]
pub struct ResolveStore {
    generation: u64,
    items: Vec<Lazy>,
}

#[derive(Serialize)]
struct Lazy {
    #[serde(skip_serializing_if = "Option::is_none")]
    documentation: Option<Documentation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<Command>,
}

impl ResolveStore {
    /// `edit_range`: the `itemDefaults.editRange` of the service, if any. Without
    /// one, the most common edit range of the items is the default.
    pub fn compact(
        &mut self,
        support: CompletionSupport,
        is_incomplete: bool,
        mut items: Vec<CompletionItem>,
        edit_range: Option<Range>,
    ) -> CompactList {
        let mut defaults = Defaults {
            edit_range: edit_range.or_else(|| support.edit_range.then(|| common_range(&items))?),
            insert_text_format: None,
        };
        if support.insert_text_format {
            let snippets = items
                .iter()
                .filter(|i| i.insert_text_format == Some(InsertTextFormat::SNIPPET))
                .count();
            if snippets * 2 > items.len() {
                defaults.insert_text_format = Some(InsertTextFormat::SNIPPET);
                for item in &mut items {
                    item.insert_text_format = match item.insert_text_format {
                        Some(InsertTextFormat::SNIPPET) => None,
                        _ => Some(InsertTextFormat::PLAIN_TEXT),
                    };
                }
            }
        }
        if support.resolves() {
            self.generation += 1;
            self.items.clear();
        }
        let items = items
            .into_iter()
            .map(|mut item| {
                if item.tags.as_ref().is_some_and(Vec::is_empty) {
                    item.tags = None;
                }
                if item.data.is_none() {
                    let lazy = Lazy {
                        documentation: item
                            .documentation
                            .take_if(|_| support.resolve_documentation),
                        detail: item.detail.take_if(|_| support.resolve_detail),
                        command: item.command.take_if(|_| support.resolve_command),
                    };
                    if lazy.documentation.is_some()
                        || lazy.detail.is_some()
                        || lazy.command.is_some()
                    {
                        item.data = Some(json!([self.generation, self.items.len()]));
                        self.items.push(lazy);
                    }
                }
                let mut text_edit_text = None;
                if support.edit_range
                    && let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit
                    && Some(edit.range) == defaults.edit_range
                {
                    let Some(CompletionTextEdit::Edit(edit)) = item.text_edit.take() else {
                        unreachable!()
                    };
                    text_edit_text = (edit.new_text != item.label).then_some(edit.new_text);
                }
                CompactItem {
                    item,
                    text_edit_text,
                }
            })
            .collect();
        CompactList {
            is_incomplete,
            item_defaults: defaults,
            items,
        }
    }

    /// Adds the resolvable properties to an item of the last list.
    pub fn resolve(&self, item: &mut Value) {
        let Some([generation, index]) = item
            .get("data")
            .and_then(|d| serde_json::from_value::<[u64; 2]>(d.clone()).ok())
        else {
            return;
        };
        if generation != self.generation {
            return;
        }
        let lazy = usize::try_from(index).ok().and_then(|i| self.items.get(i));
        if let (Some(Value::Object(lazy)), Some(item)) =
            (lazy.map(|l| json!(l)), item.as_object_mut())
        {
            item.extend(lazy);
        }
    }
}

fn common_range(items: &[CompletionItem]) -> Option<Range> {
    let mut counts: Vec<(Range, usize)> = Vec::new();
    for item in items {
        if let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit {
            match counts.iter_mut().find(|(r, _)| *r == edit.range) {
                Some((_, n)) => *n += 1,
                None => counts.push((edit.range, 1)),
            }
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(r, _)| r)
}

#[cfg(test)]
mod tests {
    use lsp_types::{Position, TextEdit};

    use super::*;

    fn item(label: &str, new_text: &str, range: Range, snippet: bool) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            documentation: Some(Documentation::String(format!("doc {label}"))),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(
                range,
                new_text.into(),
            ))),
            insert_text_format: snippet.then_some(InsertTextFormat::SNIPPET),
            tags: Some(Vec::new()),
            ..Default::default()
        }
    }

    #[test]
    fn compact_and_resolve() {
        let a = Range::new(Position::new(0, 1), Position::new(0, 2));
        let b = Range::new(Position::new(0, 0), Position::new(0, 2));
        let mut color = item("color", "color: $0;", a, true);
        color.command = Some(Command::new("Suggest".into(), "retrigger".into(), None));
        let items = vec![color, item("top", "top", a, true), item("x", "y", b, false)];
        let support = CompletionSupport {
            edit_range: true,
            insert_text_format: true,
            resolve_documentation: true,
            resolve_detail: true,
            resolve_command: true,
        };
        let mut store = ResolveStore::default();
        let list = serde_json::to_value(store.compact(support, false, items, None)).unwrap();
        assert_eq!(
            list,
            json!({
                "isIncomplete": false,
                "itemDefaults": { "editRange": a, "insertTextFormat": 2 },
                "items": [
                    { "label": "color", "data": [1, 0], "textEditText": "color: $0;" },
                    { "label": "top", "data": [1, 1] },
                    {
                        "label": "x", "data": [1, 2], "insertTextFormat": 1,
                        "textEdit": { "range": b, "newText": "y" }
                    }
                ]
            })
        );
        let mut resolved = list["items"][2].clone();
        store.resolve(&mut resolved);
        assert_eq!(resolved["documentation"], "doc x");
        let mut resolved = list["items"][0].clone();
        store.resolve(&mut resolved);
        assert_eq!(resolved["command"]["command"], "retrigger");

        // A later list makes the old data stale.
        store.compact(support, false, Vec::new(), None);
        let mut stale = list["items"][2].clone();
        store.resolve(&mut stale);
        assert!(stale.get("documentation").is_none());
    }

    #[test]
    fn no_support() {
        let a = Range::new(Position::new(0, 1), Position::new(0, 2));
        let mut store = ResolveStore::default();
        let items = vec![item("top", "top", a, true)];
        let list = store.compact(CompletionSupport::default(), true, items, None);
        let list = serde_json::to_value(list).unwrap();
        assert!(list.get("itemDefaults").is_none());
        assert_eq!(list["items"][0]["documentation"], "doc top");
        assert_eq!(list["items"][0]["textEdit"]["newText"], "top");
    }
}
