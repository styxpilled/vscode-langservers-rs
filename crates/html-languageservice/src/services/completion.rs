//! Port of `htmlCompletion.ts`.

use std::collections::HashMap;

use css_languageservice::services::links::{DocumentContext, FileSystemProvider};
use lsp_types::{
    Command, CompletionItem, CompletionItemKind, CompletionTextEdit, Documentation,
    InsertTextFormat, MarkupContent, MarkupKind, Position, Range, TextEdit,
};
use serde::{Deserialize, Serialize};
use text_document::TextDocument;

use super::path_completion::PathCompletionParticipant;
use crate::LanguageService;
use crate::language_facts::{HoverSettings, HtmlDataManager, generate_documentation};
use crate::parser::entities::ENTITIES;
use crate::parser::scanner::{Scanner, ScannerState, TokenType};
use crate::parser::{HtmlDocument, NodeId};
use crate::utils::{char_at, char_before, is_letter_or_digit, is_whitespace_or_empty, substring};

/// `CompletionList`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionList {
    pub is_incomplete: bool,
    pub items: Vec<CompletionItem>,
}

/// `attributeDefaultValue`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttributeDefaultValue {
    Empty,
    Singlequotes,
    #[default]
    Doublequotes,
}

/// `CompletionConfiguration`. `providers` holds the other keys: a data
/// provider whose id maps to `false` is off.
#[derive(Clone, Debug, Default)]
pub struct CompletionConfiguration {
    pub hide_auto_complete_proposals: bool,
    pub hide_end_tag_suggestions: bool,
    pub attribute_default_value: Option<AttributeDefaultValue>,
    pub providers: HashMap<String, bool>,
}

impl CompletionConfiguration {
    /// From the JSON object, with JS truthiness for the flags. Keys with a
    /// boolean value that are not flags are data provider ids.
    pub fn from_json(value: &serde_json::Value) -> Self {
        let truthy = |k: &str| match value.get(k) {
            None | Some(serde_json::Value::Null) => false,
            Some(serde_json::Value::Bool(b)) => *b,
            Some(serde_json::Value::Number(n)) => {
                n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan())
            }
            Some(serde_json::Value::String(s)) => !s.is_empty(),
            Some(_) => true,
        };
        let mut providers = HashMap::new();
        for (k, v) in value.as_object().into_iter().flatten() {
            if let Some(b) = v.as_bool() {
                providers.insert(k.clone(), b);
            }
        }
        CompletionConfiguration {
            hide_auto_complete_proposals: truthy("hideAutoCompleteProposals"),
            hide_end_tag_suggestions: truthy("hideEndTagSuggestions"),
            attribute_default_value: value
                .get("attributeDefaultValue")
                .and_then(|v| AttributeDefaultValue::deserialize(v).ok()),
            providers,
        }
    }
}

/// `HtmlAttributeValueContext`.
#[derive(Clone, Debug)]
pub struct HtmlAttributeValueContext {
    pub position: Position,
    pub tag: String,
    pub attribute: String,
    pub value: String,
    pub range: Range,
    pub attributes: Option<Vec<(String, Option<String>)>>,
}

/// `ICompletionParticipant`.
pub trait CompletionParticipant {
    fn on_html_attribute_value(&mut self, _context: &HtmlAttributeValueContext) {}
    fn on_html_content(&mut self, _position: Position) {}
}

/// The path participant, then the other participants.
struct Chain<'a, 'b>(
    &'a mut PathCompletionParticipant,
    &'a mut [&'b mut dyn CompletionParticipant],
);

impl CompletionParticipant for Chain<'_, '_> {
    fn on_html_attribute_value(&mut self, context: &HtmlAttributeValueContext) {
        self.0.on_html_attribute_value(context);
        for p in self.1.iter_mut() {
            p.on_html_attribute_value(context);
        }
    }

    fn on_html_content(&mut self, position: Position) {
        self.0.on_html_content(position);
        for p in self.1.iter_mut() {
            p.on_html_content(position);
        }
    }
}

fn replace(range: Range, text: impl Into<String>) -> Option<CompletionTextEdit> {
    Some(CompletionTextEdit::Edit(TextEdit::new(range, text.into())))
}

