//! Port of `cssCompletion.ts`, `scssCompletion.ts` and `lessCompletion.ts`.
//! A method that a subclass overrides matches on the dialect.

use foldhash::HashSet;
use indexmap::IndexSet;

use lsp_types::{
    ClientCapabilities, Command, CompletionItem, CompletionItemKind, CompletionItemTag,
    CompletionTextEdit, Documentation, InsertTextFormat, MarkupContent, MarkupKind, Position,
    Range, TextEdit,
};
use serde::{Deserialize, Serialize};
use text_document::TextDocument;

use crate::data::{PropertyData, ValueData};
use crate::language_facts::CssDataManager;
use crate::language_facts::builtin_data as facts;
use crate::language_facts::color_data::{COLOR_FUNCTIONS, COLOR_KEYWORDS, COLORS};
use crate::language_facts::colors::{is_color_string, is_color_value};
use crate::language_facts::entry::{Entry, get_entry_description};
use crate::parser::nodes::{Ast, Field, Kind, NodeId, NodeType, ReferenceType};
use crate::parser::scanner::{Scanner, TokenType};
use crate::parser::symbol_scope::{SymbolId, Symbols};
use crate::parser::{Dialect, Stylesheet};
use crate::services::links::{DocumentContext, FileSystemProvider};
use crate::services::path_completion::PathCompletionParticipant;
use crate::utils::{limited_string, utf16_len};

/// `CompletionSettings`.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompletionSettings {
    pub trigger_property_value_completion: Option<bool>,
    pub complete_property_with_semicolon: Option<bool>,
}

