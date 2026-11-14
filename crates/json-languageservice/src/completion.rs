//! Port of `services/jsonCompletion.ts`.

use std::collections::{HashMap, HashSet};

use jsonc::{Scanner, SyntaxKind};
use lsp_types::{
    ClientCapabilities, Command, CompletionItem, CompletionItemKind, CompletionItemLabelDetails,
    CompletionList, CompletionTextEdit, Documentation, InsertTextFormat, MarkupContent, MarkupKind,
    Position, Range, TextEdit,
};
use serde_json::Value;

use crate::schema::{Items, JsonSchema, SchemaId, SchemaRef, SchemaType};
use crate::schema_service::{JsonSchemaService, ResolvedSchema};
use crate::strings::{js_stringify, js_stringify_indent, js_trim, stringify_object, utf16_prefix};
use crate::{AstKind, JsonDocument, NodeId, TextDocument};

#[derive(Clone, Copy, Debug, Default)]
pub struct JsonCompletion {
    supports_markdown: bool,
    supports_label_details: bool,
}

impl JsonCompletion {
    pub fn new(capabilities: &ClientCapabilities) -> Self {
        let item = capabilities
            .text_document
            .as_ref()
            .and_then(|t| t.completion.as_ref())
            .and_then(|c| c.completion_item.as_ref());
        JsonCompletion {
            supports_markdown: item
                .and_then(|i| i.documentation_format.as_ref())
                .is_some_and(|f| f.contains(&MarkupKind::Markdown)),
            supports_label_details: item.and_then(|i| i.label_details_support) == Some(true),
        }
    }

    pub fn do_complete(
        &self,
        service: &mut JsonSchemaService,
        document: &TextDocument,
        position: Position,
        doc: &JsonDocument,
    ) -> CompletionList {
        let text = document.text();
        let offset = document.offset_at(position);
        let mut node = doc.get_node_from_offset(offset, true);
        if is_in_comment(text, node.map_or(0, |n| doc[n].offset()), offset) {
            return CompletionList::default();
        }
        if let Some(n) = node
            && offset == doc[n].end()
            && offset > 0
        {
            let ch = text.as_bytes()[offset - 1];
            if (doc[n].kind == AstKind::Object && ch == b'}')
                || (doc[n].kind == AstKind::Array && ch == b']')
            {
                node = doc[n].parent;
            }
        }

        let current_word = current_word(text, offset);
        let overwrite_range = match node.filter(|&n| is_scalar(doc, n)) {
            Some(n) => document.range(doc[n].offset(), doc[n].end()),
            None => {
                let mut start = offset - current_word.len();
                if start > 0 && text.as_bytes()[start - 1] == b'"' {
                    start -= 1;
                }
                Range::new(document.position_at(start), position)
            }
        };

        let schema = service.get_schema_for_resource(document.uri().as_str(), Some(doc));
        let mut cx = Context {
            settings: *self,
            service,
            schema: schema.as_deref(),
            text,
            doc,
            items: Vec::new(),
            proposed: HashMap::new(),
            overwrite_range,
            is_incomplete: false,
            exclude: None,
            stack: Vec::new(),
        };

        let mut add_value = true;
        let mut current_property = None;
        if let Some(n) = node
            && matches!(doc[n].kind, AstKind::String(_))
            && let Some(parent) = doc[n].parent
            && matches!(doc[parent].kind, AstKind::Property { .. })
            && doc.children(parent)[0] == n
        {
            add_value = doc.value(parent).is_none();
            current_property = Some(parent);
            node = doc[parent].parent;
        }

        if let Some(n) = node
            && doc[n].kind == AstKind::Object
        {
            if doc[n].offset() == offset {
                return cx.finish();
            }
            for &p in doc.children(n) {
                if current_property != Some(p) {
                    cx.proposed.insert(doc.key(p).to_owned(), None);
                }
            }
            let separator_after = if add_value {
                evaluate_separator_after(text, document.offset_at(overwrite_range.end))
            } else {
                ""
            };
            match cx.schema {
                Some(schema) => cx.property_completions(schema, n, add_value, separator_after),
                None => cx.schema_less_property_completions(n),
            }
            if cx.schema.is_none()
                && !current_word.is_empty()
                && offset
                    .checked_sub(current_word.len() + 1)
                    .map(|i| text.as_bytes()[i])
                    != Some(b'"')
            {
                let mut item = item(
                    CompletionItemKind::PROPERTY,
                    js_stringify(&Value::from(current_word)),
                    insert_text_for_property(current_word, None, false, separator_after),
                );
                item.documentation = empty_documentation();
                cx.add(item);
                cx.is_incomplete = true;
            }
        }

        let mut types = HashSet::new();
        match cx.schema {
            Some(schema) => cx.value_completions(schema, node, offset, &mut types),
            None => cx.schema_less_value_completions(node, offset),
        }
        if cx.items.is_empty() {
            let offset_for_separator = match node.filter(|&n| is_scalar(doc, n)) {
                Some(n) => doc[n].end(),
                None => offset,
            };
            let separator_after = evaluate_separator_after(text, offset_for_separator);
            cx.filler_value_completions(&types, separator_after);
        }
        cx.finish()
    }
}