fn markup(m: Option<MarkupContent>) -> Option<Documentation> {
    m.map(Documentation::MarkupContent)
}

impl LanguageService {
    /// `doComplete2`: `doComplete` with path completion. The file system is
    /// synchronous. `None` where upstream rejects.
    #[allow(clippy::too_many_arguments)]
    pub fn do_complete2(
        &self,
        document: &TextDocument,
        position: Position,
        html_document: &HtmlDocument,
        document_context: &dyn DocumentContext,
        file_system: Option<&dyn FileSystemProvider>,
        settings: Option<&CompletionConfiguration>,
        participants: &mut [&mut dyn CompletionParticipant],
    ) -> Option<CompletionList> {
        let Some(file_system) = file_system else {
            return Some(self.do_complete(
                document,
                position,
                html_document,
                settings,
                participants,
            ));
        };
        let mut path = PathCompletionParticipant::default();
        let result = self.do_complete(
            document,
            position,
            html_document,
            settings,
            &mut [&mut Chain(&mut path, participants)],
        );
        let path_result = path.compute_completions(document, document_context, file_system)?;
        let mut items = path_result.items;
        items.extend(result.items);
        Some(CompletionList {
            is_incomplete: result.is_incomplete || path_result.is_incomplete,
            items,
        })
    }

    /// `doComplete`.
    pub fn do_complete(
        &self,
        document: &TextDocument,
        position: Position,
        html_document: &HtmlDocument,
        settings: Option<&CompletionConfiguration>,
        participants: &mut [&mut dyn CompletionParticipant],
    ) -> CompletionList {
        let mut c = Completion {
            ls: self,
            document,
            text: document.text(),
            position,
            offset: document.offset_at(position),
            html_document,
            settings,
            participants,
            supports_markdown: self.completion_markdown,
            node: 0,
            result: CompletionList::default(),
        };
        c.run();
        let mut list = c.result;
        if !self.completion_markdown {
            for item in &mut list.items {
                if let Some(Documentation::MarkupContent(m)) = &mut item.documentation {
                    m.kind = MarkupKind::PlainText;
                }
            }
        }
        list
    }

    /// `doQuoteComplete`.
    pub fn do_quote_complete(
        &self,
        document: &TextDocument,
        position: Position,
        html_document: &HtmlDocument,
        settings: Option<&CompletionConfiguration>,
    ) -> Option<String> {
        let offset = document.offset_at(position);
        if offset == 0 {
            return None;
        }
        let default_value = settings
            .and_then(|s| s.attribute_default_value)
            .unwrap_or_default();
        if default_value == AttributeDefaultValue::Empty {
            return None;
        }
        let text = document.text();
        if char_before(text, offset) != Some('=') {
            return None;
        }
        let value = if default_value == AttributeDefaultValue::Doublequotes {
            "\"$1\""
        } else {
            "'$1'"
        };
        let node = &html_document[html_document.find_node_before(offset)];
        if node.attributes.is_some()
            && node.start < offset
            && node.end_tag_start_truthy().is_none_or(|e| e > offset)
        {
            let mut scanner = Scanner::at(text, node.start);
            let mut token = scanner.scan();
            while token != TokenType::Eos && scanner.token_end() <= offset {
                if token == TokenType::AttributeName && scanner.token_end() == offset - 1 {
                    token = scanner.scan();
                    if token != TokenType::DelimiterAssign {
                        return None;
                    }
                    token = scanner.scan();
                    if matches!(token, TokenType::Unknown | TokenType::AttributeValue) {
                        return None;
                    }
                    return Some(value.to_owned());
                }
                token = scanner.scan();
            }
        }
        None
    }

