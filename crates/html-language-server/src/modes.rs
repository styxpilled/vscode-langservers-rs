//! Port of `modes/languageModes.ts`, `modes/htmlMode.ts`, `modes/cssMode.ts`,
//! `modes/formatting.ts`, `modes/htmlFolding.ts`, `modes/selectionRanges.ts`
//! and `modes/semanticTokens.ts`. Without the `javascript` feature, script
//! regions have no mode, as with `embeddedLanguages: { javascript: false }`.

use std::cell::{Cell, OnceCell};
use std::rc::Rc;

use css_language_server::fs::RequestService;
use css_language_server::settings::{
    completion_settings, get, hover_settings as css_hover_settings, truthy, validation_settings,
};
use css_languageservice::language_facts::CssDataManager;
use css_languageservice::language_facts::colors::Color;
use css_languageservice::parser::{Dialect, Stylesheet, parse_stylesheet_regions};
use css_languageservice::services::completion::Completion;
use css_languageservice::services::links::DocumentLink;
use css_languageservice::services::navigation::ColorInformation;
use css_languageservice::services::{folding, hover, navigation, selection_range, validation};
use html_languageservice::services::{
    formatter, links, navigation as html_navigation, selection_range as html_selection_range,
};
use html_languageservice::{HtmlDocument, LanguageService};
use lsp_types::{
    ColorPresentation, CompletionItem, CompletionTextEdit, Diagnostic, DocumentHighlight,
    DocumentSymbol, FoldingRange, Hover, Location, Position, Range, SelectionRange,
    SymbolInformation, TextEdit, WorkspaceEdit,
};
use serde_json::Value;
#[cfg(feature = "javascript")]
use serde_json::value::RawValue;
use text_document::TextDocument;

use crate::context::WorkspaceContext;
#[cfg(feature = "javascript")]
use crate::embedded::ScriptFile;
use crate::embedded::{CSS_STYLE_RULE, DocumentRegions, OffsetMap, View};
#[cfg(feature = "javascript")]
use crate::javascript::{Script, ScriptDocs, Scripts};
use crate::settings::{
    completion_configuration, format_configuration, format_settings, hover_settings,
};

pub enum Completions {
    List {
        is_incomplete: bool,
        items: Vec<CompletionItem>,
        /// The `itemDefaults.editRange` of the service.
        edit_range: Option<Range>,
    },
    /// The script modes, whose items resolve through the script service.
    #[cfg(feature = "javascript")]
    Raw(Box<RawValue>),
}

#[cfg(test)]
impl Completions {
    /// The list as upstream sends it.
    pub fn into_value(self) -> Value {
        match self {
            Completions::List {
                is_incomplete,
                items,
                edit_range,
            } => {
                let mut list = serde_json::json!({ "isIncomplete": is_incomplete, "items": items });
                if let Some(range) = edit_range {
                    list["itemDefaults"] = serde_json::json!({ "editRange": range });
                }
                list
            }
            #[cfg(feature = "javascript")]
            Completions::Raw(list) => serde_json::from_str(list.get()).unwrap(),
        }
    }
}

/// The compact embedded CSS document and its style sheet. They depend only on
/// the text of the CSS regions.
struct CssModel {
    document: TextDocument,
    stylesheet: Stylesheet,
}

struct Css {
    model: Rc<CssModel>,
    map: OffsetMap,
}

/// The embedded CSS of a document: positions and ranges go through `view`.
#[derive(Clone, Copy)]
struct CssView<'a> {
    view: View<'a>,
    stylesheet: &'a Stylesheet,
}

impl CssView<'_> {
    fn document(&self) -> &TextDocument {
        self.view.compact
    }

    fn position(&self, position: Position) -> Position {
        self.view.to_compact(position)
    }

    fn range(&self, range: &mut Range) {
        self.view.range(range);
    }

    fn selection_range(&self, range: &mut SelectionRange) {
        self.range(&mut range.range);
        if let Some(parent) = &mut range.parent {
            self.selection_range(parent);
        }
    }

    fn symbol(&self, symbol: &mut DocumentSymbol) {
        self.range(&mut symbol.range);
        self.range(&mut symbol.selection_range);
        for child in symbol.children.iter_mut().flatten() {
            self.symbol(child);
        }
    }
}

/// An open document with the models that `LanguageModelCache` keeps.
pub struct Document {
    pub text: TextDocument,
    html: OnceCell<HtmlDocument>,
    regions: OnceCell<DocumentRegions>,
    css: OnceCell<Css>,
    /// The CSS model of the last version, kept when the CSS text did not change.
    previous_css: Cell<Option<Rc<CssModel>>>,
    #[cfg(feature = "javascript")]
    scripts: [OnceCell<Vec<ScriptFile>>; 2],
}