struct Context<'a> {
    settings: JsonCompletion,
    service: &'a JsonSchemaService,
    schema: Option<&'a ResolvedSchema>,
    text: &'a str,
    doc: &'a JsonDocument,
    items: Vec<CompletionItem>,
    /// Label to index in `items`. `None` for the keys of existing properties.
    proposed: HashMap<String, Option<usize>>,
    overwrite_range: Range,
    is_incomplete: bool,
    /// Labels to drop, for `uniqueItems`.
    exclude: Option<HashSet<String>>,
    /// The schemas in `add_schema_value_completions` calls, to stop at cycles.
    stack: Vec<SchemaId>,
}

fn is_scalar(doc: &JsonDocument, node: NodeId) -> bool {
    matches!(
        doc[node].kind,
        AstKind::String(_) | AstKind::Number { .. } | AstKind::Boolean(_) | AstKind::Null
    )
}

fn item(kind: CompletionItemKind, label: String, insert_text: String) -> CompletionItem {
    CompletionItem {
        kind: Some(kind),
        label,
        insert_text: Some(insert_text),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..CompletionItem::default()
    }
}

fn empty_documentation() -> Option<Documentation> {
    Some(Documentation::String(String::new()))
}

fn is_falsy(documentation: Option<&Documentation>) -> bool {
    match documentation {
        None => true,
        Some(Documentation::String(s)) => s.is_empty(),
        Some(Documentation::MarkupContent(_)) => false,
    }
}

fn suggest_command() -> Command {
    Command::new(
        "Suggest".to_owned(),
        "editor.action.triggerSuggest".to_owned(),
        None,
    )
}

/// `getSuggestionKind`.
fn suggestion_kind(type_: Option<&str>) -> CompletionItemKind {
    match type_ {
        Some("object") => CompletionItemKind::MODULE,
        Some("property") => CompletionItemKind::PROPERTY,
        _ => CompletionItemKind::VALUE,
    }
}

fn schema_kind(type_: &Option<SchemaType>) -> CompletionItemKind {
    suggestion_kind(
        type_
            .as_ref()
            .and_then(|t| t.names().first())
            .map(String::as_str),
    )
}

fn node_kind(doc: &JsonDocument, node: NodeId) -> CompletionItemKind {
    suggestion_kind(Some(doc[node].node_type().as_str()))
}

fn truthy(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

/// Escapes `$`, `\` and `}`.
fn insert_text_for_plain_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '$' | '}') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn insert_text_for_value(value: &Value, separator_after: &str) -> String {
    let text = js_stringify_indent(value, "\t");
    match text.as_str() {
        "{}" => format!("{{$1}}{separator_after}"),
        "[]" => format!("[$1]{separator_after}"),
        _ => insert_text_for_plain_text(&(text + separator_after)),
    }
}

fn insert_text_for_snippet_value(value: &Value, separator_after: &str) -> String {
    let literal = |v: &Value| match v {
        Value::String(s) if s.starts_with('^') => s[1..].to_owned(),
        _ => js_stringify(v),
    };
    stringify_object(value, "", &literal) + separator_after
}

fn insert_text_for_guessed_value(value: &Value, separator_after: &str) -> String {
    match value {
        Value::Null => format!("${{1:null}}{separator_after}"),
        Value::Object(_) | Value::Array(_) => insert_text_for_value(value, separator_after),
        Value::String(_) => {
            let quoted = js_stringify(value);
            let inner = insert_text_for_plain_text(&quoted[1..quoted.len() - 1]);
            format!("\"${{1:{inner}}}\"{separator_after}")
        }
        Value::Number(_) | Value::Bool(_) => {
            format!("${{1:{}}}{separator_after}", js_stringify(value))
        }
    }
}