    /// `doTagComplete`.
    pub fn do_tag_complete(
        &self,
        document: &TextDocument,
        position: Position,
        html_document: &HtmlDocument,
    ) -> Option<String> {
        let offset = document.offset_at(position);
        if offset == 0 {
            return None;
        }
        let text = document.text();
        match char_before(text, offset) {
            Some('>') => {
                let node = &html_document[html_document.find_node_before(offset)];
                let tag = node.tag.as_deref()?;
                if node.start < offset
                    && node.end_tag_start_truthy().is_none_or(|e| e > offset)
                    && !HtmlDataManager::is_void_element(tag, self.data_manager.void_elements())
                {
                    let mut scanner = Scanner::at(text, node.start);
                    let mut token = scanner.scan();
                    while token != TokenType::Eos && scanner.token_end() <= offset {
                        if token == TokenType::StartTagClose && scanner.token_end() == offset {
                            return Some(format!("$0</{tag}>"));
                        }
                        token = scanner.scan();
                    }
                }
            }
            Some('/') => {
                let mut id = Some(html_document.find_node_before(offset));
                while let Some(n) = id {
                    let node = &html_document[n];
                    if !(node.closed && node.end_tag_start_truthy().is_none_or(|e| e <= offset)) {
                        break;
                    }
                    id = node.parent;
                }
                let node = &html_document[id?];
                let tag = node.tag.as_deref()?;
                let mut scanner = Scanner::at(text, node.start);
                let mut token = scanner.scan();
                while token != TokenType::Eos && scanner.token_end() <= offset {
                    if token == TokenType::EndTagOpen && scanner.token_end() == offset {
                        return Some(if char_at(text, offset) != Some('>') {
                            format!("{tag}>")
                        } else {
                            tag.to_owned()
                        });
                    }
                    token = scanner.scan();
                }
            }
            _ => {}
        }
        None
    }
}

struct Completion<'a, 'p> {
    ls: &'a LanguageService,
    document: &'a TextDocument,
    text: &'a str,
    position: Position,
    offset: usize,
    html_document: &'a HtmlDocument,
    settings: Option<&'a CompletionConfiguration>,
    participants: &'a mut [&'p mut dyn CompletionParticipant],
    supports_markdown: bool,
    node: NodeId,
    result: CompletionList,
}

impl Completion<'_, '_> {
    fn providers(&self) -> impl Iterator<Item = &crate::HtmlDataProvider> {
        let settings = self.settings;
        self.ls
            .data_manager
            .data_providers()
            .iter()
            .filter(move |p| settings.is_none_or(|s| s.providers.get(p.id()) != Some(&false)))
            .map(|p| &**p)
    }

    fn replace_range(&self, mut start: usize, end: usize) -> Range {
        if start > self.offset {
            start = self.offset;
        }
        Range::new(
            self.document.position_at(start),
            self.document.position_at(end),
        )
    }

    fn push(&mut self, item: CompletionItem) {
        self.result.items.push(item);
    }