/// The models of a document that its next version can keep.
#[derive(Default)]
pub struct Kept {
    css: Option<Rc<CssModel>>,
}

impl Document {
    pub fn new(text: TextDocument) -> Self {
        Document::changed(text, Kept::default())
    }

    /// The next version of a document, with the models of the last version.
    pub fn changed(text: TextDocument, kept: Kept) -> Self {
        Document {
            text,
            html: OnceCell::new(),
            regions: OnceCell::new(),
            css: OnceCell::new(),
            previous_css: Cell::new(kept.css),
            #[cfg(feature = "javascript")]
            scripts: Default::default(),
        }
    }

    /// `jsDocuments`: the script files of a script language.
    #[cfg(feature = "javascript")]
    fn script(&self, script: Script) -> ScriptDocs<'_> {
        let files = self.scripts[script as usize]
            .get_or_init(|| self.regions().script_files(&self.text, script.id()));
        ScriptDocs {
            html: &self.text,
            files,
        }
    }

    pub fn regions(&self) -> &DocumentRegions {
        self.regions
            .get_or_init(|| DocumentRegions::new(&self.text))
    }

    /// The address of the CSS model, to check that a version keeps it.
    #[cfg(test)]
    pub fn css_model(&self) -> *const () {
        Rc::as_ptr(&self.css.get().expect("CSS model").model).cast()
    }

    /// The text, and the models that the next version can keep.
    pub fn into_parts(self: Rc<Self>) -> (TextDocument, Kept) {
        let css = match self.css.get() {
            Some(css) => Some(css.model.clone()),
            None => self.previous_css.take(),
        };
        let text = match Rc::try_unwrap(self) {
            Ok(document) => document.text,
            Err(document) => document.text.clone(),
        };
        (text, Kept { css })
    }

    /// The embedded CSS: the CSS regions one after the other, each parsed
    /// alone into one style sheet.
    fn css(&self) -> CssView<'_> {
        let css = self.css.get_or_init(|| {
            let (text, map) = self.regions().compact(self.text.text(), "css");
            let model = match self.previous_css.take() {
                Some(model) if model.document.text() == text => model,
                _ => {
                    let stylesheet = parse_stylesheet_regions(Dialect::Css, &text, &map.regions());
                    let document = TextDocument::new(
                        self.text.uri().clone(),
                        "css",
                        self.text.version(),
                        text,
                    );
                    Rc::new(CssModel {
                        document,
                        stylesheet,
                    })
                }
            };
            Css { model, map }
        });
        CssView {
            view: View {
                html: &self.text,
                compact: &css.model.document,
                map: &css.map,
            },
            stylesheet: &css.model.stylesheet,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Html,
    Css,
    #[cfg(feature = "javascript")]
    Script(Script),
}

/// The `settings` argument of the modes: an object with `css`, `html`,
/// `javascript` and `js/ts`, or `None` (`undefined`).
pub type Settings<'a> = Option<&'a Value>;

/// `LanguageModes`.
pub struct LanguageModes {
    pub html: LanguageService,
    css_enabled: bool,
    css_data: CssDataManager,
    css_completion_markdown: bool,
    css_hover_markdown: bool,
    pub request_service: RequestService,
    #[cfg(feature = "javascript")]
    scripts: Option<Scripts>,
}

/// `getLanguageModes`.
impl LanguageModes {
    pub fn new(
        css_enabled: bool,
        javascript_enabled: bool,
        html: LanguageService,
        css_completion_markdown: bool,
        css_hover_markdown: bool,
        request_service: RequestService,
    ) -> Self {
        let _ = javascript_enabled;
        LanguageModes {
            html,
            css_enabled,
            css_data: CssDataManager::new(true, &[]),
            css_completion_markdown,
            css_hover_markdown,
            request_service,
            #[cfg(feature = "javascript")]
            scripts: javascript_enabled.then(Scripts::default),
        }
    }

    fn mode(&self, language_id: Option<&str>) -> Option<Mode> {
        match language_id? {
            "html" => Some(Mode::Html),
            "css" if self.css_enabled => Some(Mode::Css),
            #[cfg(feature = "javascript")]
            "javascript" if self.scripts.is_some() => Some(Mode::Script(Script::JavaScript)),
            #[cfg(feature = "javascript")]
            "typescript" if self.scripts.is_some() => Some(Mode::Script(Script::TypeScript)),
            _ => None,
        }
    }