/// Replaces `${1:text}` with `text` and removes `$1`.
fn strip_placeholders(s: &str) -> String {
    let b = s.as_bytes();
    let digits = |i: usize| b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut out = String::with_capacity(s.len());
    let mut copied = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' {
            if b.get(i + 1) == Some(&b'{') {
                let d = digits(i + 2);
                let start = i + 2 + d + 1;
                if d > 0
                    && b.get(i + 2 + d) == Some(&b':')
                    && let Some(len) = s[start..].find('}').filter(|&len| len > 0)
                {
                    out.push_str(&s[copied..i]);
                    out.push_str(&s[start..start + len]);
                    i = start + len + 1;
                    copied = i;
                    continue;
                }
            }
            let d = digits(i + 1);
            if d > 0 {
                out.push_str(&s[copied..i]);
                i += 1 + d;
                copied = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&s[copied..]);
    out
}

fn has_items(schema: &JsonSchema) -> bool {
    !matches!(
        schema.items,
        None | Some(Items::One(SchemaRef::Bool(false)))
    )
}

fn insert_text_for_property(
    key: &str,
    property_schema: Option<&JsonSchema>,
    add_value: bool,
    separator_after: &str,
) -> String {
    let property_text = insert_text_for_value(&Value::from(key), "");
    if !add_value {
        return property_text;
    }
    let mut value = None;
    let mut proposals = 0;
    if let Some(s) = property_schema {
        if let Some(snippets) = &s.vscode().default_snippets {
            if let [snippet] = snippets.as_slice()
                && let Some(body) = &snippet.body
            {
                value = Some(insert_text_for_snippet_value(body, ""));
            }
            proposals += snippets.len();
        }
        if let Some(e) = &s.enum_ {
            if value.is_none() && e.len() == 1 {
                value = Some(insert_text_for_guessed_value(&e[0], ""));
            }
            proposals += e.len();
        }
        if let Some(c) = &s.const_ {
            if value.is_none() {
                value = Some(insert_text_for_guessed_value(c, ""));
            }
            proposals += 1;
        }
        if let Some(d) = &s.default {
            if value.is_none() {
                value = Some(insert_text_for_guessed_value(d, ""));
            }
            proposals += 1;
        }
        if let Some(examples) = s.examples.as_ref().filter(|e| !e.is_empty()) {
            if value.is_none() {
                value = Some(insert_text_for_guessed_value(&examples[0], ""));
            }
            proposals += examples.len();
        }
        if proposals == 0 {
            let type_ = s
                .type_
                .as_ref()
                .and_then(|t| t.names().first())
                .map(String::as_str)
                .filter(|t| !t.is_empty())
                .or_else(|| {
                    if s.properties.is_some() {
                        Some("object")
                    } else if has_items(s) {
                        Some("array")
                    } else {
                        None
                    }
                });
            value = Some(
                match type_ {
                    Some("boolean") => "$1",
                    Some("string") => "\"$1\"",
                    Some("object") => "{$1}",
                    Some("array") => "[$1]",
                    Some("number" | "integer") => "${1:0}",
                    Some("null") => "${1:null}",
                    _ if s.one_of.as_ref().is_some_and(|o| !o.is_empty()) => "$1",
                    _ => return property_text,
                }
                .to_owned(),
            );
        }
    }
    let value = match value {
        Some(v) if proposals <= 1 => v,
        _ => "$1".to_owned(),
    };
    format!("{property_text}: {value}{separator_after}")
}

fn current_word(text: &str, offset: usize) -> &str {
    let bytes = text.as_bytes();
    let mut i = offset;
    while i > 0 && !b" \t\n\r\x0b\":{[,]}".contains(&bytes[i - 1]) {
        i -= 1;
    }
    &text[i..offset]
}

fn evaluate_separator_after(text: &str, offset: usize) -> &'static str {
    let mut scanner = Scanner::new(text, true);
    scanner.set_position(offset);
    match scanner.scan() {
        SyntaxKind::CommaToken
        | SyntaxKind::CloseBraceToken
        | SyntaxKind::CloseBracketToken
        | SyntaxKind::EOF => "",
        _ => ",",
    }
}

fn is_in_comment(text: &str, start: usize, offset: usize) -> bool {
    let mut scanner = Scanner::new(text, false);
    scanner.set_position(start);
    let mut token = scanner.scan();
    while token != SyntaxKind::EOF && scanner.token_offset() + scanner.token_length() < offset {
        token = scanner.scan();
    }
    matches!(
        token,
        SyntaxKind::LineCommentTrivia | SyntaxKind::BlockCommentTrivia
    ) && scanner.token_offset() <= offset
}

impl<'a> Context<'a> {
    fn finish(self) -> CompletionList {
        CompletionList {
            is_incomplete: self.is_incomplete,
            items: self.items,
        }
    }