/// `CompletionList` with `itemDefaults`, which `lsp_types` does not have.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionList {
    pub is_incomplete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_defaults: Option<ItemDefaults>,
    pub items: Vec<CompletionItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDefaults {
    pub edit_range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyCompletionContext {
    pub property_name: String,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyValueCompletionContext {
    pub property_name: String,
    pub property_value: String,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UriLiteralCompletionContext {
    pub uri_value: String,
    pub position: Position,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPathCompletionContext {
    pub path_value: String,
    pub position: Position,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MixinReferenceCompletionContext {
    pub mixin_name: String,
    pub range: Range,
}

/// `ICompletionParticipant`.
pub trait CompletionParticipant {
    fn on_css_property(&mut self, _context: PropertyCompletionContext) {}
    fn on_css_property_value(&mut self, _context: PropertyValueCompletionContext) {}
    fn on_css_uri_literal_value(&mut self, _context: UriLiteralCompletionContext) {}
    fn on_css_import_path(&mut self, _context: ImportPathCompletionContext) {}
    fn on_css_mixin_reference(&mut self, _context: MixinReferenceCompletionContext) {}
}

/// `doesSupportMarkdown`: `true` without client capabilities.
pub fn supports_markdown(capabilities: Option<&ClientCapabilities>) -> bool {
    let Some(capabilities) = capabilities else {
        return true;
    };
    capabilities
        .text_document
        .as_ref()
        .and_then(|t| t.completion.as_ref())
        .and_then(|c| c.completion_item.as_ref())
        .and_then(|i| i.documentation_format.as_ref())
        .is_some_and(|f| f.contains(&MarkupKind::Markdown))
}

/// `CSSCompletion`, `SCSSCompletion` and `LESSCompletion`.
pub struct Completion<'a> {
    pub dialect: Dialect,
    pub data: &'a CssDataManager,
    pub markdown: bool,
}

impl Completion<'_> {
    /// `doComplete2`: `doComplete` with path completion. The file system
    /// must provide `readDirectory`. `None` where upstream rejects.
    #[allow(clippy::too_many_arguments)]
    pub fn do_complete2(
        &self,
        document: &TextDocument,
        position: Position,
        stylesheet: &Stylesheet,
        context: &dyn DocumentContext,
        file_system: Option<&dyn FileSystemProvider>,
        settings: Option<&CompletionSettings>,
        participants: &mut [&mut dyn CompletionParticipant],
    ) -> Option<CompletionList> {
        let Some(file_system) = file_system else {
            return Some(self.do_complete(document, position, stylesheet, settings, participants));
        };
        let mut path = PathCompletionParticipant::default();
        let result = {
            let mut all: Vec<&mut dyn CompletionParticipant> = vec![&mut path];
            all.extend(
                participants
                    .iter_mut()
                    .map(|p| &mut **p as &mut dyn CompletionParticipant),
            );
            self.do_complete(document, position, stylesheet, settings, &mut all)
        };
        let mut path_result = path.compute_completions(document, context, file_system)?;
        path_result.items.extend(result.items);
        Some(CompletionList {
            is_incomplete: result.is_incomplete || path_result.is_incomplete,
            item_defaults: result.item_defaults,
            items: path_result.items,
        })
    }

    /// `doComplete`.
    pub fn do_complete(
        &self,
        document: &TextDocument,
        position: Position,
        stylesheet: &Stylesheet,
        settings: Option<&CompletionSettings>,
        participants: &mut [&mut dyn CompletionParticipant],
    ) -> CompletionList {
        let ast = &stylesheet.ast;
        let offset = document.offset_at(position);
        let mut state = State {
            dialect: self.dialect,
            data: self.data,
            markdown: self.markdown,
            settings: settings.copied().unwrap_or_default(),
            participants,
            document,
            ast,
            root: stylesheet.root,
            offset: offset as isize,
            position,
            current_word: String::new(),
            default_replace_range: Range::default(),
            node_path: Vec::new(),
            symbols: None,
            result: CompletionList::default(),
        };
        if state.is_in_comment() {
            return CompletionList::default();
        }
        state.current_word = get_current_word(document.text(), offset).to_owned();
        state.default_replace_range = state.word_range();
        state.result.item_defaults = Some(ItemDefaults {
            edit_range: state.default_replace_range,
        });
        state.node_path = ast.get_node_path(state.root, state.offset);
        state.complete();
        state.result
    }
}

fn retrigger() -> Command {
    Command {
        title: "Suggest".into(),
        command: "editor.action.triggerSuggest".into(),
        arguments: None,
    }
}

mod sort_texts {
    pub const ENUMS: &str = " ";
    pub const NORMAL: &str = "d";
    pub const VENDOR_PREFIXED: &str = "x";
    pub const TERM: &str = "y";
    pub const VARIABLE: &str = "z";
}

struct State<'a, 'p> {
    dialect: Dialect,
    data: &'a CssDataManager,
    markdown: bool,
    settings: CompletionSettings,
    participants: &'a mut [&'p mut dyn CompletionParticipant],
    document: &'a TextDocument,
    ast: &'a Ast,
    root: NodeId,
    offset: isize,
    position: Position,
    current_word: String,
    default_replace_range: Range,
    node_path: Vec<NodeId>,
    symbols: Option<Symbols>,
    result: CompletionList,
}

fn item(label: impl Into<String>, kind: CompletionItemKind) -> CompletionItem {
    CompletionItem {
        label: label.into(),
        kind: Some(kind),
        ..Default::default()
    }
}

fn edit(range: Range, text: impl Into<String>) -> Option<CompletionTextEdit> {
    Some(CompletionTextEdit::Edit(TextEdit::new(range, text.into())))
}

fn doc(text: &str) -> Option<Documentation> {
    Some(Documentation::String(text.to_owned()))
}

fn tags(deprecated: bool) -> Option<Vec<CompletionItemTag>> {
    Some(if deprecated {
        vec![CompletionItemTag::DEPRECATED]
    } else {
        Vec::new()
    })
}

/// `isDeprecated`.
fn is_deprecated(entry: &(impl Entry + ?Sized)) -> bool {
    matches!(entry.status(), Some("nonstandard" | "obsolete"))
}

fn snippet_if(cond: bool) -> Option<InsertTextFormat> {
    cond.then_some(InsertTextFormat::SNIPPET)
}

/// `moveCursorInsideParenthesis`.
fn move_cursor_inside_parenthesis(text: &str) -> String {
    match text.strip_suffix("()") {
        Some(name) => format!("{name}($1)"),
        None => text.to_owned(),
    }
}

/// `getCurrentWord`.
fn get_current_word(text: &str, offset: usize) -> &str {
    let bytes = text.as_bytes();
    let mut i = offset;
    while i > 0 && !b" \t\n\r\":{[()]},*>+".contains(&bytes[i - 1]) {
        i -= 1;
    }
    &text[i..offset]
}

impl State<'_, '_> {
    fn text(&self) -> &str {
        self.document.text()
    }

    fn position_at(&self, offset: isize) -> Position {
        self.document.position_at(offset.max(0) as usize)
    }

    /// The range of the current word before the position.
    fn word_range(&self) -> Range {
        let start = self
            .position
            .character
            .saturating_sub(utf16_len(&self.current_word) as u32);
        Range::new(Position::new(self.position.line, start), self.position)
    }

    fn push(&mut self, item: CompletionItem) {
        self.result.items.push(item);
    }

    fn colon_position(&self, node: NodeId) -> Option<isize> {
        self.ast.colon_position(node)
    }

    fn symbols(&mut self) -> &Symbols {
        let (ast, root) = (self.ast, self.root);
        self.symbols.get_or_insert_with(|| Symbols::new(ast, root))
    }

    fn find_symbols(&mut self, ty: ReferenceType) -> Vec<SymbolId> {
        let offset = self.offset;
        self.symbols().find_symbols_at_offset(offset, ty)
    }

    /// `getCommentScanStart`: a node never starts inside a comment, so the last
    /// node that starts before the offset is a safe place to start the scan.
    fn comment_scan_start(&self) -> usize {
        let ast = self.ast;
        let mut node = Some(self.root);
        let mut start = 0;
        while let Some(n) = node {
            let last = ast
                .children(n)
                .rev()
                .find(|&c| ast.offset(c) >= 0 && ast.offset(c) < self.offset);
            if let Some(l) = last {
                start = ast.offset(l);
                // `url(...)` is scanned in its own mode, so never start inside one.
                if let Some(uri) = ast.find_parent(l, NodeType::URILiteral) {
                    return ast.offset(uri) as usize;
                }
            }
            node = last;
        }
        start as usize
    }

    /// `isInComment`: comments are not in the tree, so scan the text.
    fn is_in_comment(&self) -> bool {
        let offset = self.offset as usize;
        let mut scanner = Scanner::new(self.dialect);
        scanner.ignore_comment = false;
        scanner.set_source(self.text());
        scanner.go_back_to(self.comment_scan_start());
        let mut previous: Option<(TokenType, usize, bool)> = None;
        loop {
            let token = scanner.scan();
            if token.kind == TokenType::Eof || token.offset > offset {
                return false;
            }
            // `//` does not start a comment in an unquoted URL. The parser reads
            // the argument as one unquoted string in URL mode.
            if token.kind == TokenType::ParenthesisL
                && let Some((TokenType::Ident, end, true)) = previous
                && end == token.offset
            {
                scanner.in_url = true;
                scanner.scan_unquoted_string();
                scanner.in_url = false;
            }
            let text = scanner.text(&token);
            let is_url =
                text.eq_ignore_ascii_case("url") || text.eq_ignore_ascii_case("url-prefix");
            previous = Some((token.kind, token.end(), is_url));

            if token.kind != TokenType::Comment || offset <= token.offset {
                continue;
            }
            let end = token.end();
            if offset < end {
                return true;
            }
            // At the end of a comment that is not closed (`/* ...` or `// ...`),
            // the text is still in the comment. `/*/` is not closed.
            let closed = text.starts_with("/*") && token.len >= 4 && text.ends_with("*/");
            if offset == end && !closed {
                return true;
            }
        }
    }

    fn complete(&mut self) {
        let ast = self.ast;
        for i in (0..self.node_path.len()).rev() {
            let node = self.node_path[i];
            let kind = ast.kind(node);
            let ty = ast.node_type(node);
            if kind == Kind::Property {
                self.declaration_property(ast.get_parent(node));
            } else if kind == Kind::Expression {
                if ast
                    .parent(node)
                    .is_some_and(|p| ast.kind(p) == Kind::Interpolation)
                {
                    self.variable_proposals(None);
                } else {
                    self.expression(node);
                }
            } else if kind == Kind::SimpleSelector {
                let types = [NodeType::ExtendsReference, NodeType::Ruleset];
                if let Some(parent) = ast.find_a_parent(node, &types) {
                    if ast.node_type(parent) == NodeType::ExtendsReference {
                        self.extends_reference(Some(node));
                    } else {
                        let nested = self.is_nested(parent);
                        self.selector(Some(parent), nested);
                    }
                }
            } else if kind == Kind::FunctionArgument {
                self.function_argument(Some(node), ast.get_parent(node));
            } else if kind == Kind::Declarations {
                self.declarations(Some(node));
            } else if kind == Kind::VariableDeclaration {
                self.variable_declaration(node);
            } else if kind == Kind::RuleSet {
                self.rule_set(node);
            } else if kind == Kind::Interpolation {
                if self.offset >= ast.offset(node) + 2 {
                    self.variable_proposals(None);
                }
            } else if kind == Kind::FunctionDeclaration {
                self.function_declaration(node);
            } else if kind == Kind::MixinReference {
                self.mixin_reference(node);
            } else if kind == Kind::Function {
                self.function_argument(None, Some(node));
            } else if kind == Kind::Supports {
                self.supports(node);
            } else if kind == Kind::SupportsCondition {
                self.supports_condition(node);
            } else if kind == Kind::MediaCondition {
                self.media_condition(node);
            } else if kind == Kind::ExtendsReference {
                self.extends_reference(None);
            } else if ty == NodeType::URILiteral {
                self.uri_literal_value(node);
            } else if ast.parent(node).is_none() {
                self.top_level();
            } else if ty == NodeType::StringLiteral
                && ast
                    .parent(node)
                    .is_some_and(|p| self.is_import_path_parent(ast.node_type(p)))
            {
                self.import_path(node);
            } else {
                continue;
            }
            if !self.result.items.is_empty() || self.offset > ast.offset(node) {
                return;
            }
        }
        self.stylesheet();
        if self.result.items.is_empty() {
            let prefix = match self.dialect {
                Dialect::Css => None,
                Dialect::Scss => Some('$'),
                Dialect::Less => Some('@'),
            };
            if prefix.is_some_and(|p| self.current_word.starts_with(p)) {
                self.variable_proposals(None);
            }
        }
    }

    fn is_import_path_parent(&self, ty: NodeType) -> bool {
        ty == NodeType::Import
            || self.dialect == Dialect::Scss && matches!(ty, NodeType::Forward | NodeType::Use)
    }

    /// `RuleSet.isNested`.
    fn is_nested(&self, rule_set: NodeId) -> bool {
        self.ast
            .parent(rule_set)
            .is_some_and(|p| self.ast.find_parent(p, NodeType::Declarations).is_some())
    }

    fn find_in_node_path(&self, types: &[NodeType]) -> Option<NodeId> {
        self.node_path
            .iter()
            .rev()
            .copied()
            .find(|&n| types.contains(&self.ast.node_type(n)))
    }

    fn completion_range(&self, existing: Option<NodeId>) -> Range {
        if let Some(n) = existing {
            let (offset, end) = (self.ast.offset(n), self.ast.end(n));
            if offset <= self.offset && self.offset <= end {
                let end = if end != -1 {
                    self.position_at(end)
                } else {
                    self.position
                };
                let start = self.position_at(offset);
                // Multi-line edits are not allowed.
                if start.line == end.line {
                    return Range::new(start, end);
                }
            }
        }
        self.default_replace_range
    }

    fn has_character_at(&self, offset: isize, ch: u8) -> bool {
        offset >= 0 && self.text().as_bytes().get(offset as usize) == Some(&ch)
    }

    // Properties

    /// `getCompletionsForDeclarationProperty`.
    fn declaration_property(&mut self, declaration: Option<NodeId>) {
        match self.dialect {
            Dialect::Css => {}
            Dialect::Scss => {
                self.result.items.extend(scss::at_directives());
                self.selector(None, true);
            }
            Dialect::Less => self.selector(None, true),
        }
        self.property_proposals(declaration);
    }

    /// `getPropertyProposals`.
    fn property_proposals(&mut self, declaration: Option<NodeId>) {
        let trigger = self
            .settings
            .trigger_property_value_completion
            .unwrap_or(true);
        let semicolon = self
            .settings
            .complete_property_with_semicolon
            .unwrap_or(true);
        let range =
            self.completion_range(declaration.and_then(|d| self.ast.field(d, Field::Property)));
        let (mut suffix, needs_value) = match declaration {
            Some(d) if self.colon_position(d).is_some() => ("", false),
            _ => (": ", true),
        };
        let semicolon_suffix = match declaration {
            None => semicolon,
            // Cases such as `.selector { p; }` or `.selector { p:; }`.
            Some(d) => {
                let semicolon_position = self.ast.semicolon_position(d);
                matches!(semicolon_position, None | Some(0))
                    && semicolon
                    && self.offset as usize >= self.document.offset_at(range.end)
            }
        };
        let with_semicolon;
        if semicolon_suffix {
            with_semicolon = format!("{suffix}$0;");
            suffix = &with_semicolon;
        }
        for entry in self.data.get_properties() {
            let relevance = entry.relevance.map_or(50.0, |r| r.clamp(0.0, 99.0));
            let prefix = if entry.name.starts_with('-') {
                sort_texts::VENDOR_PREFIXED
            } else {
                sort_texts::NORMAL
            };
            let item = CompletionItem {
                documentation: get_entry_description(&**entry, self.markdown, None)
                    .map(Documentation::MarkupContent),
                tags: tags(is_deprecated(&**entry)),
                text_edit: edit(range, format!("{}{suffix}", entry.name)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                command: (trigger && needs_value).then(retrigger),
                sort_text: Some(format!("{prefix}_{:x}", (255.0 - relevance).round() as u8)),
                ..item(&entry.name, CompletionItemKind::PROPERTY)
            };
            self.push(item);
        }
        let context = PropertyCompletionContext {
            property_name: self.current_word.clone(),
            range: self.default_replace_range,
        };
        for p in self.participants.iter_mut() {
            p.on_css_property(context.clone());
        }
    }

    // Values

    /// `getCompletionsForDeclarationValue`.
    fn declaration_value(&mut self, node: NodeId) {
        let ast = self.ast;
        let property_name = ast.full_property_name(node);
        let entry = self.data.get_property(&property_name);
        let mut existing = ast.field(node, Field::Value);
        while let Some(e) = existing
            && ast.has_children(e)
        {
            existing = ast.find_child_at_offset(e, self.offset, false);
        }
        let context = PropertyValueCompletionContext {
            property_name,
            property_value: self.current_word.clone(),
            range: self.completion_range(existing),
        };
        for p in self.participants.iter_mut() {
            p.on_css_property_value(context.clone());
        }

        if let Some(entry) = entry {
            for restriction in entry.restrictions.iter().flatten() {
                match restriction.as_str() {
                    "color" => self.color_proposals(existing),
                    "position" => self.keyword_proposals(facts::POSITION_KEYWORDS, existing),
                    "repeat" => self.keyword_proposals(facts::REPEAT_STYLE_KEYWORDS, existing),
                    "line-style" => self.keyword_proposals(facts::LINE_STYLE_KEYWORDS, existing),
                    "line-width" => {
                        let range = self.completion_range(existing);
                        for &w in facts::LINE_WIDTH_KEYWORDS {
                            self.push(CompletionItem {
                                text_edit: edit(range, w),
                                ..item(w, CompletionItemKind::VALUE)
                            });
                        }
                    }
                    "geometry-box" => {
                        self.keyword_proposals(facts::GEOMETRY_BOX_KEYWORDS, existing)
                    }
                    "box" => self.keyword_proposals(facts::BOX_KEYWORDS, existing),
                    "image" => self.function_keyword_proposals(facts::IMAGE_FUNCTIONS, existing),
                    "timing-function" => self
                        .function_keyword_proposals(facts::TRANSITION_TIMING_FUNCTIONS, existing),
                    "shape" => {
                        self.function_keyword_proposals(facts::BASIC_SHAPE_FUNCTIONS, existing)
                    }
                    _ => {}
                }
            }
            self.value_enum_proposals(entry.values.as_deref(), existing);
            self.css_wide_keyword_proposals(existing);
            self.unit_proposals(entry, existing);
        } else {
            let range = self.completion_range(existing);
            for value in self.collect_values(node) {
                self.push(CompletionItem {
                    text_edit: edit(range, &value),
                    ..item(value, CompletionItemKind::VALUE)
                });
            }
        }
        self.variable_proposals(existing);
        self.term_proposals(entry, existing);
    }

    /// `getValueEnumProposals`.
    fn value_enum_proposals(&mut self, values: Option<&[ValueData]>, existing: Option<NodeId>) {
        let range = self.completion_range(existing);
        for value in values.into_iter().flatten() {
            let mut insert = value.name.clone();
            let mut format = None;
            if insert.ends_with(')')
                && let Some(from) = insert.rfind('(')
            {
                insert.insert_str(from + 1, "$1");
                format = Some(InsertTextFormat::SNIPPET);
            }
            let mut sort_text = sort_texts::ENUMS.to_owned();
            if value.name.starts_with('-') {
                sort_text += sort_texts::VENDOR_PREFIXED;
            }
            self.push(CompletionItem {
                documentation: get_entry_description(value, self.markdown, None)
                    .map(Documentation::MarkupContent),
                tags: tags(is_deprecated(value)),
                text_edit: edit(range, insert),
                sort_text: Some(sort_text),
                insert_text_format: format,
                ..item(&value.name, CompletionItemKind::VALUE)
            });
        }
    }

    /// `getCSSWideKeywordProposals`.
    fn css_wide_keyword_proposals(&mut self, existing: Option<NodeId>) {
        let range = self.completion_range(existing);
        for &(keyword, description) in facts::CSS_WIDE_KEYWORDS {
            self.push(CompletionItem {
                documentation: doc(description),
                text_edit: edit(range, keyword),
                ..item(keyword, CompletionItemKind::VALUE)
            });
        }
        for &(func, description) in facts::CSS_WIDE_FUNCTIONS {
            self.push(CompletionItem {
                documentation: doc(description),
                text_edit: edit(range, move_cursor_inside_parenthesis(func)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                command: func.starts_with("var").then(retrigger),
                ..item(func, CompletionItemKind::FUNCTION)
            });
        }
    }

    /// `getVariableProposals`.
    fn variable_proposals(&mut self, existing: Option<NodeId>) {
        let range = self.completion_range(existing);
        let ast = self.ast;
        for id in self.find_symbols(ReferenceType::Variable) {
            let symbol = self.symbols().symbol(id).clone();
            let insert = if symbol.name.starts_with("--") {
                format!("var({})", symbol.name)
            } else {
                symbol.name.clone()
            };
            let documentation = symbol.value.as_deref().map(limited_string);
            let kind = if documentation.as_deref().is_some_and(is_color_string) {
                CompletionItemKind::COLOR
            } else {
                CompletionItemKind::VARIABLE
            };
            let mut detail = None;
            if ast.node_type(symbol.node) == NodeType::FunctionParameter
                && let Some(mixin) = ast.get_parent(symbol.node)
                && ast.node_type(mixin) == NodeType::MixinDeclaration
            {
                detail = Some(format!("argument from '{}'", ast.identifier_name(mixin)));
            }
            self.push(CompletionItem {
                documentation: documentation.map(Documentation::String),
                text_edit: edit(range, insert),
                sort_text: Some(sort_texts::VARIABLE.into()),
                detail,
                ..item(symbol.name, kind)
            });
        }
    }

    /// `getVariableProposalsForCSSVarFunction`.
    fn variable_proposals_for_css_var_function(&mut self) {
        let ast = self.ast;
        let mut referenced = IndexSet::<_, foldhash::fast::RandomState>::default();
        ast.accept(self.root, &mut |n| {
            if ast.kind(n) == Kind::Identifier
                && ast.node(n).is_custom_property
                && (self.offset < ast.offset(n) || ast.end(n) < self.offset)
            {
                referenced.insert(ast.get_text(n));
            }
            true
        });
        let range = self.completion_range(None);
        for id in self.find_symbols(ReferenceType::Variable) {
            let symbol = self.symbols().symbol(id).clone();
            if symbol.name.starts_with("--") {
                let documentation = symbol.value.as_deref().map(limited_string);
                let kind = if documentation.as_deref().is_some_and(is_color_string) {
                    CompletionItemKind::COLOR
                } else {
                    CompletionItemKind::VARIABLE
                };
                self.push(CompletionItem {
                    documentation: documentation.map(Documentation::String),
                    text_edit: edit(range, &symbol.name),
                    ..item(&symbol.name, kind)
                });
            }
            referenced.shift_remove(symbol.name.as_str());
        }
        for name in referenced {
            if name.starts_with("--") {
                self.push(CompletionItem {
                    text_edit: edit(range, name),
                    ..item(name, CompletionItemKind::VARIABLE)
                });
            }
        }
    }

    /// `getUnitProposals`.
    fn unit_proposals(&mut self, entry: &PropertyData, mut existing: Option<NodeId>) {
        let mut number = "0";
        if !self.current_word.is_empty() {
            if let Some(m) = match_number(&self.current_word) {
                self.result.is_incomplete = m.len() == self.current_word.len();
                number = m;
            }
        } else {
            self.result.is_incomplete = true;
        }
        let number = number.to_owned();
        if let Some(e) = existing
            && self
                .ast
                .parent(e)
                .is_some_and(|p| self.ast.node_type(p) == NodeType::Term)
        {
            // Include the unary operator.
            existing = self.ast.get_parent(e);
        }
        let range = self.completion_range(existing);
        for restriction in entry.restrictions.iter().flatten() {
            let units = facts::UNITS.iter().find(|(r, _)| r == restriction);
            for unit in units.map_or(&[][..], |(_, u)| u) {
                let text = format!("{number}{unit}");
                self.push(CompletionItem {
                    text_edit: edit(range, &text),
                    ..item(text, CompletionItemKind::UNIT)
                });
            }
        }
    }

    /// `getColorProposals`.
    fn color_proposals(&mut self, existing: Option<NodeId>) {
        match self.dialect {
            Dialect::Css => {}
            Dialect::Scss => self.scss_function_proposals(scss::COLOR_PROPOSALS, existing, false),
            Dialect::Less => self.less_function_proposals(less::COLOR_PROPOSALS, existing, false),
        }
        let range = self.completion_range(existing);
        for &(color, description) in COLORS {
            self.push(CompletionItem {
                documentation: doc(description),
                text_edit: edit(range, color),
                ..item(color, CompletionItemKind::COLOR)
            });
        }
        for &(color, description) in COLOR_KEYWORDS {
            self.push(CompletionItem {
                documentation: doc(description),
                text_edit: edit(range, color),
                ..item(color, CompletionItemKind::VALUE)
            });
        }
        let ast = self.ast;
        let mut values = IndexSet::<_, foldhash::fast::RandomState>::default();
        ast.accept(self.root, &mut |n| {
            let color = matches!(ast.kind(n), Kind::HexColorValue | Kind::Function)
                && is_color_value(ast, n);
            if color && (self.offset < ast.offset(n) || ast.end(n) < self.offset) {
                values.insert(ast.get_text(n));
            }
            true
        });
        for color in values {
            self.push(CompletionItem {
                text_edit: edit(range, color),
                ..item(color, CompletionItemKind::COLOR)
            });
        }
        for p in COLOR_FUNCTIONS {
            self.push(CompletionItem {
                detail: Some(p.func.into()),
                documentation: doc(p.desc),
                text_edit: edit(range, p.insert_text),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..item(p.label, CompletionItemKind::FUNCTION)
            });
        }
    }

    /// The keyword tables (`getPositionProposals` and others).
    fn keyword_proposals(&mut self, table: &[(&str, &str)], existing: Option<NodeId>) {
        let range = self.completion_range(existing);
        for &(keyword, description) in table {
            self.push(CompletionItem {
                documentation: doc(description),
                text_edit: edit(range, keyword),
                ..item(keyword, CompletionItemKind::VALUE)
            });
        }
    }

    /// The function tables (`getImageProposals` and others).
    fn function_keyword_proposals(&mut self, table: &[(&str, &str)], existing: Option<NodeId>) {
        let range = self.completion_range(existing);
        for &(func, description) in table {
            let insert = move_cursor_inside_parenthesis(func);
            self.push(CompletionItem {
                documentation: doc(description),
                insert_text_format: snippet_if(func != insert),
                text_edit: edit(range, insert),
                ..item(func, CompletionItemKind::FUNCTION)
            });
        }
    }

    /// `collectValues`: the values of the other declarations of the property.
    fn collect_values(&self, declaration: NodeId) -> Vec<String> {
        let ast = self.ast;
        let name = ast.full_property_name(declaration);
        let mut entries = IndexSet::<_, foldhash::fast::RandomState>::default();
        ast.accept(self.root, &mut |n| {
            if ast.kind(n).is_declaration()
                && n != declaration
                && ast.full_property_name(n) == name
                && let Some(value) = ast.field(n, Field::Value)
            {
                ast.accept(value, &mut |v| {
                    if matches!(
                        ast.kind(v),
                        Kind::Identifier | Kind::NumericValue | Kind::HexColorValue
                    ) {
                        entries.insert(ast.get_text(v).to_owned());
                    }
                    true
                });
            }
            true
        });
        entries.into_iter().collect()
    }

    // Structure

    /// `getCompletionsForStylesheet`.
    fn stylesheet(&mut self) {
        let Some(node) = self
            .ast
            .find_first_child_before_offset(self.root, self.offset)
        else {
            return self.top_level();
        };
        match self.ast.kind(node) {
            Kind::RuleSet => self.rule_set(node),
            Kind::Supports => self.supports(node),
            _ => {}
        }
    }

    /// `getCompletionForTopLevel`.
    fn top_level(&mut self) {
        if self.dialect == Dialect::Scss {
            self.result.items.extend(scss::at_directives());
            self.result.items.extend(scss::module_loaders());
        }
        let range = self.completion_range(None);
        for entry in self.data.get_at_directives() {
            self.push(CompletionItem {
                text_edit: edit(range, &entry.name),
                documentation: get_entry_description(&**entry, self.markdown, None)
                    .map(Documentation::MarkupContent),
                tags: tags(is_deprecated(&**entry)),
                ..item(&entry.name, CompletionItemKind::KEYWORD)
            });
        }
        self.selector(None, false);
    }

    /// `getCompletionsForRuleSet`.
    fn rule_set(&mut self, rule_set: NodeId) {
        let ast = self.ast;
        let declarations = ast.field(rule_set, Field::Declarations);
        if let Some(d) = declarations
            && ast.ends_with(d, "}")
            && self.offset >= ast.end(d)
        {
            return self.top_level();
        }
        if declarations.is_none_or(|d| self.offset <= ast.offset(d)) {
            let nested = self.is_nested(rule_set);
            return self.selector(Some(rule_set), nested);
        }
        self.declarations(declarations);
    }

    /// `getCompletionsForSelector`.
    fn selector(&mut self, rule_set: Option<NodeId>, is_nested: bool) {
        if self.dialect == Dialect::Scss {
            self.scss_function_proposals(scss::SELECTOR_FUNCS, None, true);
        }
        let ast = self.ast;
        let existing = self.find_in_node_path(&[
            NodeType::PseudoSelector,
            NodeType::IdentifierSelector,
            NodeType::ClassSelector,
            NodeType::ElementNameSelector,
        ]);
        let before_word = |s: &Self| s.offset - s.current_word.len() as isize - 1;
        if existing.is_none() && self.has_character_at(before_word(self), b':') {
            // After the `:` of a pseudo selector: there is no node for `:` alone.
            self.current_word.insert(0, ':');
            if self.has_character_at(before_word(self), b':') {
                self.current_word.insert(0, ':');
            }
            self.default_replace_range = self.word_range();
        }
        let range = self.completion_range(existing);

        let pseudo = [
            (self.data.get_pseudo_classes(), ":-"),
            (self.data.get_pseudo_elements(), "::-"),
        ];
        for (entries, vendor) in pseudo {
            for entry in entries {
                let insert = move_cursor_inside_parenthesis(&entry.name);
                self.push(CompletionItem {
                    documentation: get_entry_description(&**entry, self.markdown, None)
                        .map(Documentation::MarkupContent),
                    tags: tags(is_deprecated(&**entry)),
                    insert_text_format: snippet_if(entry.name != insert),
                    text_edit: edit(range, insert),
                    sort_text: entry
                        .name
                        .starts_with(vendor)
                        .then(|| sort_texts::VENDOR_PREFIXED.into()),
                    ..item(&entry.name, CompletionItemKind::FUNCTION)
                });
            }
        }
        // HTML tags only at the top level.
        if !is_nested {
            for &tag in facts::HTML5_TAGS.iter().chain(facts::SVG_ELEMENTS) {
                self.push(CompletionItem {
                    text_edit: edit(range, tag),
                    ..item(tag, CompletionItemKind::KEYWORD)
                });
            }
        }

        let mut visited: HashSet<&str> = HashSet::from_iter([self.current_word.as_str()]);
        let mut classes = Vec::new();
        ast.accept(self.root, &mut |n| {
            if ast.node_type(n) == NodeType::SimpleSelector && ast.length(n) > 0 {
                let selector = ast.get_text(n);
                if selector.starts_with('.') && visited.insert(selector) {
                    classes.push(selector);
                }
                return false;
            }
            true
        });
        for class in classes {
            self.push(CompletionItem {
                text_edit: edit(range, class),
                ..item(class, CompletionItemKind::KEYWORD)
            });
        }

        if let Some(rs) = rule_set
            && self.is_nested(rs)
        {
            let mut selectors = ast.list_children(rs, Field::Selectors);
            let first = selectors.clone().rfind(|&c| ast.offset(c) <= self.offset);
            if first.is_some() && first == selectors.next() {
                self.property_proposals(None);
            }
        }
    }

    /// `getCompletionsForDeclarations`.
    fn declarations(&mut self, declarations: Option<NodeId>) {
        let ast = self.ast;
        // Incomplete nodes.
        let Some(declarations) = declarations.filter(|&d| self.offset != ast.offset(d)) else {
            return;
        };
        let Some(node) = ast.find_first_child_before_offset(declarations, self.offset) else {
            return self.declaration_property(None);
        };
        let kind = ast.kind(node);
        if kind.is_abstract_declaration() {
            let semicolon = ast.semicolon_position(node);
            match self.colon_position(node) {
                // Upstream throws for a `VariableDeclaration` here: it has no
                // `getProperty`. Here it has no property.
                Some(colon) if self.offset > colon => {}
                _ => return self.declaration_property(Some(node)),
            }
            if let Some(semicolon) = semicolon
                && semicolon < self.offset
            {
                // No new properties right after the semicolon.
                if self.offset == semicolon + 1 {
                    return;
                }
                return self.declaration_property(None);
            }
            if kind.is_declaration() {
                self.declaration_value(node);
            }
        } else if kind == Kind::ExtendsReference {
            self.extends_reference(None);
        } else if self.current_word.starts_with('@') || kind == Kind::RuleSet {
            self.declaration_property(None);
        }
    }

    /// `getCompletionsForVariableDeclaration`.
    fn variable_declaration(&mut self, declaration: NodeId) {
        if self.offset != 0
            && self
                .colon_position(declaration)
                .is_some_and(|c| self.offset > c)
        {
            self.variable_proposals(self.ast.field(declaration, Field::Value));
        }
    }

    /// `getCompletionsForExpression`.
    fn expression(&mut self, expression: NodeId) {
        let ast = self.ast;
        if let Some(parent) = ast.get_parent(expression)
            && ast.kind(parent) == Kind::FunctionArgument
        {
            return self.function_argument(Some(parent), ast.get_parent(parent));
        }
        let Some(declaration) = ast.find_parent(expression, NodeType::Declaration) else {
            return self.term_proposals(None, None);
        };
        match ast.find_child_at_offset(expression, self.offset, true) {
            None => self.declaration_value(declaration),
            Some(n) if matches!(ast.kind(n), Kind::NumericValue | Kind::Identifier) => {
                self.declaration_value(declaration)
            }
            Some(_) => {}
        }
    }

    /// `getCompletionsForFunctionArgument`. The function can be a mixin
    /// reference; upstream throws for `@content(...)`, which has no identifier.
    fn function_argument(&mut self, arg: Option<NodeId>, func: Option<NodeId>) {
        let ast = self.ast;
        let Some(func) = func else { return };
        if ast
            .field(func, Field::Identifier)
            .is_some_and(|i| ast.matches(i, "var"))
        {
            let first = ast.list_children(func, Field::Arguments).next();
            if first.is_none() || first == arg {
                self.variable_proposals_for_css_var_function();
            }
        }
    }

    /// `getCompletionsForFunctionDeclaration`.
    fn function_declaration(&mut self, declaration: NodeId) {
        let ast = self.ast;
        if let Some(d) = ast.field(declaration, Field::Declarations)
            && self.offset > ast.offset(d)
            && self.offset < ast.end(d)
        {
            self.term_proposals(None, None);
        }
    }

    /// `getCompletionsForMixinReference`.
    fn mixin_reference(&mut self, reference: NodeId) {
        let ast = self.ast;
        for id in self.find_symbols(ReferenceType::Mixin) {
            let node = self.symbols().symbol(id).node;
            if ast.kind(node) == Kind::MixinDeclaration {
                let item = self.term_proposal(id, None);
                self.push(item);
            }
        }
        let context = MixinReferenceCompletionContext {
            mixin_name: self.current_word.clone(),
            range: self.completion_range(ast.field(reference, Field::Identifier)),
        };
        for p in self.participants.iter_mut() {
            p.on_css_mixin_reference(context.clone());
        }
    }

    /// `getTermProposals`.
    fn term_proposals(&mut self, entry: Option<&PropertyData>, existing: Option<NodeId>) {
        let allowed = |ty: Option<&str>| {
            let restrictions = entry.and_then(|e| e.restrictions.as_ref());
            ty.is_none_or(|t| restrictions.is_none_or(|r| r.iter().any(|x| x == t)))
        };
        match self.dialect {
            Dialect::Css => {}
            Dialect::Scss => {
                let funcs: Vec<_> = scss::BUILT_IN_FUNCS
                    .iter()
                    .filter(|f| allowed(f.ty))
                    .copied()
                    .collect();
                self.scss_function_proposals(&funcs, existing, true);
            }
            Dialect::Less => {
                let funcs: Vec<_> = less::BUILT_IN_PROPOSALS
                    .iter()
                    .filter(|f| allowed(f.ty))
                    .copied()
                    .collect();
                self.less_function_proposals(&funcs, existing, true);
            }
        }
        let ast = self.ast;
        for id in self.find_symbols(ReferenceType::Function) {
            if ast.kind(self.symbols().symbol(id).node) == Kind::FunctionDeclaration {
                let item = self.term_proposal(id, existing);
                self.push(item);
            }
        }
    }

    /// `makeTermProposal`.
    fn term_proposal(&mut self, symbol: SymbolId, existing: Option<NodeId>) -> CompletionItem {
        let ast = self.ast;
        let range = self.completion_range(existing);
        let symbol = self.symbols().symbol(symbol);
        let params: Vec<&str> = ast
            .list_children(symbol.node, Field::Parameters)
            .map(|c| {
                if ast.kind(c) == Kind::FunctionParameter {
                    ast.identifier_name(c)
                } else {
                    ast.get_text(c)
                }
            })
            .collect();
        let snippets: Vec<String> = params
            .iter()
            .enumerate()
            .map(|(i, p)| format!("${{{}:{p}}}", i + 1))
            .collect();
        CompletionItem {
            detail: Some(format!("{}({})", symbol.name, params.join(", "))),
            text_edit: edit(range, format!("{}({})", symbol.name, snippets.join(", "))),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            sort_text: Some(sort_texts::TERM.into()),
            ..item(&symbol.name, CompletionItemKind::FUNCTION)
        }
    }

    /// `getCompletionsForSupportsCondition`.
    fn supports_condition(&mut self, condition: NodeId) {
        let ast = self.ast;
        if let Some(child) = ast.find_first_child_before_offset(condition, self.offset) {
            if ast.kind(child).is_declaration() {
                return match self.colon_position(child) {
                    Some(colon) if self.offset > colon => self.declaration_value(child),
                    _ => self.declaration_property(Some(child)),
                };
            }
            if ast.kind(child) == Kind::SupportsCondition {
                return self.supports_condition(child);
            }
        }
        let extra = ast.extra(condition);
        if let Some(l) = extra.and_then(|e| e.l_parent)
            && self.offset > l as isize
            && extra
                .and_then(|e| e.r_parent)
                .is_none_or(|r| self.offset <= r as isize)
        {
            self.declaration_property(None);
        }
    }

    /// `getCompletionsForSupports`.
    fn supports(&mut self, supports: NodeId) {
        let ast = self.ast;
        let declarations = ast.field(supports, Field::Declarations);
        if declarations.is_none_or(|d| self.offset <= ast.offset(d)) {
            if let Some(child) = ast.find_first_child_before_offset(supports, self.offset)
                && ast.kind(child) == Kind::SupportsCondition
            {
                self.supports_condition(child);
            }
            return;
        }
        self.top_level();
    }

    /// `getCompletionsForMediaCondition`.
    fn media_condition(&mut self, condition: NodeId) {
        let ast = self.ast;
        let media = self.data.get_at_directive("@media");
        if let Some(child) = ast.find_first_child_before_offset(condition, self.offset) {
            if ast.kind(child) == Kind::MediaFeature {
                if let Some(name) = ast.child(child, 0)
                    && self.offset > ast.end(name)
                {
                    let name = ast.get_text(name);
                    let descriptor = media
                        .and_then(|m| m.descriptors.as_ref())
                        .and_then(|d| d.iter().find(|d| d.name == name));
                    if let Some(d) = descriptor {
                        return self.value_enum_proposals(d.values.as_deref(), None);
                    }
                }
            } else if ast.kind(child) == Kind::MediaCondition {
                return self.media_condition(child);
            }
        }
        // All media descriptors.
        let range = self.completion_range(None);
        for d in media
            .and_then(|m| m.descriptors.as_ref())
            .into_iter()
            .flatten()
        {
            let discrete = d.ty.as_deref() == Some("discrete");
            let text = if discrete {
                format!("{}: ", d.name)
            } else {
                d.name.clone()
            };
            self.push(CompletionItem {
                text_edit: edit(range, text),
                documentation: get_entry_description(d, self.markdown, None)
                    .map(Documentation::MarkupContent),
                tags: tags(is_deprecated(d)),
                command: (discrete && d.values.is_some()).then(retrigger),
                ..item(&d.name, CompletionItemKind::KEYWORD)
            });
        }
    }

    /// `getCompletionsForExtendsReference`: rule names in SCSS.
    fn extends_reference(&mut self, existing: Option<NodeId>) {
        if self.dialect != Dialect::Scss {
            return;
        }
        let range = self.completion_range(existing);
        for id in self.find_symbols(ReferenceType::Rule) {
            let name = self.symbols().symbol(id).name.clone();
            self.push(CompletionItem {
                text_edit: edit(range, &name),
                ..item(name, CompletionItemKind::FUNCTION)
            });
        }
    }

    /// `getCompletionForUriLiteralValue`.
    fn uri_literal_value(&mut self, node: NodeId) {
        let ast = self.ast;
        let context = match ast.child(node, 0) {
            None => {
                let empty = self.position_at(ast.offset(node) + "url(".len() as isize);
                UriLiteralCompletionContext {
                    uri_value: String::new(),
                    position: self.position,
                    range: Range::new(empty, empty),
                }
            }
            Some(value) => UriLiteralCompletionContext {
                uri_value: ast.get_text(value).to_owned(),
                position: self.position,
                range: self.completion_range(Some(value)),
            },
        };
        for p in self.participants.iter_mut() {
            p.on_css_uri_literal_value(context.clone());
        }
    }

    /// `getCompletionForImportPath`.
    fn import_path(&mut self, node: NodeId) {
        let ast = self.ast;
        let range = self.completion_range(Some(node));
        if self.dialect == Dialect::Scss
            && ast
                .get_parent(node)
                .is_some_and(|p| matches!(ast.node_type(p), NodeType::Forward | NodeType::Use))
        {
            for (label, documentation, url) in scss::MODULE_BUILT_INS {
                self.push(CompletionItem {
                    documentation: Some(scss::with_reference(documentation, url)),
                    text_edit: edit(range, format!("'{label}'")),
                    ..item(*label, CompletionItemKind::MODULE)
                });
            }
        }
        let context = ImportPathCompletionContext {
            path_value: ast.get_text(node).to_owned(),
            position: self.position,
            range,
        };
        for p in self.participants.iter_mut() {
            p.on_css_import_path(context.clone());
        }
    }

    // SCSS and Less functions

    /// `SCSSCompletion.createFunctionProposals`.
    fn scss_function_proposals(
        &mut self,
        proposals: &[scss::FunctionInfo],
        existing: Option<NodeId>,
        sort_to_end: bool,
    ) {
        let range = self.completion_range(existing);
        for p in proposals {
            let label = p.func.find('(').map_or("", |i| &p.func[..i]);
            self.push(CompletionItem {
                detail: Some(p.func.into()),
                documentation: doc(p.desc),
                text_edit: edit(range, scss::insert_text(p.func)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                sort_text: sort_to_end.then(|| "z".into()),
                ..item(label, CompletionItemKind::FUNCTION)
            });
        }
    }

    /// `LESSCompletion.createFunctionProposals`.
    fn less_function_proposals(
        &mut self,
        proposals: &[less::FunctionInfo],
        existing: Option<NodeId>,
        sort_to_end: bool,
    ) {
        let range = self.completion_range(existing);
        for p in proposals {
            self.push(CompletionItem {
                detail: Some(p.example.into()),
                documentation: p.description.and_then(doc),
                text_edit: edit(range, format!("{}($0)", p.name)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                sort_text: sort_to_end.then(|| "z".into()),
                ..item(p.name, CompletionItemKind::FUNCTION)
            });
        }
    }
}

/// `/^-?\d[\.\d+]*/`.
fn match_number(word: &str) -> Option<&str> {
    let b = word.as_bytes();
    let start = usize::from(b.first() == Some(&b'-'));
    if !b.get(start).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let len = b[start + 1..]
        .iter()
        .take_while(|c| c.is_ascii_digit() || matches!(c, b'.' | b'+'))
        .count();
    Some(&word[..start + 1 + len])
}

mod scss {
    use super::*;

    #[derive(Clone, Copy)]
    pub struct FunctionInfo {
        pub func: &'static str,
        pub desc: &'static str,
        pub ty: Option<&'static str>,
    }

    const fn f(func: &'static str, desc: &'static str) -> FunctionInfo {
        FunctionInfo {
            func,
            desc,
            ty: None,
        }
    }

    const VARIABLE_DEFAULTS: &[(&str, &str)] = &[
        ("$red", "1"),
        ("$green", "2"),
        ("$blue", "3"),
        ("$alpha", "1.0"),
        ("$color", "#000000"),
        ("$weight", "0.5"),
        ("$hue", "0"),
        ("$saturation", "0%"),
        ("$lightness", "0%"),
        ("$degrees", "0"),
        ("$amount", "0"),
        ("$string", "\"\""),
        ("$substring", "\"s\""),
        ("$number", "0"),
        ("$limit", "1"),
    ];

    pub const COLOR_PROPOSALS: &[FunctionInfo] = &[
        f("red($color)", "Gets the red component of a color."),
        f("green($color)", "Gets the green component of a color."),
        f("blue($color)", "Gets the blue component of a color."),
        f(
            "mix($color, $color, [$weight])",
            "Mixes two colors together.",
        ),
        f("hue($color)", "Gets the hue component of a color."),
        f(
            "saturation($color)",
            "Gets the saturation component of a color.",
        ),
        f(
            "lightness($color)",
            "Gets the lightness component of a color.",
        ),
        f(
            "adjust-hue($color, $degrees)",
            "Changes the hue of a color.",
        ),
        f("lighten($color, $amount)", "Makes a color lighter."),
        f("darken($color, $amount)", "Makes a color darker."),
        f("saturate($color, $amount)", "Makes a color more saturated."),
        f(
            "desaturate($color, $amount)",
            "Makes a color less saturated.",
        ),
        f("grayscale($color)", "Converts a color to grayscale."),
        f("complement($color)", "Returns the complement of a color."),
        f("invert($color)", "Returns the inverse of a color."),
        f("alpha($color)", "Gets the opacity component of a color."),
        f(
            "opacity($color)",
            "Gets the alpha component (opacity) of a color.",
        ),
        f(
            "rgba($color, $alpha)",
            "Changes the alpha component for a color.",
        ),
        f("opacify($color, $amount)", "Makes a color more opaque."),
        f("fade-in($color, $amount)", "Makes a color more opaque."),
        f(
            "transparentize($color, $amount)",
            "Makes a color more transparent.",
        ),
        f(
            "fade-out($color, $amount)",
            "Makes a color more transparent.",
        ),
        f(
            "adjust-color($color, [$red], [$green], [$blue], [$hue], [$saturation], [$lightness], [$alpha])",
            "Increases or decreases one or more components of a color.",
        ),
        f(
            "scale-color($color, [$red], [$green], [$blue], [$saturation], [$lightness], [$alpha])",
            "Fluidly scales one or more properties of a color.",
        ),
        f(
            "change-color($color, [$red], [$green], [$blue], [$hue], [$saturation], [$lightness], [$alpha])",
            "Changes one or more properties of a color.",
        ),
        f(
            "ie-hex-str($color)",
            "Converts a color into the format understood by IE filters.",
        ),
    ];

    pub const SELECTOR_FUNCS: &[FunctionInfo] = &[
        f(
            "selector-nest($selectors…)",
            "Nests selector beneath one another like they would be nested in the stylesheet.",
        ),
        f(
            "selector-append($selectors…)",
            "Appends selectors to one another without spaces in between.",
        ),
        f(
            "selector-extend($selector, $extendee, $extender)",
            "Extends $extendee with $extender within $selector.",
        ),
        f(
            "selector-replace($selector, $original, $replacement)",
            "Replaces $original with $replacement within $selector.",
        ),
        f(
            "selector-unify($selector1, $selector2)",
            "Unifies two selectors to produce a selector that matches elements matched by both.",
        ),
        f(
            "is-superselector($super, $sub)",
            "Returns whether $super matches all the elements $sub does, and possibly more.",
        ),
        f(
            "simple-selectors($selector)",
            "Returns the simple selectors that comprise a compound selector.",
        ),
        f(
            "selector-parse($selector)",
            "Parses a selector into the format returned by &.",
        ),
    ];

    pub const BUILT_IN_FUNCS: &[FunctionInfo] = &[
        f("unquote($string)", "Removes quotes from a string."),
        f("quote($string)", "Adds quotes to a string."),
        f(
            "str-length($string)",
            "Returns the number of characters in a string.",
        ),
        f(
            "str-insert($string, $insert, $index)",
            "Inserts $insert into $string at $index.",
        ),
        f(
            "str-index($string, $substring)",
            "Returns the index of the first occurance of $substring in $string.",
        ),
        f(
            "str-slice($string, $start-at, [$end-at])",
            "Extracts a substring from $string.",
        ),
        f("to-upper-case($string)", "Converts a string to upper case."),
        f("to-lower-case($string)", "Converts a string to lower case."),
        FunctionInfo {
            func: "percentage($number)",
            desc: "Converts a unitless number to a percentage.",
            ty: Some("percentage"),
        },
        f(
            "round($number)",
            "Rounds a number to the nearest whole number.",
        ),
        f(
            "ceil($number)",
            "Rounds a number up to the next whole number.",
        ),
        f(
            "floor($number)",
            "Rounds a number down to the previous whole number.",
        ),
        f("abs($number)", "Returns the absolute value of a number."),
        f("min($numbers)", "Finds the minimum of several numbers."),
        f("max($numbers)", "Finds the maximum of several numbers."),
        f("random([$limit])", "Returns a random number."),
        f("length($list)", "Returns the length of a list."),
        f("nth($list, $n)", "Returns a specific item in a list."),
        f(
            "set-nth($list, $n, $value)",
            "Replaces the nth item in a list.",
        ),
        f(
            "join($list1, $list2, [$separator])",
            "Joins together two lists into one.",
        ),
        f(
            "append($list1, $val, [$separator])",
            "Appends a single value onto the end of a list.",
        ),
        f(
            "zip($lists)",
            "Combines several lists into a single multidimensional list.",
        ),
        f(
            "index($list, $value)",
            "Returns the position of a value within a list.",
        ),
        f("list-separator(#list)", "Returns the separator of a list."),
        f(
            "map-get($map, $key)",
            "Returns the value in a map associated with a given key.",
        ),
        f(
            "map-merge($map1, $map2)",
            "Merges two maps together into a new map.",
        ),
        f(
            "map-remove($map, $keys)",
            "Returns a new map with keys removed.",
        ),
        f("map-keys($map)", "Returns a list of all keys in a map."),
        f("map-values($map)", "Returns a list of all values in a map."),
        f(
            "map-has-key($map, $key)",
            "Returns whether a map has a value associated with a given key.",
        ),
        f(
            "keywords($args)",
            "Returns the keywords passed to a function that takes variable arguments.",
        ),
        f(
            "feature-exists($feature)",
            "Returns whether a feature exists in the current Sass runtime.",
        ),
        f(
            "variable-exists($name)",
            "Returns whether a variable with the given name exists in the current scope.",
        ),
        f(
            "global-variable-exists($name)",
            "Returns whether a variable with the given name exists in the global scope.",
        ),
        f(
            "function-exists($name)",
            "Returns whether a function with the given name exists.",
        ),
        f(
            "mixin-exists($name)",
            "Returns whether a mixin with the given name exists.",
        ),
        f(
            "inspect($value)",
            "Returns the string representation of a value as it would be represented in Sass.",
        ),
        f("type-of($value)", "Returns the type of a value."),
        f(
            "unit($number)",
            "Returns the unit(s) associated with a number.",
        ),
        f("unitless($number)", "Returns whether a number has units."),
        f(
            "comparable($number1, $number2)",
            "Returns whether two numbers can be added, subtracted, or compared.",
        ),
        f("call($name, $args…)", "Dynamically calls a Sass function."),
    ];

    /// label, documentation, insert text.
    const AT_DIRECTIVES: &[(&str, &str, Option<&str>)] = &[
        ("@extend", "Inherits the styles of another selector.", None),
        (
            "@at-root",
            "Causes one or more rules to be emitted at the root of the document.",
            None,
        ),
        (
            "@debug",
            "Prints the value of an expression to the standard error output stream. Useful for debugging complicated Sass files.",
            None,
        ),
        (
            "@warn",
            "Prints the value of an expression to the standard error output stream. Useful for libraries that need to warn users of deprecations or recovering from minor mixin usage mistakes. Warnings can be turned off with the `--quiet` command-line option or the `:quiet` Sass option.",
            None,
        ),
        (
            "@error",
            "Throws the value of an expression as a fatal error with stack trace. Useful for validating arguments to mixins and functions.",
            None,
        ),
        (
            "@if",
            "Includes the body if the expression does not evaluate to `false` or `null`.",
            Some("@if ${1:expr} {\n\t$0\n}"),
        ),
        (
            "@for",
            "For loop that repeatedly outputs a set of styles for each `$var` in the `from/through` or `from/to` clause.",
            Some("@for \\$${1:var} from ${2:start} ${3|to,through|} ${4:end} {\n\t$0\n}"),
        ),
        (
            "@each",
            "Each loop that sets `$var` to each item in the list or map, then outputs the styles it contains using that value of `$var`.",
            Some("@each \\$${1:var} in ${2:list} {\n\t$0\n}"),
        ),
        (
            "@while",
            "While loop that takes an expression and repeatedly outputs the nested styles until the statement evaluates to `false`.",
            Some("@while ${1:condition} {\n\t$0\n}"),
        ),
        (
            "@mixin",
            "Defines styles that can be re-used throughout the stylesheet with `@include`.",
            Some("@mixin ${1:name} {\n\t$0\n}"),
        ),
        (
            "@include",
            "Includes the styles defined by another mixin into the current rule.",
            None,
        ),
        (
            "@function",
            "Defines complex operations that can be re-used throughout stylesheets.",
            None,
        ),
    ];

    pub fn at_directives() -> impl Iterator<Item = CompletionItem> {
        AT_DIRECTIVES
            .iter()
            .map(|&(label, documentation, insert)| CompletionItem {
                documentation: doc(documentation),
                insert_text: insert.map(Into::into),
                insert_text_format: insert.map(|_| InsertTextFormat::SNIPPET),
                ..item(label, CompletionItemKind::KEYWORD)
            })
    }

    const SASS_DOCUMENTATION: &str = "Sass documentation";

    /// `addReferencesToDocumentation`. Upstream appends the references again
    /// for each new `SCSSCompletion`; here once.
    pub fn with_reference(documentation: &str, url: &str) -> Documentation {
        Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("{documentation}\n\n[{SASS_DOCUMENTATION}]({url})"),
        })
    }

    /// Upstream also sends the `references` of these items, which are not
    /// completion item fields.
    const MODULE_LOADERS: &[(&str, &str, &str, &str)] = &[
        (
            "@use",
            "Loads mixins, functions, and variables from other Sass stylesheets as 'modules', and combines CSS from multiple stylesheets together.",
            "https://sass-lang.com/documentation/at-rules/use",
            "@use $0;",
        ),
        (
            "@forward",
            "Loads a Sass stylesheet and makes its mixins, functions, and variables available when this stylesheet is loaded with the @use rule.",
            "https://sass-lang.com/documentation/at-rules/forward",
            "@forward $0;",
        ),
    ];

    pub fn module_loaders() -> impl Iterator<Item = CompletionItem> {
        MODULE_LOADERS
            .iter()
            .map(|&(label, documentation, url, insert)| CompletionItem {
                documentation: Some(with_reference(documentation, url)),
                insert_text: Some(insert.into()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..item(label, CompletionItemKind::KEYWORD)
            })
    }

    pub const MODULE_BUILT_INS: &[(&str, &str, &str)] = &[
        (
            "sass:math",
            "Provides functions that operate on numbers.",
            "https://sass-lang.com/documentation/modules/math",
        ),
        (
            "sass:string",
            "Makes it easy to combine, search, or split apart strings.",
            "https://sass-lang.com/documentation/modules/string",
        ),
        (
            "sass:color",
            "Generates new colors based on existing ones, making it easy to build color themes.",
            "https://sass-lang.com/documentation/modules/color",
        ),
        (
            "sass:list",
            "Lets you access and modify values in lists.",
            "https://sass-lang.com/documentation/modules/list",
        ),
        (
            "sass:map",
            "Makes it possible to look up the value associated with a key in a map, and much more.",
            "https://sass-lang.com/documentation/modules/map",
        ),
        (
            "sass:selector",
            "Provides access to Sass’s powerful selector engine.",
            "https://sass-lang.com/documentation/modules/selector",
        ),
        (
            "sass:meta",
            "Exposes the details of Sass’s inner workings.",
            "https://sass-lang.com/documentation/modules/meta",
        ),
    ];

    /// `func.replace(/\[?(\$\w+)\]?/g, ...)`: each `$name` (also in `[]`)
    /// becomes `\$name: ${n:default}`.
    pub fn insert_text(func: &str) -> String {
        let b = func.as_bytes();
        let word = |i: usize| {
            b[i..]
                .iter()
                .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
                .count()
        };
        let mut out = String::new();
        let mut tab_stop = 1;
        let (mut i, mut copied) = (0, 0);
        while i < b.len() {
            let start = i;
            let mut j = i + usize::from(b[i] == b'[');
            if b.get(j) == Some(&b'$') && word(j + 1) > 0 {
                let name_end = j + 1 + word(j + 1);
                let name = &func[j..name_end];
                j = name_end + usize::from(b.get(name_end) == Some(&b']'));
                let default = VARIABLE_DEFAULTS
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map_or("", |(_, d)| d);
                out += &func[copied..start];
                out += &format!("\\{name}: ${{{tab_stop}:{default}}}");
                tab_stop += 1;
                copied = j;
                i = j;
            } else {
                i += 1;
            }
        }
        out += &func[copied..];
        out
    }
}

mod less {
    #[derive(Clone, Copy)]
    pub struct FunctionInfo {
        pub name: &'static str,
        pub example: &'static str,
        pub description: Option<&'static str>,
        pub ty: Option<&'static str>,
    }

    const fn f(
        name: &'static str,
        example: &'static str,
        description: &'static str,
    ) -> FunctionInfo {
        FunctionInfo {
            name,
            example,
            description: Some(description),
            ty: None,
        }
    }

    const fn typed(
        name: &'static str,
        example: &'static str,
        description: &'static str,
        ty: &'static str,
    ) -> FunctionInfo {
        FunctionInfo {
            name,
            example,
            description: Some(description),
            ty: Some(ty),
        }
    }

    const fn blend(name: &'static str, example: &'static str) -> FunctionInfo {
        FunctionInfo {
            name,
            example,
            description: None,
            ty: None,
        }
    }

    pub const BUILT_IN_PROPOSALS: &[FunctionInfo] = &[
        f(
            "if",
            "if(condition, trueValue [, falseValue]);",
            "returns one of two values depending on a condition.",
        ),
        f(
            "boolean",
            "boolean(condition);",
            "\"store\" a boolean test for later evaluation in a guard or if().",
        ),
        f(
            "length",
            "length(@list);",
            "returns the number of elements in a value list",
        ),
        f(
            "extract",
            "extract(@list, index);",
            "returns a value at the specified position in the list",
        ),
        f(
            "range",
            "range([start, ] end [, step]);",
            "generate a list spanning a range of values",
        ),
        f(
            "each",
            "each(@list, ruleset);",
            "bind the evaluation of a ruleset to each member of a list.",
        ),
        f("escape", "escape(@string);", "URL encodes a string"),
        f("e", "e(@string);", "escape string content"),
        f(
            "replace",
            "replace(@string, @pattern, @replacement[, @flags]);",
            "string replace",
        ),
        f(
            "unit",
            "unit(@dimension, [@unit: '']);",
            "remove or change the unit of a dimension",
        ),
        typed(
            "color",
            "color(@string);",
            "parses a string to a color",
            "color",
        ),
        f(
            "convert",
            "convert(@value, unit);",
            "converts numbers from one type into another",
        ),
        typed(
            "data-uri",
            "data-uri([mimetype,] url);",
            "inlines a resource and falls back to `url()`",
            "url",
        ),
        f("abs", "abs(number);", "absolute value of a number"),
        f(
            "acos",
            "acos(number);",
            "arccosine - inverse of cosine function",
        ),
        f(
            "asin",
            "asin(number);",
            "arcsine - inverse of sine function",
        ),
        f("ceil", "ceil(@number);", "rounds up to an integer"),
        f("cos", "cos(number);", "cosine function"),
        f("floor", "floor(@number);", "rounds down to an integer"),
        typed(
            "percentage",
            "percentage(@number);",
            "converts to a %, e.g. 0.5 > 50%",
            "percentage",
        ),
        f(
            "round",
            "round(number, [places: 0]);",
            "rounds a number to a number of places",
        ),
        f(
            "sqrt",
            "sqrt(number);",
            "calculates square root of a number",
        ),
        f("sin", "sin(number);", "sine function"),
        f("tan", "tan(number);", "tangent function"),
        f(
            "atan",
            "atan(number);",
            "arctangent - inverse of tangent function",
        ),
        f("pi", "pi();", "returns pi"),
        f(
            "pow",
            "pow(@base, @exponent);",
            "first argument raised to the power of the second argument",
        ),
        f(
            "mod",
            "mod(number, number);",
            "first argument modulus second argument",
        ),
        f(
            "min",
            "min(@x, @y);",
            "returns the lowest of one or more values",
        ),
        f(
            "max",
            "max(@x, @y);",
            "returns the lowest of one or more values",
        ),
    ];

    pub const COLOR_PROPOSALS: &[FunctionInfo] = &[
        f("argb", "argb(@color);", "creates a #AARRGGBB"),
        f(
            "hsl",
            "hsl(@hue, @saturation, @lightness);",
            "creates a color",
        ),
        f(
            "hsla",
            "hsla(@hue, @saturation, @lightness, @alpha);",
            "creates a color",
        ),
        f("hsv", "hsv(@hue, @saturation, @value);", "creates a color"),
        f(
            "hsva",
            "hsva(@hue, @saturation, @value, @alpha);",
            "creates a color",
        ),
        f(
            "hue",
            "hue(@color);",
            "returns the `hue` channel of `@color` in the HSL space",
        ),
        f(
            "saturation",
            "saturation(@color);",
            "returns the `saturation` channel of `@color` in the HSL space",
        ),
        f(
            "lightness",
            "lightness(@color);",
            "returns the `lightness` channel of `@color` in the HSL space",
        ),
        f(
            "hsvhue",
            "hsvhue(@color);",
            "returns the `hue` channel of `@color` in the HSV space",
        ),
        f(
            "hsvsaturation",
            "hsvsaturation(@color);",
            "returns the `saturation` channel of `@color` in the HSV space",
        ),
        f(
            "hsvvalue",
            "hsvvalue(@color);",
            "returns the `value` channel of `@color` in the HSV space",
        ),
        f(
            "red",
            "red(@color);",
            "returns the `red` channel of `@color`",
        ),
        f(
            "green",
            "green(@color);",
            "returns the `green` channel of `@color`",
        ),
        f(
            "blue",
            "blue(@color);",
            "returns the `blue` channel of `@color`",
        ),
        f(
            "alpha",
            "alpha(@color);",
            "returns the `alpha` channel of `@color`",
        ),
        f(
            "luma",
            "luma(@color);",
            "returns the `luma` value (perceptual brightness) of `@color`",
        ),
        f(
            "saturate",
            "saturate(@color, 10%);",
            "return `@color` 10% points more saturated",
        ),
        f(
            "desaturate",
            "desaturate(@color, 10%);",
            "return `@color` 10% points less saturated",
        ),
        f(
            "lighten",
            "lighten(@color, 10%);",
            "return `@color` 10% points lighter",
        ),
        f(
            "darken",
            "darken(@color, 10%);",
            "return `@color` 10% points darker",
        ),
        f(
            "fadein",
            "fadein(@color, 10%);",
            "return `@color` 10% points less transparent",
        ),
        f(
            "fadeout",
            "fadeout(@color, 10%);",
            "return `@color` 10% points more transparent",
        ),
        f(
            "fade",
            "fade(@color, 50%);",
            "return `@color` with 50% transparency",
        ),
        f(
            "spin",
            "spin(@color, 10);",
            "return `@color` with a 10 degree larger in hue",
        ),
        f(
            "mix",
            "mix(@color1, @color2, [@weight: 50%]);",
            "return a mix of `@color1` and `@color2`",
        ),
        f(
            "greyscale",
            "greyscale(@color);",
            "returns a grey, 100% desaturated color",
        ),
        f(
            "contrast",
            "contrast(@color1, [@darkcolor: black], [@lightcolor: white], [@threshold: 43%]);",
            "return `@darkcolor` if `@color1 is> 43% luma` otherwise return `@lightcolor`, see notes",
        ),
        blend("multiply", "multiply(@color1, @color2);"),
        blend("screen", "screen(@color1, @color2);"),
        blend("overlay", "overlay(@color1, @color2);"),
        blend("softlight", "softlight(@color1, @color2);"),
        blend("hardlight", "hardlight(@color1, @color2);"),
        blend("difference", "difference(@color1, @color2);"),
        blend("exclusion", "exclusion(@color1, @color2);"),
        blend("average", "average(@color1, @color2);"),
        blend("negation", "negation(@color1, @color2);"),
    ];
}