    /// The JavaScript or TypeScript mode, with the script files of
    /// `document`.
    #[cfg(feature = "javascript")]
    fn script<'a>(
        &'a self,
        script: Script,
        document: &'a Document,
    ) -> (&'a crate::javascript::JavaScriptMode, ScriptDocs<'a>) {
        let scripts = self.scripts.as_ref().expect("script mode without scripts");
        (scripts.get(script), document.script(script))
    }

    /// `getModeAtPosition`.
    pub fn mode_at_position(&self, document: &Document, position: Position) -> Option<Mode> {
        self.mode(
            document
                .regions()
                .language_at_position(&document.text, position),
        )
    }

    /// `getAllModesInDocument`.
    pub fn modes_in_document(&self, document: &Document) -> Vec<Mode> {
        let ids = document.regions().languages_in_document();
        ids.into_iter()
            .filter_map(|id| self.mode(Some(id)))
            .collect()
    }

    /// `getModesInRange`: the mode and whether it is an attribute value.
    fn modes_in_range(
        &self,
        document: &Document,
        range: Range,
    ) -> Vec<(Range, Option<Mode>, bool)> {
        document
            .regions()
            .language_ranges(&document.text, range)
            .into_iter()
            .map(|r| (r.range, self.mode(r.language_id), r.attribute_value))
            .collect()
    }

    fn html_document<'a>(&self, document: &'a Document) -> &'a HtmlDocument {
        document
            .html
            .get_or_init(|| self.html.parse_html_document(&document.text))
    }

    /// `doValidation` of the CSS mode.
    pub fn css_validation(&self, document: &Document, settings: Settings) -> Vec<Diagnostic> {
        let css = document.css();
        let mut diagnostics = validation::do_validation(
            css.document(),
            css.stylesheet,
            validation_settings(get(settings, "css")).as_ref(),
            &self.css_data,
        );
        for d in &mut diagnostics {
            css.range(&mut d.range);
        }
        diagnostics
    }

    /// `doValidation` of a script mode.
    #[cfg(feature = "javascript")]
    pub fn script_validation(
        &self,
        script: Script,
        document: &Document,
        settings: Settings,
    ) -> Result<Vec<Diagnostic>, String> {
        let (mode, docs) = self.script(script, document);
        mode.validation(docs, settings)
    }

    /// The number of file changes of a script mode.
    #[cfg(all(test, feature = "javascript"))]
    pub fn script_version(&self, script: Script) -> i32 {
        self.scripts.as_ref().unwrap().get(script).version()
    }

    /// `doResolve` of the script modes; other items do not change.
    pub fn resolve(&self, document: &Document, item: Value) -> Result<Value, String> {
        #[cfg(feature = "javascript")]
        if let Some(Mode::Script(script)) =
            self.mode(item.pointer("/data/languageId").and_then(Value::as_str))
        {
            let (mode, js) = self.script(script, document);
            return mode.resolve(js, item);
        }
        let _ = document;
        Ok(item)
    }

    /// `doSignatureHelp`: the script modes only.
    pub fn signature_help(
        &self,
        mode: Mode,
        document: &Document,
        position: Position,
    ) -> Result<Value, String> {
        match mode {
            #[cfg(feature = "javascript")]
            Mode::Script(script) => {
                let (mode, js) = self.script(script, document);
                mode.signature_help(js, position)
            }
            _ => {
                let _ = (document, position);
                Ok(Value::Null)
            }
        }
    }

    /// `doComplete`. `Err` where the JavaScript mode fails.
    pub fn complete(
        &self,
        mode: Mode,
        document: &Document,
        position: Position,
        context: &WorkspaceContext,
        settings: Settings,
    ) -> Result<Completions, String> {
        let list = match mode {
            Mode::Html => {
                let options = completion_configuration(get(settings, "html"), true);
                let list = self.html.do_complete2(
                    &document.text,
                    position,
                    self.html_document(document),
                    context,
                    Some(&self.request_service),
                    Some(&options),
                    &mut [],
                );
                Completions::List {
                    is_incomplete: list.is_incomplete,
                    items: list.items,
                    edit_range: None,
                }
            }
            Mode::Css => {
                let css = document.css();
                let completion = Completion {
                    dialect: Dialect::Css,
                    data: &self.css_data,
                    markdown: self.css_completion_markdown,
                };
                let mut list = completion.do_complete2(
                    css.document(),
                    css.position(position),
                    css.stylesheet,
                    context,
                    Some(&self.request_service),
                    completion_settings(get(get(settings, "css"), "completion")).as_ref(),
                    &mut [],
                );
                for item in &mut list.items {
                    match &mut item.text_edit {
                        Some(CompletionTextEdit::Edit(edit)) => css.range(&mut edit.range),
                        Some(CompletionTextEdit::InsertAndReplace(edit)) => {
                            css.range(&mut edit.insert);
                            css.range(&mut edit.replace);
                        }
                        None => {}
                    }
                    for edit in item.additional_text_edits.iter_mut().flatten() {
                        css.range(&mut edit.range);
                    }
                }
                Completions::List {
                    is_incomplete: list.is_incomplete,
                    items: list.items,
                    edit_range: list.item_defaults.map(|mut d| {
                        css.range(&mut d.edit_range);
                        d.edit_range
                    }),
                }
            }
            #[cfg(feature = "javascript")]
            Mode::Script(script) => {
                let (mode, js) = self.script(script, document);
                return mode
                    .complete(js, document.text.uri().as_str(), position)
                    .map(Completions::Raw);
            }
        };
        Ok(list)
    }

    /// `doHover`. The HTML mode reads `settings`; the CSS mode always reads
    /// `global` (its `doHover.length` is 2, so the server passes no settings).
    pub fn hover(
        &self,
        mode: Mode,
        document: &Document,
        position: Position,
        settings: Settings,
        global: Settings,
    ) -> Result<Option<Hover>, String> {
        Ok(match mode {
            Mode::Html => self.html.do_hover(
                &document.text,
                position,
                self.html_document(document),
                hover_settings(get(settings, "html")),
            ),
            Mode::Css => {
                let css = document.css();
                let mut hover = hover::do_hover(
                    css.document(),
                    css.position(position),
                    css.stylesheet,
                    &self.css_data,
                    self.css_hover_markdown,
                    css_hover_settings(get(get(global, "css"), "hover")).as_ref(),
                );
                if let Some(range) = hover.as_mut().and_then(|h| h.range.as_mut()) {
                    css.range(range);
                }
                hover
            }
            #[cfg(feature = "javascript")]
            Mode::Script(script) => {
                let (mode, js) = self.script(script, document);
                from_json(mode.hover(js, position)?)?
            }
        })
    }

    pub fn document_highlights(
        &self,
        mode: Mode,
        document: &Document,
        position: Position,
    ) -> Result<Vec<DocumentHighlight>, String> {
        Ok(match mode {
            Mode::Html => html_navigation::find_document_highlights(
                &document.text,
                position,
                self.html_document(document),
            ),
            Mode::Css => {
                let css = document.css();
                let mut highlights = navigation::find_document_highlights(
                    css.document(),
                    css.position(position),
                    css.stylesheet,
                );
                for h in &mut highlights {
                    css.range(&mut h.range);
                }
                highlights
            }
            #[cfg(feature = "javascript")]
            Mode::Script(script) => {
                let (mode, js) = self.script(script, document);
                from_json(mode.document_highlights(js, position)?)?
            }
        })
    }

    /// `findDefinition`: the CSS and script modes. `None` for a mode without
    /// definitions.
    pub fn definition(
        &self,
        mode: Mode,
        document: &Document,
        position: Position,
    ) -> Result<Option<Value>, String> {
        match mode {
            Mode::Css => {
                let css = document.css();
                let mut location = navigation::find_definition(
                    css.document(),
                    css.position(position),
                    css.stylesheet,
                );
                if let Some(location) = &mut location {
                    css.range(&mut location.range);
                }
                Ok(Some(serde_json::to_value(location).unwrap()))
            }
            #[cfg(feature = "javascript")]
            Mode::Script(script) => {
                let (mode, js) = self.script(script, document);
                Ok(Some(mode.definition(
                    js,
                    document.text.uri().as_str(),
                    position,
                )?))
            }
            _ => Ok(None),
        }
    }

    /// `findReferences`: the CSS and script modes.
    pub fn references(
        &self,
        mode: Mode,
        document: &Document,
        position: Position,
    ) -> Result<Vec<Location>, String> {
        match mode {
            Mode::Css => {
                let css = document.css();
                let mut locations = navigation::find_references(
                    css.document(),
                    css.position(position),
                    css.stylesheet,
                );
                for location in &mut locations {
                    css.range(&mut location.range);
                }
                Ok(locations)
            }
            #[cfg(feature = "javascript")]
            Mode::Script(script) => {
                let (mode, js) = self.script(script, document);
                let references = mode.references(js, document.text.uri().as_str(), position)?;
                from_json(Value::Array(references))
            }
            _ => Ok(Vec::new()),
        }
    }

    pub fn document_links(
        &self,
        mode: Mode,
        document: &Document,
        context: &WorkspaceContext,
    ) -> Vec<DocumentLink> {
        match mode {
            Mode::Html => links::find_document_links(&document.text, context),
            _ => Vec::new(),
        }
    }

    pub fn document_symbols(
        &self,
        mode: Mode,
        document: &Document,
    ) -> Result<Vec<SymbolInformation>, String> {
        Ok(match mode {
            Mode::Html => {
                html_navigation::find_document_symbols(&document.text, self.html_document(document))
            }
            Mode::Css => {
                let css = document.css();
                navigation::find_symbol_informations(css.document(), css.stylesheet)
                    .into_iter()
                    .filter(|s| s.name != CSS_STYLE_RULE)
                    .map(|mut s| {
                        css.range(&mut s.location.range);
                        s
                    })
                    .collect()
            }
            #[cfg(feature = "javascript")]
            Mode::Script(script) => {
                let (mode, js) = self.script(script, document);
                let symbols = mode.document_symbols(js, document.text.uri().as_str())?;
                from_json(Value::Array(symbols))?
            }
        })
    }

    pub fn document_symbols_nested(
        &self,
        mode: Mode,
        document: &Document,
    ) -> Result<Vec<DocumentSymbol>, String> {
        Ok(match mode {
            Mode::Html => html_navigation::find_document_symbols2(
                &document.text,
                self.html_document(document),
            ),
            Mode::Css => {
                let css = document.css();
                navigation::find_document_symbols(css.document(), css.stylesheet)
                    .into_iter()
                    .flat_map(|s| {
                        if s.name == CSS_STYLE_RULE {
                            s.children.unwrap_or_default()
                        } else {
                            vec![s]
                        }
                    })
                    .map(|mut s| {
                        css.symbol(&mut s);
                        s
                    })
                    .collect()
            }
            #[cfg(feature = "javascript")]
            Mode::Script(_) => self
                .document_symbols(mode, document)?
                .into_iter()
                .map(|s| {
                    #[allow(deprecated)]
                    DocumentSymbol {
                        name: s.name,
                        detail: None,
                        kind: s.kind,
                        tags: s.tags,
                        deprecated: None,
                        range: s.location.range,
                        selection_range: s.location.range,
                        children: None,
                    }
                })
                .collect(),
        })
    }

    pub fn document_colors(&self, mode: Mode, document: &Document) -> Vec<ColorInformation> {
        if mode != Mode::Css {
            return Vec::new();
        }
        let css = document.css();
        let mut colors = navigation::find_document_colors(css.document(), css.stylesheet);
        for c in &mut colors {
            css.range(&mut c.range);
        }
        colors
    }

    pub fn color_presentations(
        &self,
        mode: Mode,
        document: &Document,
        color: Color,
        range: Range,
    ) -> Vec<ColorPresentation> {
        if mode != Mode::Css {
            return Vec::new();
        }
        document.css();
        navigation::get_color_presentations(color, range)
    }

    /// `doAutoInsert` of the HTML mode.
    pub fn auto_insert(
        &self,
        document: &Document,
        position: Position,
        kind: &str,
        settings: Settings,
    ) -> Option<String> {
        let text = &document.text;
        let offset = text.offset_at(position);
        let previous = text.text()[..offset].chars().next_back();
        match kind {
            "autoQuote" if previous == Some('=') => {
                let options = completion_configuration(get(settings, "html"), false);
                self.html.do_quote_complete(
                    text,
                    position,
                    self.html_document(document),
                    Some(&options),
                )
            }
            "autoClose" if matches!(previous, Some('>' | '/')) => {
                self.html
                    .do_tag_complete(text, position, self.html_document(document))
            }
            _ => None,
        }
    }

    /// `doRename`: the HTML and script modes (the CSS mode has none).
    pub fn rename(
        &self,
        mode: Mode,
        document: &Document,
        position: Position,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>, String> {
        match mode {
            Mode::Html => Ok(html_navigation::do_rename(
                &document.text,
                position,
                new_name,
                self.html_document(document),
            )),
            #[cfg(feature = "javascript")]
            Mode::Script(script) => {
                let (mode, js) = self.script(script, document);
                let edit = mode.rename(js, document.text.uri().as_str(), position, new_name)?;
                from_json(edit)
            }
            _ => Ok(None),
        }
    }

    pub fn linked_editing(
        &self,
        mode: Mode,
        document: &Document,
        position: Position,
    ) -> Option<Vec<Range>> {
        (mode == Mode::Html).then(|| {
            html_navigation::find_linked_editing_ranges(
                &document.text,
                position,
                self.html_document(document),
            )
        })?
    }

    /// `getFoldingRanges` of `modes/htmlFolding.ts`.
    pub fn folding_ranges(
        &self,
        document: &Document,
        max_ranges: Option<usize>,
    ) -> Result<Vec<FoldingRange>, String> {
        let text = &document.text;
        let range = Range::new(
            Position::new(0, 0),
            Position::new(text.line_count() as u32, 0),
        );
        let mut result = self.html.get_folding_ranges(text, None);
        let mut ranges_per_mode: Vec<(Mode, Vec<FoldingRange>)> = Vec::new();
        for (mode_range, mode, attribute_value) in self.modes_in_range(document, range) {
            let Some(mode) = mode.filter(|m| *m != Mode::Html && !attribute_value) else {
                continue;
            };
            let index = match ranges_per_mode.iter().position(|(m, _)| *m == mode) {
                Some(index) => index,
                None => {
                    ranges_per_mode.push((mode, self.mode_folding_ranges(mode, document)?));
                    ranges_per_mode.len() - 1
                }
            };
            result.extend(
                ranges_per_mode[index]
                    .1
                    .iter()
                    .filter(|r| {
                        r.start_line >= mode_range.start.line && r.end_line < mode_range.end.line
                    })
                    .cloned(),
            );
        }
        Ok(match max_ranges {
            Some(max) if max > 0 && result.len() > max => limit_ranges(result, max),
            _ => result,
        })
    }

    fn mode_folding_ranges(
        &self,
        mode: Mode,
        document: &Document,
    ) -> Result<Vec<FoldingRange>, String> {
        match mode {
            Mode::Css => {
                let css = document.css();
                let line = |line: u32, character: Option<u32>| {
                    css.view
                        .to_html(Position::new(line, character.unwrap_or(0)))
                };
                Ok(folding::get_folding_ranges(css.document(), None)
                    .into_iter()
                    .map(|mut r| {
                        let start = line(r.start_line, r.start_character);
                        let end = line(r.end_line, r.end_character);
                        r.start_line = start.line;
                        r.end_line = end.line;
                        r.start_character = r.start_character.map(|_| start.character);
                        r.end_character = r.end_character.map(|_| end.character);
                        r
                    })
                    .collect())
            }
            #[cfg(feature = "javascript")]
            Mode::Script(script) => {
                let (mode, js) = self.script(script, document);
                mode.folding_ranges(js)
            }
            Mode::Html => Ok(Vec::new()),
        }
    }

    /// `getSelectionRanges` of `modes/selectionRanges.ts`.
    pub fn selection_ranges(
        &self,
        document: &Document,
        positions: &[Position],
    ) -> Result<Vec<SelectionRange>, String> {
        positions
            .iter()
            .map(|&position| {
                // Upstream parses the document for each position.
                let html_range = html_selection_range::selection_range(
                    position,
                    &document.text,
                    self.html_document(document),
                );
                let mut range = match self.mode_at_position(document, position) {
                    Some(Mode::Html) | None => return Ok(html_range),
                    Some(Mode::Css) => {
                        let css = document.css();
                        let mut range = selection_range::get_selection_ranges(
                            css.document(),
                            &[css.position(position)],
                            css.stylesheet,
                        )
                        .swap_remove(0);
                        css.selection_range(&mut range);
                        range
                    }
                    #[cfg(feature = "javascript")]
                    Some(Mode::Script(script)) => {
                        let (mode, js) = self.script(script, document);
                        from_json(mode.selection_range(js, position)?)?
                    }
                };
                // An embedded range that leaves the HTML range (the root of a
                // compact document of several regions) is dropped.
                let inside = |r: Range| {
                    before_or_same(html_range.range.start, r.start)
                        && before_or_same(r.end, html_range.range.end)
                };
                while !inside(range.range) {
                    match range.parent.take() {
                        Some(parent) => range = *parent,
                        None => return Ok(html_range),
                    }
                }
                let mut top = &mut range;
                while top
                    .parent
                    .as_ref()
                    .is_some_and(|p| inside_range_but_not_same(html_range.range, p.range))
                {
                    top = top.parent.as_mut().unwrap();
                }
                top.parent = Some(Box::new(html_range));
                remove_repeated(&mut range);
                Ok(range)
            })
            .collect()
    }

    /// `format` of a mode other than HTML: the script modes only. Both format
    /// the `javascript` embedded document.
    fn mode_format(
        &self,
        mode: Mode,
        document: &Document,
        range: Range,
        options: &Value,
        settings: Settings,
    ) -> Result<Option<Vec<TextEdit>>, String> {
        match mode {
            #[cfg(feature = "javascript")]
            Mode::Script(script) => {
                let (mode, _) = self.script(script, document);
                let js = document
                    .regions()
                    .embedded_document(&document.text, "javascript", true);
                mode.format(&document.text, &js, range, options, settings)
                    .map(Some)
            }
            _ => {
                let _ = (document, range, options, settings);
                Ok(None)
            }
        }
    }

    /// `format` of `modes/formatting.ts`, with the `format` of the HTML mode.
    /// `Err` where upstream throws.
    pub fn format(
        &self,
        document: &Document,
        mut format_range: Range,
        options: &Value,
        settings: Settings,
    ) -> Result<Vec<TextEdit>, String> {
        let text = &document.text;
        let content = text.text();
        let end_pos = format_range.end;
        let mut end_offset = text.offset_at(end_pos);
        if end_pos.character == 0 && end_pos.line > 0 && end_offset != content.len() {
            // If the selection ends after a line break, exclude that line break.
            let previous_line_start = text.offset_at(Position::new(end_pos.line - 1, 0));
            while end_offset > previous_line_start
                && matches!(content.as_bytes()[end_offset - 1], b'\r' | b'\n')
            {
                end_offset -= 1;
            }
            format_range = Range::new(format_range.start, text.position_at(end_offset));
        }
        let mut result = Vec::new();
        let ranges = self.modes_in_range(document, format_range);
        let mut start = format_range.start;
        let mut i = 0;
        while i < ranges.len() && ranges[i].1 != Some(Mode::Html) {
            let (range, mode, attribute_value) = ranges[i];
            if let Some(mode) = mode.filter(|_| !attribute_value) {
                let edits = self.mode_format(
                    mode,
                    document,
                    Range::new(start, range.end),
                    options,
                    settings,
                )?;
                result.extend(edits.into_iter().flatten());
            }
            start = range.end;
            i += 1;
        }
        if i == ranges.len() {
            return Ok(result);
        }
        format_range.start = start;

        let html_settings = get(settings, "html");
        let config = format_configuration(&format_settings(html_settings, options));
        let mut html_edits = formatter::format(text, Some(format_range), &config);
        let mut formatted = text.apply_edits(&html_edits);
        if truthy(options.get("insertFinalNewline"))
            && end_offset == content.len()
            && !formatted.ends_with('\n')
        {
            formatted.push('\n');
            html_edits.push(TextEdit::new(Range::new(end_pos, end_pos), "\n".to_owned()));
        }

        // The embedded formatters run on the formatted content, so they see
        // the indentation of the HTML formatter.
        let script_enabled = !has_word(unformatted(settings), "script");
        let uri = format!("{}.tmp", text.uri().as_str());
        let new_document = Document::new(TextDocument::new(
            uri.parse().map_err(|e| format!("{e:?}"))?,
            text.language_id(),
            text.version(),
            formatted,
        ));
        let new_text = &new_document.text;
        let after_range = content.len() - text.offset_at(format_range.end);
        let new_end = new_text.position_at(new_text.text().len() - after_range);
        let mut embedded_edits = Vec::new();
        for (range, mode, attribute_value) in
            self.modes_in_range(&new_document, Range::new(format_range.start, new_end))
        {
            let Some(mode) = mode.filter(|m| !attribute_value && enabled(*m, script_enabled))
            else {
                continue;
            };
            if let Some(edits) = self.mode_format(mode, &new_document, range, options, settings)? {
                embedded_edits.extend(edits);
            }
        }
        if embedded_edits.is_empty() {
            result.extend(html_edits);
            return Ok(result);
        }
        let result_content = new_text.apply_edits(&embedded_edits);
        let replace =
            &result_content[text.offset_at(format_range.start)..result_content.len() - after_range];
        result.push(TextEdit::new(format_range, replace.to_owned()));
        Ok(result)
    }

    /// `getSemanticTokens` of the `SemanticTokenProvider`.
    pub fn semantic_tokens(
        &self,
        document: &Document,
        ranges: Option<Vec<Range>>,
    ) -> Result<Vec<u32>, String> {
        let mut tokens = Vec::new();
        for mode in self.modes_in_document(document) {
            #[cfg(feature = "javascript")]
            if let Mode::Script(script) = mode {
                let (mode, js) = self.script(script, document);
                tokens.extend(
                    mode.semantic_tokens(js)?
                        .into_iter()
                        .map(|t| (t.start, t.length, t.type_index, t.modifier_set)),
                );
            }
            let _ = (mode, &mut tokens);
        }
        Ok(encode_tokens(tokens, ranges, &document.text))
    }

    /// The legend of the `SemanticTokenProvider`: the legend of the script
    /// modes.
    pub fn semantic_token_legend(&self) -> Value {
        #[cfg(feature = "javascript")]
        if self.scripts.is_some() {
            return crate::javascript::semantic_token_legend();
        }
        serde_json::json!({ "types": [], "modifiers": [] })
    }

    /// `getTextDocumentContent` of the modes.
    pub fn text_document_content(&self, uri: &str) -> Option<&'static str> {
        #[cfg(feature = "javascript")]
        if self.scripts.is_some() {
            return crate::javascript::text_document_content(uri);
        }
        let _ = uri;
        None
    }

    /// Whether a mode validates (`doValidation`).
    pub fn validates(&self, mode: Mode) -> bool {
        match mode {
            Mode::Css => true,
            #[cfg(feature = "javascript")]
            Mode::Script(_) => true,
            Mode::Html => false,
        }
    }
}