    fn add(&mut self, mut item: CompletionItem) {
        if self
            .exclude
            .as_ref()
            .is_some_and(|e| e.contains(&item.label))
        {
            return;
        }
        match self.proposed.get(&item.label) {
            None => {
                let mut label = item.label.replace('\n', "↵");
                if label.encode_utf16().count() > 60 {
                    let shortened = format!("{}...", js_trim(utf16_prefix(&label, 57)));
                    if !self.proposed.contains_key(&shortened) {
                        label = shortened;
                    }
                }
                item.text_edit = Some(CompletionTextEdit::Edit(TextEdit::new(
                    self.overwrite_range,
                    item.insert_text.clone().unwrap_or_default(),
                )));
                item.label = label.clone();
                self.proposed.insert(label, Some(self.items.len()));
                self.items.push(item);
            }
            Some(Some(i)) => {
                let existing = &mut self.items[*i];
                if is_falsy(existing.documentation.as_ref()) {
                    existing.documentation = item.documentation;
                }
                if existing.detail.as_deref().is_none_or(str::is_empty) {
                    existing.detail = item.detail;
                }
                if existing.label_details.is_none() {
                    existing.label_details = item.label_details;
                }
            }
            Some(None) => {}
        }
    }

    fn markup(&self, markup: Option<&str>) -> Option<Documentation> {
        markup
            .filter(|m| !m.is_empty() && self.settings.supports_markdown)
            .map(|m| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: m.to_owned(),
                })
            })
    }

    /// `fromMarkup(markdownDescription) || description`.
    fn description(&self, schema: &JsonSchema) -> Option<Documentation> {
        self.markup(schema.markdown_description.as_deref())
            .or_else(|| schema.description.clone().map(Documentation::String))
    }

    /// `fromMarkup(markdownDescription) || description || ''`.
    fn description_or_empty(&self, schema: &JsonSchema) -> Documentation {
        self.markup(schema.markdown_description.as_deref())
            .unwrap_or_else(|| {
                Documentation::String(schema.description.clone().unwrap_or_default())
            })
    }

    fn property_completions(
        &mut self,
        schema: &'a ResolvedSchema,
        node: NodeId,
        add_value: bool,
        separator_after: &str,
    ) {
        let schemas = &schema.schemas;
        let matching = self.doc.get_matching_schemas(
            schemas,
            schema.root,
            Some(self.doc[node].offset()),
            None,
            schema.active_vocabularies.as_ref(),
        );
        let suggest_suffix = format!("$1{separator_after}");
        for s in matching {
            if s.node != node || s.inverted {
                continue;
            }
            let s = &schemas[s.schema];
            if let Some(properties) = &s.properties {
                for (key, r) in properties.iter() {
                    let SchemaRef::Schema(p) = *r else {
                        continue;
                    };
                    let p = &schemas[p];
                    if p.vscode().deprecation_message.is_some() || p.do_not_suggest {
                        continue;
                    }
                    let insert_text =
                        insert_text_for_property(key, Some(p), add_value, separator_after);
                    let mut item = item(CompletionItemKind::PROPERTY, key.clone(), insert_text);
                    item.filter_text = Some(js_stringify(&Value::from(key.as_str())));
                    item.documentation = Some(self.description_or_empty(p));
                    item.detail = p.vscode().completion_detail.clone();
                    item.sort_text = p.vscode().suggest_sort_text.clone();
                    if item
                        .insert_text
                        .as_ref()
                        .is_some_and(|t| t.ends_with(&suggest_suffix))
                    {
                        item.command = Some(suggest_command());
                    }
                    self.add(item);
                }
            }
            let Some(SchemaRef::Schema(names)) = s.property_names else {
                continue;
            };
            let names = &schemas[names];
            if names.vscode().deprecation_message.is_some() || names.do_not_suggest {
                continue;
            }
            let add_name = |this: &mut Self,
                            name: &Value,
                            documentation: Option<Documentation>,
                            detail: Option<String>,
                            sort_text: Option<String>| {
                // Upstream throws for a name that is not a string.
                let Value::String(name) = name else {
                    return;
                };
                let insert_text = insert_text_for_property(name, None, add_value, separator_after);
                let mut item = item(CompletionItemKind::PROPERTY, name.clone(), insert_text);
                item.filter_text = Some(js_stringify(&Value::from(name.as_str())));
                item.documentation = Some(match documentation {
                    Some(d) if !is_falsy(Some(&d)) => d,
                    _ => this.description_or_empty(names),
                });
                item.sort_text = sort_text;
                item.detail = detail;
                if item
                    .insert_text
                    .as_ref()
                    .is_some_and(|t| t.ends_with(&suggest_suffix))
                {
                    item.command = Some(suggest_command());
                }
                this.add(item);
            };
            if let Some(e) = &names.enum_ {
                for (i, name) in e.iter().enumerate() {
                    let documentation = if let Some(m) = names
                        .vscode()
                        .markdown_enum_descriptions
                        .as_ref()
                        .filter(|m| i < m.len())
                    {
                        self.markup(m[i].as_deref())
                    } else if let Some(d) = names
                        .vscode()
                        .enum_descriptions
                        .as_ref()
                        .filter(|d| i < d.len())
                    {
                        d[i].clone().map(Documentation::String)
                    } else {
                        None
                    };
                    let sort_text = names
                        .vscode()
                        .enum_sort_texts
                        .as_ref()
                        .and_then(|t| t.get(i).cloned().flatten());
                    let detail = names
                        .vscode()
                        .enum_details
                        .as_ref()
                        .and_then(|t| t.get(i).cloned().flatten());
                    add_name(self, name, documentation, detail, sort_text);
                }
            }
            if let Some(examples) = &names.examples {
                for name in examples {
                    add_name(self, name, None, None, None);
                }
            }
            if let Some(c) = names.const_.as_ref().filter(|c| crate::is_truthy(c)) {
                add_name(
                    self,
                    c,
                    None,
                    names.vscode().completion_detail.clone(),
                    names.vscode().suggest_sort_text.clone(),
                );
            }
        }
    }

    fn schema_less_property_completions(&mut self, node: NodeId) {
        let doc = self.doc;
        let similar = |this: &mut Self, object: NodeId| {
            for &p in doc.children(object) {
                let key = doc.key(p);
                let mut item = item(
                    CompletionItemKind::PROPERTY,
                    key.to_owned(),
                    insert_text_for_value(&Value::from(key), ""),
                );
                item.filter_text = Some(js_stringify(&Value::from(key)));
                item.documentation = empty_documentation();
                this.add(item);
            }
        };
        match doc[node].parent {
            Some(parent) => match doc[parent].kind {
                AstKind::Property { .. } => {
                    let parent_key = doc.key(parent);
                    doc.visit(|n| {
                        if n != parent
                            && matches!(doc[n].kind, AstKind::Property { .. })
                            && doc.key(n) == parent_key
                            && let Some(v) = doc.value(n)
                            && doc[v].kind == AstKind::Object
                        {
                            similar(self, v);
                        }
                        true
                    });
                }
                AstKind::Array => {
                    for &item in doc.children(parent) {
                        if item != node && doc[item].kind == AstKind::Object {
                            similar(self, item);
                        }
                    }
                }
                _ => {}
            },
            None if doc[node].kind == AstKind::Object => {
                let mut item = item(
                    CompletionItemKind::PROPERTY,
                    "$schema".to_owned(),
                    insert_text_for_property("$schema", None, true, ""),
                );
                item.documentation = empty_documentation();
                item.filter_text = Some(js_stringify(&Value::from("$schema")));
                self.add(item);
            }
            None => {}
        }
    }

    fn schema_less_value_completions(&mut self, node: Option<NodeId>, offset: usize) {
        let doc = self.doc;
        let mut node = node;
        let mut offset_for_separator = offset;
        if let Some(n) = node.filter(|&n| is_scalar(doc, n)) {
            offset_for_separator = doc[n].end();
            node = doc[n].parent;
        }
        let Some(node) = node else {
            for (label, value) in [
                ("Empty object", Value::Object(Default::default())),
                ("Empty array", Value::Array(Vec::new())),
            ] {
                let kind =
                    suggestion_kind(Some(if value.is_object() { "object" } else { "array" }));
                let mut item = item(kind, label.to_owned(), insert_text_for_value(&value, ""));
                item.documentation = empty_documentation();
                self.add(item);
            }
            return;
        };
        let separator_after = evaluate_separator_after(self.text, offset_for_separator);
        let text = self.text;
        let collect = |this: &mut Self, value: NodeId| {
            if let Some(p) = doc[value].parent
                && !doc[p].contains(offset, true)
            {
                let (label, insert_text) = match doc[value].kind {
                    AstKind::Array => (
                        "[]".to_owned(),
                        insert_text_for_value(&Value::Array(Vec::new()), separator_after),
                    ),
                    AstKind::Object => (
                        "{}".to_owned(),
                        insert_text_for_value(&Value::Object(Default::default()), separator_after),
                    ),
                    _ => {
                        let content = &text[doc[value].offset()..doc[value].end()];
                        (
                            content.to_owned(),
                            insert_text_for_plain_text(&format!("{content}{separator_after}")),
                        )
                    }
                };
                let mut item = item(node_kind(doc, value), label, insert_text);
                item.documentation = empty_documentation();
                this.add(item);
            }
            if let AstKind::Boolean(b) = doc[value].kind {
                this.boolean_value_completion(!b, separator_after);
            }
        };

        match doc[node].kind {
            AstKind::Property { colon_offset }
                if colon_offset.is_none_or(|c| offset > c as usize) =>
            {
                if let Some(v) = doc.value(node)
                    && (offset > doc[v].end()
                        || matches!(doc[v].kind, AstKind::Object | AstKind::Array))
                {
                    return;
                }
                let parent_key = doc.key(node);
                doc.visit(|n| {
                    if matches!(doc[n].kind, AstKind::Property { .. })
                        && doc.key(n) == parent_key
                        && let Some(v) = doc.value(n)
                    {
                        collect(self, v);
                    }
                    true
                });
                if parent_key == "$schema"
                    && let Some(p) = doc[node].parent
                    && doc[p].parent.is_none()
                {
                    self.dollar_schema_completions(separator_after);
                }
            }
            AstKind::Array => match doc[node].parent {
                Some(p) if matches!(doc[p].kind, AstKind::Property { .. }) => {
                    let parent_key = doc.key(p);
                    doc.visit(|n| {
                        if matches!(doc[n].kind, AstKind::Property { .. })
                            && doc.key(n) == parent_key
                            && let Some(v) = doc.value(n)
                            && doc[v].kind == AstKind::Array
                        {
                            for &item in doc.children(v) {
                                collect(self, item);
                            }
                        }
                        true
                    });
                }
                _ => {
                    for &item in doc.children(node) {
                        collect(self, item);
                    }
                }
            },
            _ => {}
        }
    }

    fn value_completions(
        &mut self,
        schema: &'a ResolvedSchema,
        node: Option<NodeId>,
        offset: usize,
        types: &mut HashSet<String>,
    ) {
        let doc = self.doc;
        let mut node = node;
        let mut offset_for_separator = offset;
        let mut value_node = None;
        let mut parent_key = None;
        if let Some(n) = node.filter(|&n| is_scalar(doc, n)) {
            offset_for_separator = doc[n].end();
            value_node = Some(n);
            node = doc[n].parent;
        }
        let Some(mut n) = node else {
            self.add_schema_value_completions(SchemaRef::Schema(schema.root), "", types);
            return;
        };
        if let AstKind::Property { colon_offset } = doc[n].kind
            && colon_offset.is_none_or(|c| offset > c as usize)
        {
            if let Some(v) = doc.value(n)
                && offset > doc[v].end()
            {
                return;
            }
            parent_key = Some(doc.key(n));
            match doc[n].parent {
                Some(p) => n = p,
                None => return,
            }
        }
        if parent_key.is_none() && doc[n].kind != AstKind::Array {
            return;
        }
        let separator_after = evaluate_separator_after(self.text, offset_for_separator);
        let schemas = &schema.schemas;
        let matching = doc.get_matching_schemas(
            schemas,
            schema.root,
            Some(doc[n].offset()),
            value_node,
            schema.active_vocabularies.as_ref(),
        );
        for s in matching {
            if s.node != n || s.inverted {
                continue;
            }
            let s = &schemas[s.schema];
            if doc[n].kind == AstKind::Array
                && let Some(items) = &s.items
            {
                if s.unique_items {
                    let existing = doc
                        .children(n)
                        .iter()
                        .filter(|&&c| !matches!(doc[c].kind, AstKind::Array | AstKind::Object))
                        .map(|&c| js_stringify(&doc.node_value(c)))
                        .collect();
                    self.exclude = Some(existing);
                }
                match items {
                    Items::Many(list) => {
                        let index = self.find_item_at_offset(n, offset);
                        if let Some(&r) = list.get(index) {
                            self.add_schema_value_completions(r, separator_after, types);
                        }
                    }
                    Items::One(r) => self.add_schema_value_completions(*r, separator_after, types),
                }
                self.exclude = None;
            }
            if let Some(key) = parent_key {
                let mut matched = false;
                if let Some(r) = s.properties.as_ref().and_then(|p| p.get(key))
                    && *r != SchemaRef::Bool(false)
                {
                    matched = true;
                    self.add_schema_value_completions(*r, separator_after, types);
                }
                if !matched && let Some(patterns) = &s.pattern_properties {
                    for p in patterns.iter() {
                        if p.regex.as_ref().is_some_and(|r| r.find(key).is_some()) {
                            matched = true;
                            self.add_schema_value_completions(p.schema, separator_after, types);
                        }
                    }
                }
                if !matched && let Some(r) = s.additional_properties {
                    self.add_schema_value_completions(r, separator_after, types);
                }
            }
        }
        if parent_key == Some("$schema") && doc[n].parent.is_none() {
            self.dollar_schema_completions(separator_after);
        }
        if types.contains("boolean") {
            self.boolean_value_completion(true, separator_after);
            self.boolean_value_completion(false, separator_after);
        }
        if types.contains("null") {
            let mut item = item(
                suggestion_kind(Some("null")),
                "null".to_owned(),
                format!("null{separator_after}"),
            );
            item.documentation = empty_documentation();
            self.add(item);
        }
    }

    fn find_item_at_offset(&self, node: NodeId, offset: usize) -> usize {
        let mut scanner = Scanner::new(self.text, true);
        for (i, &child) in self.doc.children(node).iter().enumerate().rev() {
            let child = &self.doc[child];
            if offset > child.end() {
                scanner.set_position(child.end());
                let token = scanner.scan();
                if token == SyntaxKind::CommaToken
                    && offset >= scanner.token_offset() + scanner.token_length()
                {
                    return i + 1;
                }
                return i;
            } else if offset >= child.offset() {
                return i;
            }
        }
        0
    }

    fn add_schema_value_completions(
        &mut self,
        schema: SchemaRef,
        separator_after: &str,
        types: &mut HashSet<String>,
    ) {
        let (SchemaRef::Schema(id), Some(resolved)) = (schema, self.schema) else {
            return;
        };
        if self.stack.contains(&id) {
            return;
        }
        let s = &resolved.schemas[id];
        self.enum_value_completions(s, separator_after);
        self.default_value_completions(s, separator_after, 0);
        if s.enum_.is_none() && s.const_.is_none() {
            types.extend(s.type_.iter().flat_map(|t| t.names()).cloned());
        }
        self.stack.push(id);
        for list in [&s.all_of, &s.any_of, &s.one_of].into_iter().flatten() {
            for &r in list {
                self.add_schema_value_completions(r, separator_after, types);
            }
        }
        self.stack.pop();
    }

    fn default_value_completions(
        &mut self,
        s: &JsonSchema,
        separator_after: &str,
        array_depth: usize,
    ) {
        let wrap = |v: &Value| (0..array_depth).fold(v.clone(), |v, _| Value::Array(vec![v]));
        let kind = if array_depth > 0 {
            suggestion_kind(Some("array"))
        } else {
            schema_kind(&s.type_)
        };
        let mut has_proposals = false;
        if let Some(default) = &s.default {
            let value = wrap(default);
            let mut item = item(
                kind,
                js_stringify(&value),
                insert_text_for_value(&value, separator_after),
            );
            if self.settings.supports_label_details {
                item.label_details = Some(CompletionItemLabelDetails {
                    detail: None,
                    description: Some("Default value".to_owned()),
                });
            } else {
                item.detail = Some("Default value".to_owned());
            }
            self.add(item);
            has_proposals = true;
        }
        if let Some(examples) = &s.examples {
            for example in examples {
                let value = wrap(example);
                self.add(item(
                    kind,
                    js_stringify(&value),
                    insert_text_for_value(&value, separator_after),
                ));
                has_proposals = true;
            }
        }
        if let Some(snippets) = &s.vscode().default_snippets {
            for snippet in snippets {
                let label = truthy(&snippet.label);
                let (kind, label, insert_text, filter_text) = if let Some(body) = &snippet.body {
                    // Upstream shadows `type` here, so the kind ignores the array depth.
                    let value = wrap(body);
                    let insert_text = insert_text_for_snippet_value(&value, separator_after);
                    let stringified = strip_placeholders(&js_stringify(&value));
                    let label = label.map_or_else(|| stringified.clone(), str::to_owned);
                    (schema_kind(&s.type_), label, insert_text, stringified)
                } else if let Some(body_text) = &snippet.body_text {
                    let (mut prefix, mut suffix, mut indent) =
                        (String::new(), String::new(), String::new());
                    for _ in 0..array_depth {
                        prefix = format!("{prefix}{indent}[\n");
                        suffix = format!("{suffix}\n{indent}]");
                        indent.push('\t');
                    }
                    let insert_text = format!(
                        "{prefix}{indent}{}{suffix}{separator_after}",
                        body_text
                            .split('\n')
                            .collect::<Vec<_>>()
                            .join(&format!("\n{indent}"))
                    );
                    let label = label.map_or_else(|| insert_text.clone(), str::to_owned);
                    let filter_text = insert_text.replace('\n', "");
                    (kind, label, insert_text, filter_text)
                } else {
                    continue;
                };
                let mut item = item(kind, label, insert_text);
                item.documentation = self
                    .markup(snippet.markdown_description.as_deref())
                    .or_else(|| snippet.description.clone().map(Documentation::String));
                item.filter_text = Some(filter_text);
                self.add(item);
                has_proposals = true;
            }
        }
        if !has_proposals
            && array_depth < 5
            && let Some(Items::One(SchemaRef::Schema(items))) = s.items
            && let Some(resolved) = self.schema
        {
            self.default_value_completions(
                &resolved.schemas[items],
                separator_after,
                array_depth + 1,
            );
        }
    }

    fn enum_value_completions(&mut self, s: &JsonSchema, separator_after: &str) {
        if let Some(c) = &s.const_ {
            let mut item = item(
                schema_kind(&s.type_),
                js_stringify(c),
                insert_text_for_value(c, separator_after),
            );
            item.documentation = self.description(s);
            self.add(item);
        }
        let Some(e) = &s.enum_ else {
            return;
        };
        for (i, value) in e.iter().enumerate() {
            let mut documentation = self.description(s);
            if let Some(m) = s
                .vscode()
                .markdown_enum_descriptions
                .as_ref()
                .filter(|m| i < m.len() && self.settings.supports_markdown)
            {
                documentation = self.markup(m[i].as_deref());
            } else if let Some(d) = s
                .vscode()
                .enum_descriptions
                .as_ref()
                .filter(|d| i < d.len())
            {
                documentation = d[i].clone().map(Documentation::String);
            }
            let mut item = item(
                schema_kind(&s.type_),
                js_stringify(value),
                insert_text_for_value(value, separator_after),
            );
            item.sort_text = s
                .vscode()
                .enum_sort_texts
                .as_ref()
                .and_then(|t| t.get(i).cloned().flatten());
            item.detail = s
                .vscode()
                .enum_details
                .as_ref()
                .and_then(|t| t.get(i).cloned().flatten());
            item.documentation = documentation;
            self.add(item);
        }
    }

    fn filler_value_completions(&mut self, types: &HashSet<String>, separator_after: &str) {
        if types.contains("object") {
            let mut item = item(
                suggestion_kind(Some("object")),
                "{}".to_owned(),
                insert_text_for_guessed_value(&Value::Object(Default::default()), separator_after),
            );
            item.detail = Some("New object".to_owned());
            item.documentation = empty_documentation();
            self.add(item);
        }
        if types.contains("array") {
            let mut item = item(
                suggestion_kind(Some("array")),
                "[]".to_owned(),
                insert_text_for_guessed_value(&Value::Array(Vec::new()), separator_after),
            );
            item.detail = Some("New array".to_owned());
            item.documentation = empty_documentation();
            self.add(item);
        }
    }

    fn boolean_value_completion(&mut self, value: bool, separator_after: &str) {
        let mut item = item(
            suggestion_kind(Some("boolean")),
            value.to_string(),
            insert_text_for_value(&Value::Bool(value), separator_after),
        );
        item.documentation = empty_documentation();
        self.add(item);
    }

    fn dollar_schema_completions(&mut self, separator_after: &str) {
        let ids = self
            .service
            .get_registered_schema_ids(Some(&|scheme: &str| scheme == "http" || scheme == "https"));
        for mut id in ids {
            if id.starts_with("https://json-schema.org/draft-") {
                id.push('#');
            }
            let value = Value::from(id);
            let mut item = item(
                CompletionItemKind::MODULE,
                js_stringify(&value),
                insert_text_for_value(&value, separator_after),
            );
            item.filter_text = Some(js_stringify(&value));
            item.documentation = empty_documentation();
            self.add(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::strip_placeholders;

    #[test]
    fn placeholders() {
        assert_eq!(strip_placeholders("a${1:b}c$2d"), "abcd");
        assert_eq!(strip_placeholders("${1:}$x${a:b}$"), "${1:}$x${a:b}$");
        assert_eq!(strip_placeholders("${12:x}y}"), "xy}");
        assert_eq!(strip_placeholders("${1:x"), "${1:x");
    }
}