    fn collect_open_tag_suggestions(&mut self, after_open_bracket: usize, tag_name_end: usize) {
        let range = self.replace_range(after_open_bracket, tag_name_end);
        let mut items = Vec::new();
        for provider in self.providers() {
            for tag in provider.provide_tags() {
                items.push(CompletionItem {
                    label: tag.name.clone(),
                    kind: Some(CompletionItemKind::PROPERTY),
                    documentation: markup(generate_documentation(
                        tag,
                        HoverSettings::default(),
                        self.supports_markdown,
                    )),
                    text_edit: replace(range, tag.name.clone()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                });
            }
        }
        self.result.items.extend(items);
    }

    /// `getLineIndent`.
    fn line_indent(&self, offset: usize) -> Option<&str> {
        let mut start = offset;
        while let Some(ch) = char_before(self.text, start) {
            if ch == '\n' || ch == '\r' {
                return Some(&self.text[start..offset]);
            }
            if !is_whitespace_or_empty(Some(ch)) {
                return None;
            }
            start -= ch.len_utf8();
        }
        Some(&self.text[..offset])
    }

    fn collect_close_tag_suggestions(
        &mut self,
        after_open_bracket: usize,
        in_open_tag: bool,
        tag_name_end: usize,
    ) {
        if self.settings.is_some_and(|s| s.hide_end_tag_suggestions) {
            return;
        }
        let range = self.replace_range(after_open_bracket, tag_name_end);
        let close_tag = if is_followed_by(
            self.text,
            tag_name_end,
            ScannerState::WithinEndTag,
            TokenType::EndTagClose,
        ) {
            ""
        } else {
            ">"
        };
        let doc = self.html_document;
        let mut curr = if in_open_tag {
            doc[self.node].parent
        } else {
            Some(self.node)
        };
        while let Some(id) = curr {
            let node = &doc[id];
            if let Some(tag) = &node.tag
                && (!node.closed || node.end_tag_start_truthy().is_some_and(|e| e > self.offset))
            {
                let mut item = CompletionItem {
                    label: format!("/{tag}"),
                    kind: Some(CompletionItemKind::PROPERTY),
                    filter_text: Some(format!("/{tag}")),
                    text_edit: replace(range, format!("/{tag}{close_tag}")),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                };
                let start_indent = self.line_indent(node.start);
                let end_indent = self.line_indent(after_open_bracket - 1);
                if let (Some(start_indent), Some(end_indent)) = (start_indent, end_indent)
                    && start_indent != end_indent
                {
                    let insert_text = format!("{start_indent}</{tag}{close_tag}");
                    item.text_edit = replace(
                        self.replace_range(after_open_bracket - 1 - end_indent.len(), self.offset),
                        insert_text,
                    );
                    item.filter_text = Some(format!("{end_indent}</{tag}"));
                }
                self.push(item);
                return;
            }
            curr = node.parent;
        }
        if in_open_tag {
            return;
        }
        let mut items = Vec::new();
        for provider in self.providers() {
            for tag in provider.provide_tags() {
                items.push(CompletionItem {
                    label: format!("/{}", tag.name),
                    kind: Some(CompletionItemKind::PROPERTY),
                    documentation: markup(generate_documentation(
                        tag,
                        HoverSettings::default(),
                        self.supports_markdown,
                    )),
                    filter_text: Some(format!("/{}{close_tag}", tag.name)),
                    text_edit: replace(range, format!("/{}{close_tag}", tag.name)),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                });
            }
        }
        self.result.items.extend(items);
    }

    fn collect_auto_close_tag_suggestion(&mut self, tag_close_end: usize, tag: &str) {
        if self
            .settings
            .is_some_and(|s| s.hide_auto_complete_proposals)
        {
            return;
        }
        let providers: Vec<_> = self
            .ls
            .data_manager
            .data_providers()
            .iter()
            .filter(|p| {
                self.settings
                    .is_none_or(|s| s.providers.get(p.id()) != Some(&false))
            })
            .cloned()
            .collect();
        let void_elements = crate::language_facts::void_elements_of(&providers);
        if !HtmlDataManager::is_void_element(tag, &void_elements) {
            let pos = self.document.position_at(tag_close_end);
            self.push(CompletionItem {
                label: format!("</{tag}>"),
                kind: Some(CompletionItemKind::PROPERTY),
                filter_text: Some(format!("</{tag}>")),
                text_edit: replace(Range::new(pos, pos), format!("$0</{tag}>")),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
        }
    }

    fn collect_tag_suggestions(&mut self, tag_start: usize, tag_end: usize) {
        self.collect_open_tag_suggestions(tag_start, tag_end);
        self.collect_close_tag_suggestions(tag_start, true, tag_end);
    }

    fn collect_attribute_name_suggestions(
        &mut self,
        current_tag: &str,
        name_start: usize,
        name_end: usize,
    ) {
        let text = self.text;
        let mut replace_end = self.offset;
        // `<` is a valid attribute name character, but the name likely ends there (#23236).
        while replace_end < name_end && text.as_bytes()[replace_end] != b'<' {
            replace_end += 1;
        }
        let current_attribute = substring(text, name_start, name_end);
        let range = self.replace_range(name_start, replace_end);
        let mut value = "";
        if !is_followed_by(
            text,
            name_end,
            ScannerState::AfterAttributeName,
            TokenType::DelimiterAssign,
        ) {
            value = match self.settings.and_then(|s| s.attribute_default_value) {
                Some(AttributeDefaultValue::Empty) => "=$1",
                Some(AttributeDefaultValue::Singlequotes) => "='$1'",
                _ => "=\"$1\"",
            };
        }
        let mut seen: HashMap<String, bool> = self.html_document[self.node]
            .attribute_names()
            .map(|a| (a.to_owned(), true))
            .collect();
        seen.insert(current_attribute.to_owned(), false);

        let mut items = Vec::new();
        for provider in self.providers() {
            for attr in provider.provide_attributes(current_tag) {
                if seen.get(&attr.name) == Some(&true) {
                    continue;
                }
                seen.insert(attr.name.clone(), true);
                let mut code_snippet = attr.name.clone();
                let mut command = None;
                if attr.value_set.as_deref() != Some("v") && !value.is_empty() {
                    code_snippet += value;
                    if attr.value_set.as_deref().is_some_and(|v| !v.is_empty())
                        || attr.name == "style"
                    {
                        command = Some(Command {
                            title: "Suggest".to_owned(),
                            command: "editor.action.triggerSuggest".to_owned(),
                            arguments: None,
                        });
                    }
                }
                items.push(CompletionItem {
                    label: attr.name.clone(),
                    kind: Some(if attr.value_set.as_deref() == Some("handler") {
                        CompletionItemKind::FUNCTION
                    } else {
                        CompletionItemKind::VALUE
                    }),
                    documentation: markup(generate_documentation(
                        attr,
                        HoverSettings::default(),
                        self.supports_markdown,
                    )),
                    text_edit: replace(range, code_snippet),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    command,
                    ..Default::default()
                });
            }
        }
        self.result.items.extend(items);
        self.collect_data_attributes_suggestions(range, &seen);
    }

    fn collect_data_attributes_suggestions(&mut self, range: Range, seen: &HashMap<String, bool>) {
        const DATA_ATTR: &str = "data-";
        let mut data_attributes: Vec<(String, String)> =
            vec![(DATA_ATTR.to_owned(), format!("{DATA_ATTR}$1=\"$2\""))];
        let doc = self.html_document;
        let mut stack: Vec<NodeId> = doc.roots().iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            for attr in doc[id].attribute_names() {
                if attr.starts_with(DATA_ATTR)
                    && !data_attributes.iter().any(|(a, _)| a == attr)
                    && seen.get(attr) != Some(&true)
                {
                    data_attributes.push((attr.to_owned(), format!("{attr}=\"$1\"")));
                }
            }
            stack.extend(doc[id].children.iter().rev());
        }
        for (label, snippet) in data_attributes {
            self.push(CompletionItem {
                label,
                kind: Some(CompletionItemKind::VALUE),
                text_edit: replace(range, snippet),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
        }
    }

    fn collect_attribute_value_suggestions(
        &mut self,
        current_tag: &str,
        current_attribute_name: Option<&str>,
        value_start: usize,
        value_end: usize,
    ) {
        let text = self.text;
        let b = text.as_bytes();
        let offset = self.offset;
        let (range, value_prefix, add_quotes) = if offset > value_start
            && offset <= value_end
            && matches!(b.get(value_start), Some(b'"' | b'\''))
        {
            // Inside a quoted attribute.
            let value_content_start = value_start + 1;
            let mut value_content_end = value_end;
            if value_end > value_start && b[value_end - 1] == b[value_start] {
                // The scanner can take a later quote across tag boundaries
                // (microsoft/vscode#273226): a `<` before the quote makes the
                // value unclosed.
                let crosses_tag_boundary =
                    offset < value_end - 1 && b[offset..value_end - 1].contains(&b'<');
                value_content_end = if crosses_tag_boundary {
                    offset
                } else {
                    value_content_end - 1
                };
            } else {
                value_content_end = offset;
            }
            let ws_before = word_start(text, offset, value_content_start);
            let ws_after = word_end(text, offset, value_content_end);
            (
                self.replace_range(ws_before, ws_after),
                if offset >= value_content_start && offset <= value_content_end {
                    &text[value_content_start..offset]
                } else {
                    ""
                },
                false,
            )
        } else {
            (
                self.replace_range(value_start, value_end),
                substring(text, value_start, offset),
                true,
            )
        };

        if !self.participants.is_empty() {
            let full_range = self.replace_range(value_start, value_end);
            let context = HtmlAttributeValueContext {
                position: self.position,
                tag: current_tag.to_lowercase(),
                attribute: current_attribute_name.unwrap_or("").to_lowercase(),
                value: value_prefix.to_owned(),
                range: full_range,
                attributes: self.html_document[self.node].attributes.clone(),
            };
            for participant in self.participants.iter_mut() {
                participant.on_html_attribute_value(&context);
            }
        }
        let mut items = Vec::new();
        for provider in self.providers() {
            for value in provider.provide_values(current_tag, current_attribute_name.unwrap_or(""))
            {
                let insert_text = if add_quotes {
                    format!("\"{}\"", value.name)
                } else {
                    value.name.clone()
                };
                items.push(CompletionItem {
                    label: value.name.clone(),
                    filter_text: Some(insert_text.clone()),
                    kind: Some(CompletionItemKind::UNIT),
                    documentation: markup(generate_documentation(
                        value,
                        HoverSettings::default(),
                        self.supports_markdown,
                    )),
                    text_edit: replace(range, insert_text),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                });
            }
        }
        self.result.items.extend(items);
        self.collect_character_entity_proposals();
    }

    fn collect_inside_content(&mut self) {
        for participant in self.participants.iter_mut() {
            participant.on_html_content(self.position);
        }
        self.collect_character_entity_proposals();
    }

    fn collect_character_entity_proposals(&mut self) {
        let text = self.text;
        let mut k = self.offset;
        let mut character_start = self.position.character as i64;
        while k > 0 && is_letter_or_digit(text, k - 1) {
            k -= 1;
            character_start -= 1;
        }
        if k > 0 && text.as_bytes()[k - 1] == b'&' {
            // Upstream `Range.create` does not check the numbers.
            let range = Range::new(
                Position::new(self.position.line, (character_start - 1) as u32),
                self.position,
            );
            for (entity, value) in ENTITIES {
                if entity.ends_with(';') {
                    let label = format!("&{entity}");
                    self.result.items.push(CompletionItem {
                        label: label.clone(),
                        kind: Some(CompletionItemKind::KEYWORD),
                        documentation: Some(Documentation::String(format!(
                            "Character entity representing '{value}'"
                        ))),
                        text_edit: replace(range, label),
                        insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                        ..Default::default()
                    });
                }
            }
        }
    }

    fn suggest_doctype(&mut self, replace_start: usize, replace_end: usize) {
        let range = self.replace_range(replace_start, replace_end);
        self.push(CompletionItem {
            label: "!DOCTYPE".to_owned(),
            kind: Some(CompletionItemKind::PROPERTY),
            documentation: Some(Documentation::String(
                "A preamble for an HTML document.".to_owned(),
            )),
            text_edit: replace(range, "!DOCTYPE html>"),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        });
    }

    fn run(&mut self) {
        let offset = self.offset;
        self.node = self.html_document.find_node_before(offset);
        let text = self.text;
        let mut scanner = Scanner::at(text, self.html_document[self.node].start);
        let mut current_tag = "";
        let mut current_attribute_name: Option<&str> = None;

        // `scanNextForEndPos`.
        let scan_next_for_end_pos = |scanner: &mut Scanner, next_token: TokenType| {
            if offset == scanner.token_end() {
                let token = scanner.scan();
                if token == next_token && scanner.token_offset() == offset {
                    return scanner.token_end();
                }
            }
            offset
        };

        let mut token = scanner.scan();
        while token != TokenType::Eos && scanner.token_offset() <= offset {
            match token {
                TokenType::StartTagOpen => {
                    if scanner.token_end() == offset {
                        let end_pos = scan_next_for_end_pos(&mut scanner, TokenType::StartTag);
                        if self.position.line == 0 {
                            self.suggest_doctype(offset, end_pos);
                        }
                        return self.collect_tag_suggestions(offset, end_pos);
                    }
                }
                TokenType::StartTag => {
                    if scanner.token_offset() <= offset && offset <= scanner.token_end() {
                        return self.collect_open_tag_suggestions(
                            scanner.token_offset(),
                            scanner.token_end(),
                        );
                    }
                    current_tag = scanner.token_text();
                }
                TokenType::AttributeName => {
                    if scanner.token_offset() <= offset && offset <= scanner.token_end() {
                        return self.collect_attribute_name_suggestions(
                            current_tag,
                            scanner.token_offset(),
                            scanner.token_end(),
                        );
                    }
                    current_attribute_name = Some(scanner.token_text());
                }
                TokenType::DelimiterAssign => {
                    if scanner.token_end() == offset {
                        let end_pos =
                            scan_next_for_end_pos(&mut scanner, TokenType::AttributeValue);
                        return self.collect_attribute_value_suggestions(
                            current_tag,
                            current_attribute_name,
                            offset,
                            end_pos,
                        );
                    }
                }
                TokenType::AttributeValue => {
                    if scanner.token_offset() <= offset && offset <= scanner.token_end() {
                        return self.collect_attribute_value_suggestions(
                            current_tag,
                            current_attribute_name,
                            scanner.token_offset(),
                            scanner.token_end(),
                        );
                    }
                }
                TokenType::Whitespace => {
                    if offset <= scanner.token_end() {
                        match scanner.scanner_state() {
                            ScannerState::AfterOpeningStartTag => {
                                let start_pos = scanner.token_offset();
                                let end_tag_pos =
                                    scan_next_for_end_pos(&mut scanner, TokenType::StartTag);
                                return self.collect_tag_suggestions(start_pos, end_tag_pos);
                            }
                            ScannerState::WithinTag | ScannerState::AfterAttributeName => {
                                return self.collect_attribute_name_suggestions(
                                    current_tag,
                                    scanner.token_end(),
                                    offset,
                                );
                            }
                            ScannerState::BeforeAttributeValue => {
                                return self.collect_attribute_value_suggestions(
                                    current_tag,
                                    current_attribute_name,
                                    scanner.token_end(),
                                    offset,
                                );
                            }
                            ScannerState::AfterOpeningEndTag => {
                                return self.collect_close_tag_suggestions(
                                    scanner.token_offset() - 1,
                                    false,
                                    offset,
                                );
                            }
                            ScannerState::WithinContent => return self.collect_inside_content(),
                            _ => {}
                        }
                    }
                }
                TokenType::EndTagOpen => {
                    if offset <= scanner.token_end() {
                        let after_open_bracket = scanner.token_offset() + 1;
                        let end_offset = scan_next_for_end_pos(&mut scanner, TokenType::EndTag);
                        return self.collect_close_tag_suggestions(
                            after_open_bracket,
                            false,
                            end_offset,
                        );
                    }
                }
                TokenType::EndTag => {
                    if offset <= scanner.token_end() {
                        let mut start = scanner.token_offset();
                        while let Some(ch) = char_before(text, start) {
                            start -= ch.len_utf8();
                            if ch == '/' {
                                return self.collect_close_tag_suggestions(
                                    start,
                                    false,
                                    scanner.token_end(),
                                );
                            } else if !is_whitespace_or_empty(Some(ch)) {
                                break;
                            }
                        }
                    }
                }
                TokenType::StartTagClose => {
                    if offset <= scanner.token_end() && !current_tag.is_empty() {
                        return self
                            .collect_auto_close_tag_suggestion(scanner.token_end(), current_tag);
                    }
                }
                TokenType::Content => {
                    if offset <= scanner.token_end() {
                        return self.collect_inside_content();
                    }
                }
                _ => {
                    if offset <= scanner.token_end() {
                        return;
                    }
                }
            }
            token = scanner.scan();
        }
    }
}

/// `isFollowedBy`.
fn is_followed_by(
    s: &str,
    offset: usize,
    initial_state: ScannerState,
    expected: TokenType,
) -> bool {
    let mut scanner = Scanner::new(s, offset, initial_state, false);
    let mut token = scanner.scan();
    while token == TokenType::Whitespace {
        token = scanner.scan();
    }
    token == expected
}

/// `getWordStart`.
fn word_start(s: &str, mut offset: usize, limit: usize) -> usize {
    while offset > limit {
        let ch = char_before(s, offset);
        if is_whitespace_or_empty(ch) {
            break;
        }
        offset -= ch.map_or(1, char::len_utf8);
    }
    offset
}

/// `getWordEnd`.
fn word_end(s: &str, mut offset: usize, limit: usize) -> usize {
    while offset < limit {
        let ch = char_at(s, offset);
        if is_whitespace_or_empty(ch) {
            break;
        }
        offset += ch.map_or(1, char::len_utf8);
    }
    offset
}