/// `enabledModes` of `onFormat`: only `javascript` is a key that formats.
fn enabled(mode: Mode, script_enabled: bool) -> bool {
    match mode {
        #[cfg(feature = "javascript")]
        Mode::Script(Script::JavaScript) => script_enabled,
        _ => {
            let _ = script_enabled;
            false
        }
    }
}

/// `settings.html.format.unformatted` if it is a string.
fn unformatted<'a>(settings: Settings<'a>) -> &'a str {
    get(get(get(settings, "html"), "format"), "unformatted")
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// `/\bword\b/.test(text)`.
fn has_word(text: &str, word: &str) -> bool {
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    text.match_indices(word).any(|(i, _)| {
        !is_word(text[..i].chars().next_back()) && !is_word(text[i + word.len()..].chars().next())
    })
}

/// `encodeTokens`.
fn encode_tokens(
    mut tokens: Vec<(Position, u32, u32, u32)>,
    ranges: Option<Vec<Range>>,
    document: &TextDocument,
) -> Vec<u32> {
    tokens.sort_by(|a, b| {
        a.0.line
            .cmp(&b.0.line)
            .then(a.0.character.cmp(&b.0.character))
    });
    let ranges = match ranges {
        Some(mut ranges) => {
            ranges.sort_by(|a, b| {
                a.start
                    .line
                    .cmp(&b.start.line)
                    .then(a.start.character.cmp(&b.start.character))
            });
            ranges
        }
        None => vec![Range::new(
            Position::new(0, 0),
            Position::new(document.line_count() as u32, 0),
        )],
    };
    let mut ranges = ranges.into_iter();
    let mut current = ranges.next();
    let (mut previous_line, mut previous_character) = (0, 0);
    let mut result = Vec::new();
    for (start, length, type_index, modifier_set) in tokens {
        let Some(mut range) = current else {
            break;
        };
        while before_or_same(range.end, start) {
            current = ranges.next();
            let Some(next) = current else {
                break;
            };
            range = next;
        }
        if current.is_none() {
            break;
        }
        let end = Position::new(start.line, start.character + length);
        if before_or_same(range.start, start) && before_or_same(end, range.end) {
            if previous_line != start.line {
                previous_character = 0;
            }
            result.extend([
                start.line - previous_line,
                start.character - previous_character,
                length,
                type_index,
                modifier_set,
            ]);
            previous_line = start.line;
            previous_character = start.character;
        }
    }
    result
}

#[cfg(feature = "javascript")]
fn from_json<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn before_or_same(a: Position, b: Position) -> bool {
    a.line < b.line || a.line == b.line && a.character <= b.character
}

/// Removes each parent with the range of its child. The offset map gives a
/// region bound for each range that ends in text between regions, so ranges
/// can repeat.
fn remove_repeated(range: &mut SelectionRange) {
    let mut current = range;
    loop {
        while let Some(parent) = current.parent.take_if(|p| p.range == current.range) {
            current.parent = parent.parent;
        }
        match &mut current.parent {
            Some(parent) => current = parent,
            None => return,
        }
    }
}

fn inside_range_but_not_same(r1: Range, r2: Range) -> bool {
    before_or_same(r1.start, r2.start) && before_or_same(r2.end, r1.end) && r1 != r2
}

/// `limitRanges`.
fn limit_ranges(mut ranges: Vec<FoldingRange>, max_ranges: usize) -> Vec<FoldingRange> {
    ranges.sort_by(|a, b| {
        a.start_line
            .cmp(&b.start_line)
            .then(a.end_line.cmp(&b.end_line))
    });
    let mut top: Option<usize> = None;
    let mut previous: Vec<usize> = Vec::new();
    let mut levels: Vec<Option<usize>> = vec![None; ranges.len()];
    let mut counts: Vec<usize> = Vec::new();
    let mut set_level = |levels: &mut Vec<Option<usize>>, i: usize, level: usize| {
        levels[i] = Some(level);
        if level < 30 {
            if counts.len() <= level {
                counts.resize(level + 1, 0);
            }
            counts[level] += 1;
        }
    };
    for i in 0..ranges.len() {
        let entry = &ranges[i];
        match top {
            None => {
                top = Some(i);
                set_level(&mut levels, i, 0);
            }
            Some(t) => {
                if entry.start_line > ranges[t].start_line {
                    if entry.end_line <= ranges[t].end_line {
                        previous.push(t);
                        top = Some(i);
                        set_level(&mut levels, i, previous.len());
                    } else if entry.start_line > ranges[t].end_line {
                        let mut t = previous.pop();
                        while let Some(p) = t {
                            if entry.start_line > ranges[p].end_line {
                                t = previous.pop();
                            } else {
                                break;
                            }
                        }
                        if let Some(p) = t {
                            previous.push(p);
                        }
                        top = Some(i);
                        set_level(&mut levels, i, previous.len());
                    }
                }
            }
        }
    }
    let mut entries = 0;
    let mut max_level = 0;
    for (i, &n) in counts.iter().enumerate() {
        if n > 0 {
            if n + entries > max_ranges {
                max_level = i;
                break;
            }
            entries += n;
        }
    }
    ranges
        .into_iter()
        .zip(levels)
        .filter_map(|(r, level)| {
            let level = level?;
            let keep = level < max_level
                || level == max_level && {
                    let keep = entries < max_ranges;
                    entries += 1;
                    keep
                };
            keep.then_some(r)
        })
        .collect()
}
