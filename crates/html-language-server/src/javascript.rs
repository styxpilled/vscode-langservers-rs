//! Port of `modes/javascriptMode.ts`, `modes/javascriptSemanticTokens.ts` and
//! `modes/javascriptLibs.ts` on tsrs (TypeScript 7). Upstream uses the
//! TypeScript 5 language service; the results come from tsrs and keep the
//! shape of upstream (`docs/differences/html-server.md`).
//!
//! Each language has one service. Upstream has one file, the embedded
//! document; the branch has one file for each script element and one for the
//! event handler attributes (`embedded::ScriptFile`), and maps the positions.
//! A file whose text did not change stays, so tsrs parses and checks only the
//! changed files.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use css_language_server::settings::{get, truthy};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, FoldingRange, FoldingRangeKind, Position, Range, TextEdit,
};
use serde::Serialize;
use serde_json::value::{RawValue, to_raw_value};
use serde_json::{Value, json};
use text_document::TextDocument;
use tsrs_core::compileroptions::{CompilerOptions, ModuleKind, ModuleResolutionKind, ScriptTarget};
use tsrs_core::context::Context;
use tsrs_core::{P, Tristate};
use tsrs_ls::LanguageService;
use tsrs_ls::format::{FormatContext, format_selection, with_format_code_settings};
use tsrs_ls::lsutil::{self, FormatCodeSettings, IndentStyle, SemicolonPreference};
use tsrs_lsproto as lsproto;
use tsrs_project::{Session, SessionInit, SessionOptions, new_session};
use tsrs_vfs::{bundled, vfstest};

use crate::embedded::{ScriptFile, View};
use crate::modes::Settings;

/// `FILE_PROTOCOL`.
pub const FILE_PROTOCOL: &str = "html-server";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    JavaScript,
    TypeScript,
}

impl Script {
    pub fn id(self) -> &'static str {
        match self {
            Script::JavaScript => "javascript",
            Script::TypeScript => "typescript",
        }
    }
}

/// The JavaScript and TypeScript modes, created at their first use.
#[derive(Default)]
pub struct Scripts {
    javascript: std::cell::OnceCell<JavaScriptMode>,
    typescript: std::cell::OnceCell<JavaScriptMode>,
}

impl Scripts {
    pub fn get(&self, script: Script) -> &JavaScriptMode {
        let cell = match script {
            Script::JavaScript => &self.javascript,
            Script::TypeScript => &self.typescript,
        };
        cell.get_or_init(|| JavaScriptMode::new(script.id()))
    }
}

const ROOT: &str = "/html-server";

/// The libraries of `lib.es2020.full.d.ts`, which tsrs does not load as a
/// `lib` entry.
const LIBS: [&str; 6] = [
    "lib.es2020.d.ts",
    "lib.dom.d.ts",
    "lib.webworker.importscripts.d.ts",
    "lib.scripthost.d.ts",
    "lib.dom.iterable.d.ts",
    "lib.dom.asynciterable.d.ts",
];

/// `ignoredErrors`.
const IGNORED_ERRORS: [i32; 2] = [1108, 2792];

const TOKEN_TYPES: [&str; 12] = [
    "class",
    "enum",
    "interface",
    "namespace",
    "typeParameter",
    "type",
    "parameter",
    "variable",
    "enumMember",
    "property",
    "function",
    "method",
];
const TOKEN_MODIFIERS: [&str; 6] = [
    "declaration",
    "static",
    "async",
    "readonly",
    "defaultLibrary",
    "local",
];

/// `getSemanticTokenLegend`.
pub fn semantic_token_legend() -> Value {
    json!({ "types": TOKEN_TYPES, "modifiers": TOKEN_MODIFIERS })
}

/// A token of `getSemanticTokens`.
pub struct SemanticToken {
    pub start: Position,
    pub length: u32,
    pub type_index: u32,
    pub modifier_set: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompletionList<'a> {
    is_incomplete: bool,
    items: Vec<Item<'a>>,
}

/// A completion item of upstream, with its extra `uri` and `position`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Item<'a> {
    uri: &'a str,
    position: Position,
    label: &'a str,
    sort_text: Option<&'a str>,
    kind: Option<u32>,
    text_edit: ItemEdit<'a>,
    data: &'a ItemData<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemEdit<'a> {
    range: Range,
    new_text: &'a str,
}

/// `CompletionItemData`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemData<'a> {
    language_id: &'a str,
    uri: &'a str,
    offset: usize,
}

/// The compiler options that `updateHostSettings` changes.
#[derive(Clone, Copy, PartialEq, Default)]
struct HostSettings {
    experimental_decorators: bool,
    strict_null_checks: bool,
}

/// `getJavaScriptMode` for `javascript` or `typescript`.
pub struct JavaScriptMode {
    language_id: &'static str,
    session: Arc<Session>,
    ctx: Context,
    extension: &'static str,
    /// The texts of the open script files; file `i` is `uri(i)`.
    files: RefCell<Vec<String>>,
    version: Cell<i32>,
    host_settings: Cell<HostSettings>,
}

fn json<T: lsproto::json::Json>(value: &T) -> Value {
    lsproto::json::marshal(value)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

fn to_ts(position: Position) -> lsproto::Position {
    lsproto::Position {
        line: position.line,
        character: position.character,
    }
}

fn range_at(document: &TextDocument, start: u32, end: u32) -> Range {
    Range::new(
        document.position_at(start as usize),
        document.position_at(end as usize),
    )
}

fn error(e: lsproto::Error) -> String {
    e.to_string()
}

impl JavaScriptMode {
    pub fn new(language_id: &'static str) -> Self {
        let fs: Arc<dyn tsrs_vfs::FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(
            std::iter::empty::<(String, &str)>(),
            true,
        )));
        let mut caps = lsproto::ResolvedClientCapabilities::default();
        caps.text_document.hover.content_format = vec![lsproto::MarkupKind::MARKDOWN];
        caps.text_document.semantic_tokens.token_types =
            TOKEN_TYPES.iter().map(|t| t.to_string()).collect();
        caps.text_document.semantic_tokens.token_modifiers =
            TOKEN_MODIFIERS.iter().map(|t| t.to_string()).collect();
        let ctx = lsproto::with_client_capabilities(&Context::background(), Arc::new(caps));
        let session = new_session(&SessionInit {
            background_ctx: ctx.clone(),
            options: Arc::new(SessionOptions {
                current_directory: ROOT.to_owned(),
                default_library_path: bundled::lib_path(),
                position_encoding: lsproto::PositionEncodingKind::UTF16,
                ..Default::default()
            }),
            fs,
            client: None,
            logger: None,
            npm_executor: None,
            spawner: None,
            content_mapper_logger: None,
            parse_cache: None,
            content_mapped_parse_cache: None,
        });
        let mut preferences = lsutil::new_default_user_preferences();
        // `includeExternalModuleExports: false, includeInsertTextCompletions: false`.
        preferences.include_completions_for_module_exports = Tristate::False;
        preferences.include_completions_for_import_statements = Tristate::False;
        preferences.include_automatic_optional_chain_completions = Tristate::False;
        preferences.include_completions_with_class_member_snippets = Tristate::False;
        preferences.include_completions_with_object_literal_method_snippets = Tristate::False;
        session.initialize_with_user_config(preferences);
        let extension = if language_id == "javascript" {
            "js"
        } else {
            "ts"
        };
        let mode = JavaScriptMode {
            language_id,
            session,
            ctx,
            extension,
            files: RefCell::default(),
            version: Cell::new(0),
            host_settings: Cell::new(HostSettings::default()),
        };
        mode.set_compiler_options(HostSettings::default());
        mode
    }

    fn set_compiler_options(&self, settings: HostSettings) {
        let tristate = |b: bool| if b { Tristate::True } else { Tristate::False };
        let options = CompilerOptions {
            allow_js: Tristate::True,
            allow_non_ts_extensions: Tristate::True,
            lib: Some(LIBS.iter().map(|s| s.to_string()).collect()),
            target: ScriptTarget::LATEST_STANDARD,
            module: ModuleKind::ES_NEXT,
            // Classic resolution is not in TypeScript 7.
            module_resolution: ModuleResolutionKind::BUNDLER,
            experimental_decorators: tristate(settings.experimental_decorators),
            strict_null_checks: tristate(settings.strict_null_checks),
            ..Default::default()
        };
        self.session
            .did_change_compiler_options_for_inferred_projects(&self.ctx, Some(P::new(options)));
        self.host_settings.set(settings);
    }

    /// `updateHostSettings`. `Err` where upstream throws.
    fn update_host_settings(&self, settings: Settings) -> Result<(), String> {
        let js_ts = get(settings, "js/ts").filter(|v| !v.is_null());
        let config = js_ts.and_then(|v| v.get("implicitProjectConfig"));
        let config = config.filter(|v| !v.is_null());
        if js_ts.is_some() && config.is_none() {
            let value = if js_ts.and_then(|v| v.get("implicitProjectConfig")).is_some() {
                "null"
            } else {
                "undefined"
            };
            return Err(format!(
                "Cannot read properties of {value} (reading 'strictNullChecks')"
            ));
        }
        let new = HostSettings {
            experimental_decorators: truthy(config.and_then(|c| c.get("experimentalDecorators"))),
            strict_null_checks: truthy(config.and_then(|c| c.get("strictNullChecks"))),
        };
        if new != self.host_settings.get() {
            self.set_compiler_options(new);
        }
        Ok(())
    }

    fn uri(&self, i: usize) -> lsproto::DocumentUri {
        lsproto::DocumentUri(format!(
            "file://{ROOT}/{}-{i}.{}",
            self.language_id, self.extension
        ))
    }

    /// The index of a script file URI.
    fn file_index(&self, uri: &str) -> Option<usize> {
        let prefix = format!("file://{ROOT}/{}-", self.language_id);
        uri.strip_prefix(&prefix)?
            .strip_suffix(self.extension)?
            .strip_suffix('.')?
            .parse()
            .ok()
    }

    #[cfg(test)]
    pub fn version(&self) -> i32 {
        self.version.get()
    }

    fn next_version(&self) -> i32 {
        self.version.set(self.version.get() + 1);
        self.version.get()
    }

    fn open(&self, uri: lsproto::DocumentUri, text: &str) {
        let kind = if self.language_id == "javascript" {
            lsproto::LanguageKind::JAVA_SCRIPT
        } else {
            lsproto::LanguageKind::TYPE_SCRIPT
        };
        let version = self.next_version();
        self.session
            .did_open_file(&self.ctx, uri, version, text.to_owned(), kind);
    }

    /// Makes the script files of `docs` the open files. A file whose text did
    /// not change stays, so tsrs does not parse and check it again.
    fn sync(&self, docs: ScriptDocs) {
        let mut open = self.files.borrow_mut();
        for (i, file) in docs.files.iter().enumerate() {
            let text = file.document.text();
            match open.get(i) {
                Some(current) if current == text => {}
                Some(_) => {
                    let version = self.next_version();
                    self.session.did_change_file(
                        &self.ctx,
                        self.uri(i),
                        version,
                        vec![lsproto::TextDocumentContentChangePartialOrWholeDocument {
                            whole_document: Some(lsproto::TextDocumentContentChangeWholeDocument {
                                text: text.to_owned(),
                            }),
                            ..Default::default()
                        }],
                    );
                    open[i] = text.to_owned();
                }
                None => {
                    self.open(self.uri(i), text);
                    open.push(text.to_owned());
                }
            }
        }
        while open.len() > docs.files.len() {
            open.pop();
            self.session.did_close_file(&self.ctx, self.uri(open.len()));
        }
    }

    /// `host.getLanguageService` for script file `i` of `docs`.
    fn service(&self, docs: ScriptDocs, i: usize) -> Result<Arc<LanguageService>, String> {
        self.sync(docs);
        self.session
            .get_language_service(&self.ctx, &self.uri(i))
            .map_err(error)
    }

    /// `doValidation` of all script files.
    pub fn validation(
        &self,
        docs: ScriptDocs,
        settings: Settings,
    ) -> Result<Vec<Diagnostic>, String> {
        self.update_host_settings(settings)?;
        let mut result = Vec::new();
        if docs.files.is_empty() {
            return Ok(result);
        }
        let service = self.service(docs, 0)?;
        let program = service.get_program();
        for (i, js) in docs.files.iter().enumerate() {
            let file = program
                .get_source_file(&self.uri(i).file_name())
                .ok_or("no source file")?;
            let diagnostics = program
                .get_syntactic_diagnostics(&self.ctx, Some(file))
                .into_iter()
                .chain(program.get_semantic_diagnostics(&self.ctx, Some(file)));
            for d in diagnostics.filter(|d| !IGNORED_ERRORS.contains(&d.code())) {
                let mut message = Vec::new();
                tsrs_compiler::diagnosticwriter::write_flattened_diagnostic_message(
                    &mut message,
                    d,
                    "\n",
                );
                let mut range = range_at(&js.document, d.pos().unwrap_or(0), d.end().unwrap_or(0));
                docs.view(i).range(&mut range);
                result.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some(self.language_id.to_owned()),
                    message: String::from_utf8_lossy(&message).into_owned(),
                    ..Default::default()
                });
            }
        }
        Ok(result)
    }

    /// `doComplete`.
    pub fn complete(
        &self,
        docs: ScriptDocs,
        uri: &str,
        position: Position,
    ) -> Result<Box<RawValue>, String> {
        let mut list = CompletionList {
            is_incomplete: false,
            items: Vec::new(),
        };
        let Some((i, p)) = docs.locate(position) else {
            return to_raw_value(&list).map_err(|e| e.to_string());
        };
        let service = self.service(docs, i)?;
        let response = service
            .provide_completion(&self.ctx, &self.uri(i), to_ts(p), None)
            .map_err(error)?;
        let items = response.list.map(|l| l.items).unwrap_or_default();
        let js = &docs.files[i].document;
        let (start, end) = word_at_text(js.text(), js.offset_at(p));
        let mut range = range_at(js, start as u32, end as u32);
        docs.view(i).range(&mut range);
        let data = ItemData {
            language_id: self.language_id,
            uri,
            offset: docs.html.offset_at(position),
        };
        list.items = items
            .iter()
            .filter(|item| {
                // Upstream does not ask for entries with an insert text.
                let insert = item.insert_text.as_deref().or(item
                    .text_edit
                    .as_ref()
                    .and_then(|e| e.text_edit.as_ref())
                    .map(|e| e.new_text.as_str()));
                insert.is_none_or(|t| t == item.label)
            })
            .map(|item| Item {
                uri,
                position,
                label: &item.label,
                sort_text: item.sort_text.as_deref(),
                kind: item.kind.map(|k| k.0),
                text_edit: ItemEdit {
                    range,
                    new_text: &item.label,
                },
                data: &data,
            })
            .collect();
        to_raw_value(&list).map_err(|e| e.to_string())
    }

    /// `doResolve`. The item data has the offset in the HTML document.
    pub fn resolve(&self, docs: ScriptDocs, mut item: Value) -> Result<Value, String> {
        let (Some(label), Some(offset)) = (
            item.get("label").and_then(Value::as_str),
            item.pointer("/data/offset").and_then(Value::as_u64),
        ) else {
            return Ok(item);
        };
        // An offset outside the scripts (an item of an older version) resolves
        // in the first file: tsrs finds global names at each position.
        let offset = offset as usize;
        let Some((i, offset)) = docs.locate_offset(offset).or_else(|| {
            let file = docs.files.first()?;
            Some((0, file.map.to_compact(offset)))
        }) else {
            return Ok(item);
        };
        let service = self.service(docs, i)?;
        let data = lsproto::CompletionItemData {
            file_name: self.uri(i).file_name(),
            position: offset as i32,
            name: label.to_owned(),
            ..Default::default()
        };
        let resolved = service.resolve_completion_item(
            &self.ctx,
            lsproto::CompletionItem {
                label: label.to_owned(),
                ..Default::default()
            },
            Some(&data),
        );
        if let Ok(resolved) = resolved {
            let resolved = json(&resolved);
            let text = |v: Option<&Value>| match v {
                Some(Value::String(s)) => s.clone(),
                Some(v) => v
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                None => String::new(),
            };
            item["detail"] = json!(text(resolved.get("detail")));
            item["documentation"] = json!(text(resolved.get("documentation")));
            item.as_object_mut().unwrap().remove("data");
        }
        Ok(item)
    }

    pub fn hover(&self, docs: ScriptDocs, position: Position) -> Result<Value, String> {
        let Some((i, p)) = docs.locate(position) else {
            return Ok(Value::Null);
        };
        let service = self.service(docs, i)?;
        let params = lsproto::HoverParams {
            text_document: lsproto::TextDocumentIdentifier { uri: self.uri(i) },
            position: to_ts(p),
            ..Default::default()
        };
        let hover = service.provide_hover(&self.ctx, &params).map_err(error)?;
        let mut hover = hover.hover.map_or(Value::Null, |h| json(&h));
        if let Some(range) = hover.get_mut("range") {
            map_range(docs.view(i), range);
        }
        Ok(hover)
    }

    pub fn signature_help(&self, docs: ScriptDocs, position: Position) -> Result<Value, String> {
        let Some((i, p)) = docs.locate(position) else {
            return Ok(Value::Null);
        };
        let service = self.service(docs, i)?;
        let help = service
            .provide_signature_help(&self.ctx, &self.uri(i), to_ts(p), None)
            .map_err(error)?;
        Ok(json(&help))
    }

    pub fn rename(
        &self,
        docs: ScriptDocs,
        uri: &str,
        position: Position,
        new_name: &str,
    ) -> Result<Value, String> {
        let Some((i, p)) = docs.locate(position) else {
            return Ok(Value::Null);
        };
        let service = self.service(docs, i)?;
        let mut params = lsproto::RenameParams::default();
        params.text_document.uri = self.uri(i);
        params.position = to_ts(p);
        params.new_name = new_name.to_owned();
        let Ok(edit) = service.provide_rename(&self.ctx, &params, None) else {
            return Ok(Value::Null);
        };
        let edit = json(&edit);
        let Some(changes) = edit.get("changes").and_then(Value::as_object) else {
            return Ok(Value::Null);
        };
        let mut edits = Vec::new();
        for (file, file_edits) in changes {
            let Some(j) = self.file_index(file) else {
                continue;
            };
            for mut e in file_edits.as_array().cloned().unwrap_or_default() {
                map_range(docs.view(j), &mut e["range"]);
                edits.push(e);
            }
        }
        Ok(json!({ "changes": { uri: edits } }))
    }

    pub fn document_highlights(
        &self,
        docs: ScriptDocs,
        position: Position,
    ) -> Result<Value, String> {
        let Some((i, p)) = docs.locate(position) else {
            return Ok(json!([]));
        };
        let service = self.service(docs, i)?;
        let files: Vec<_> = (0..docs.files.len()).map(|j| self.uri(j)).collect();
        let result = service
            .provide_multi_document_highlights(&self.ctx, &self.uri(i), to_ts(p), &files)
            .map_err(error)?;
        let mut highlights = Vec::new();
        for file in result.multi_document_highlights.into_iter().flatten() {
            let Some(j) = self.file_index(&file.uri.0) else {
                continue;
            };
            for h in &file.highlights {
                let mut h = json(h);
                map_range(docs.view(j), &mut h["range"]);
                // Upstream has `Write` or `Text`.
                if h["kind"] != json!(3) {
                    h["kind"] = json!(1);
                }
                highlights.push(h);
            }
        }
        Ok(Value::Array(highlights))
    }

    /// The symbols of all script files.
    pub fn document_symbols(&self, docs: ScriptDocs, uri: &str) -> Result<Vec<Value>, String> {
        let mut result = Vec::new();
        for i in 0..docs.files.len() {
            let service = self.service(docs, i)?;
            let symbols = service
                .provide_document_symbols(&self.ctx, &self.uri(i))
                .map_err(error)?;
            let mut symbols = json(&symbols);
            for mut s in symbols
                .as_array_mut()
                .map(std::mem::take)
                .unwrap_or_default()
            {
                s["location"]["uri"] = json!(uri);
                map_range(docs.view(i), &mut s["location"]["range"]);
                result.push(s);
            }
        }
        Ok(result)
    }

    fn map_location(&self, docs: ScriptDocs, uri: &str, mut location: Value) -> Option<Value> {
        let file = location["uri"].as_str()?.to_owned();
        if let Some(i) = self.file_index(&file) {
            location["uri"] = json!(uri);
            map_range(docs.view(i), &mut location["range"]);
        } else {
            let name = file.rsplit('/').next()?;
            library(name)?;
            location["uri"] = json!(lib_uri(self.language_id, name));
        }
        Some(location)
    }

    pub fn definition(
        &self,
        docs: ScriptDocs,
        uri: &str,
        position: Position,
    ) -> Result<Value, String> {
        let Some((i, p)) = docs.locate(position) else {
            return Ok(Value::Null);
        };
        let service = self.service(docs, i)?;
        let definition = service
            .provide_definition(&self.ctx, &self.uri(i), to_ts(p))
            .map_err(error)?;
        let locations = match json(&definition) {
            Value::Null => return Ok(Value::Null),
            Value::Array(a) => a,
            location => vec![location],
        };
        Ok(Value::Array(
            locations
                .into_iter()
                .filter_map(|l| self.map_location(docs, uri, l))
                .collect(),
        ))
    }

    pub fn references(
        &self,
        docs: ScriptDocs,
        uri: &str,
        position: Position,
    ) -> Result<Vec<Value>, String> {
        let Some((i, p)) = docs.locate(position) else {
            return Ok(Vec::new());
        };
        let service = self.service(docs, i)?;
        let params = lsproto::ReferenceParams {
            text_document: lsproto::TextDocumentIdentifier { uri: self.uri(i) },
            position: to_ts(p),
            context: lsproto::ReferenceContext {
                include_declaration: true,
            },
            ..Default::default()
        };
        let references = service
            .provide_references(&self.ctx, &params, None)
            .map_err(error)?;
        let references = json(&references);
        Ok(references
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| {
                let j = self.file_index(l["uri"].as_str()?)?;
                let mut range = l["range"].clone();
                map_range(docs.view(j), &mut range);
                Some(json!({ "uri": uri, "range": range }))
            })
            .collect())
    }

    pub fn selection_range(&self, docs: ScriptDocs, position: Position) -> Result<Value, String> {
        let Some((i, p)) = docs.locate(position) else {
            return Ok(Value::Null);
        };
        let service = self.service(docs, i)?;
        let mut params = lsproto::SelectionRangeParams::default();
        params.text_document.uri = self.uri(i);
        params.positions = vec![to_ts(p)];
        let ranges = service
            .provide_selection_ranges(&self.ctx, &params)
            .map_err(error)?;
        let mut range = json(&ranges).get(0).cloned().unwrap_or(Value::Null);
        let mut current = &mut range;
        while current.is_object() {
            map_range(docs.view(i), &mut current["range"]);
            current = &mut current["parent"];
        }
        Ok(range)
    }

    /// `format` of the mode: `js` is the embedded document without the
    /// attribute values, with the lines and columns of `document`. It is a
    /// file of its own while it formats.
    pub fn format(
        &self,
        document: &TextDocument,
        js: &TextDocument,
        range: Range,
        options: &Value,
        settings: Settings,
    ) -> Result<Vec<TextEdit>, String> {
        let uri = lsproto::DocumentUri(format!(
            "file://{ROOT}/{}-format.{}",
            self.language_id, self.extension
        ));
        self.open(uri.clone(), js.text());
        let result = self.format_file(&uri, document, js, range, options, settings);
        self.session.did_close_file(&self.ctx, uri);
        result
    }

    fn format_file(
        &self,
        uri: &lsproto::DocumentUri,
        document: &TextDocument,
        js: &TextDocument,
        range: Range,
        options: &Value,
        settings: Settings,
    ) -> Result<Vec<TextEdit>, String> {
        let service = self
            .session
            .get_language_service(&self.ctx, uri)
            .map_err(error)?;
        let file = service
            .get_program()
            .get_source_file(&uri.file_name())
            .ok_or("no source file")?;
        let format_settings = get(get(settings, "javascript"), "format");
        let tab_size = options.get("tabSize").and_then(Value::as_u64).unwrap_or(0) as u32;
        let insert_spaces = truthy(options.get("insertSpaces"));
        let initial_indent_level = compute_initial_indent(document, range, tab_size);
        let settings = convert_options(
            insert_spaces,
            tab_size,
            format_settings,
            initial_indent_level + 1,
        );
        let start = js.offset_at(range.start) as u32;
        let mut end = js.offset_at(range.end) as u32;
        let mut last_line_range = None;
        if range.end.line > range.start.line {
            let line_start = js.offset_at(Position::new(range.end.line, 0));
            let before_end = &js.text()[line_start..end as usize];
            if range.end.character == 0 || before_end.chars().all(char::is_whitespace) {
                end = line_start as u32;
                last_line_range = Some(Range::new(Position::new(range.end.line, 0), range.end));
            }
        }
        let ctx = with_format_code_settings(&FormatContext::default(), settings, "\n");
        let mut result: Vec<TextEdit> = format_selection(&ctx, file, start, end)
            .into_iter()
            .filter(|c| c.pos() >= start && c.end() <= end)
            .map(|c| TextEdit::new(range_at(js, c.pos(), c.end()), c.new_text))
            .collect();
        if let Some(range) = last_line_range {
            result.push(TextEdit::new(
                range,
                generate_indent(initial_indent_level, insert_spaces, tab_size),
            ));
        }
        Ok(result)
    }

    /// `getFoldingRanges` of the mode.
    pub fn folding_ranges(&self, docs: ScriptDocs) -> Result<Vec<FoldingRange>, String> {
        let mut result = Vec::new();
        for i in 0..docs.files.len() {
            let service = self.service(docs, i)?;
            let ranges = service
                .provide_folding_range(&self.ctx, &self.uri(i))
                .map_err(error)?;
            let ranges: Vec<FoldingRange> =
                serde_json::from_value(json(&ranges)).unwrap_or_default();
            let js = &docs.files[i].document;
            let view = docs.view(i);
            for r in ranges {
                let start = Position::new(r.start_line, r.start_character.unwrap_or(0));
                let end = view.to_html(Position::new(r.end_line, r.end_character.unwrap_or(0)));
                let kind = folding_kind(&js.text()[js.offset_at(start)..]);
                let start = view.to_html(start);
                if start.line < end.line {
                    result.push(FoldingRange {
                        kind,
                        start_line: start.line,
                        end_line: end.line,
                        ..Default::default()
                    });
                }
            }
        }
        Ok(result)
    }

    /// `getSemanticTokens`, in the legend of `semantic_token_legend`.
    pub fn semantic_tokens(&self, docs: ScriptDocs) -> Result<Vec<SemanticToken>, String> {
        let legend = tsrs_ls::semantic_tokens_legend(
            &lsproto::get_client_capabilities(&self.ctx)
                .text_document
                .semantic_tokens,
        );
        let type_map: Vec<Option<u32>> = legend
            .token_types
            .iter()
            .map(|t| TOKEN_TYPES.iter().position(|u| u == t).map(|i| i as u32))
            .collect();
        let modifier_map: Vec<Option<u32>> = legend
            .token_modifiers
            .iter()
            .map(|m| {
                TOKEN_MODIFIERS
                    .iter()
                    .position(|u| u == m)
                    .map(|i| i as u32)
            })
            .collect();
        let mut result = Vec::new();
        for i in 0..docs.files.len() {
            let service = self.service(docs, i)?;
            let tokens = service
                .provide_semantic_tokens(&self.ctx, &self.uri(i))
                .map_err(error)?;
            let data = tokens.semantic_tokens.map(|t| t.data).unwrap_or_default();
            let (mut line, mut character) = (0, 0);
            for t in data.as_chunks::<5>().0 {
                if t[0] > 0 {
                    line += t[0];
                    character = 0;
                }
                character += t[1];
                let Some(Some(type_index)) = type_map.get(t[3] as usize) else {
                    continue;
                };
                let mut modifier_set = 0;
                for (bit, m) in modifier_map.iter().enumerate() {
                    if t[4] & (1 << bit) != 0
                        && let Some(m) = m
                    {
                        modifier_set |= 1 << m;
                    }
                }
                result.push(SemanticToken {
                    start: docs.view(i).to_html(Position::new(line, character)),
                    length: t[2],
                    type_index: *type_index,
                    modifier_set,
                });
            }
        }
        Ok(result)
    }
}

/// The script files of an HTML document.
#[derive(Clone, Copy)]
pub struct ScriptDocs<'a> {
    pub html: &'a TextDocument,
    pub files: &'a [ScriptFile],
}

impl<'a> ScriptDocs<'a> {
    fn view(self, i: usize) -> View<'a> {
        View {
            html: self.html,
            compact: &self.files[i].document,
            map: &self.files[i].map,
        }
    }

    /// The file of an HTML offset, and the offset in the file.
    fn locate_offset(self, offset: usize) -> Option<(usize, usize)> {
        let i = self.files.iter().position(|f| f.map.contains(offset))?;
        Some((i, self.files[i].map.to_compact(offset)))
    }

    fn locate(self, position: Position) -> Option<(usize, Position)> {
        let (i, offset) = self.locate_offset(self.html.offset_at(position))?;
        Some((i, self.files[i].document.position_at(offset)))
    }
}

/// Maps a JSON range of a script file to the HTML document.
fn map_range(view: View, value: &mut Value) {
    if let Ok(mut range) = serde_json::from_value::<Range>(value.clone()) {
        view.range(&mut range);
        *value = json!(range);
    }
}

/// `libParentUri` + `fileName`.
fn lib_uri(language_id: &str, file_name: &str) -> String {
    format!("{FILE_PROTOCOL}://{language_id}/libs/{file_name}")
}

/// `loadLibrary`.
fn library(name: &str) -> Option<&'static str> {
    bundled::embedded_file(&format!("{}/{name}", bundled::lib_path()))
}

/// `getTextDocumentContent` of the JavaScript and TypeScript modes.
pub fn text_document_content(uri: &str) -> Option<&'static str> {
    ["javascript", "typescript"]
        .into_iter()
        .find_map(|id| library(uri.strip_prefix(&lib_uri(id, ""))?))
}

impl Drop for JavaScriptMode {
    fn drop(&mut self) {
        self.session.close();
    }
}

/// `getWordAtText` with `JS_WORD_REGEX`: the first word that ends at or
/// after `offset` if it starts at or before it. Byte offsets.
fn word_at_text(text: &str, offset: usize) -> (usize, usize) {
    let line_start = text[..offset].rfind(['\r', '\n']).map_or(0, |i| i + 1);
    let line = &text[line_start..];
    let line = &line[..line.find(['\r', '\n']).unwrap_or(line.len())];
    let in_line = offset - line_start;
    match js_words(line).find(|(start, word)| start + word.len() >= in_line) {
        Some((start, word)) if start <= in_line => {
            (line_start + start, line_start + start + word.len())
        }
        _ => (offset, offset),
    }
}

/// The matches of `JS_WORD_REGEX` in `line`:
/// `(-?\d*\.\d\w*)|([^\`\~\!\@\#\%\^\&\*\(\)\-\=\+\[\{\]\}\\\|\;\:\'\"\,\.\<\>\/\?\s]+)`.
fn js_words(line: &str) -> impl Iterator<Item = (usize, &str)> {
    const SEPARATORS: &str = "`~!@#%^&*()-=+[{]}\\|;:'\",.<>/?";
    let is_word = |c: char| !c.is_whitespace() && c != '\u{feff}' && !SEPARATORS.contains(c);
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < line.len() {
            let start = i;
            if let Some(len) = number_at(&line[i..]) {
                i += len;
                return Some((start, &line[start..i]));
            }
            let c = line[i..].chars().next().unwrap();
            if is_word(c) {
                i += line[i..].find(|c| !is_word(c)).unwrap_or(line.len() - i);
                return Some((start, &line[start..i]));
            }
            i += c.len_utf8();
        }
        None
    })
}

/// The length of a match of `-?\d*\.\d\w*` at the start of `s`.
fn number_at(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = usize::from(b.first() == Some(&b'-'));
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if b.get(i) != Some(&b'.') || !b.get(i + 1).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    i += 2;
    while b
        .get(i)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
    {
        i += 1;
    }
    Some(i)
}

/// The kind of `getFoldingRanges`: `/^\s*\/(?:(\/\s*#(?:end)?region\b)|(\*|\/))/`.
fn folding_kind(text: &str) -> Option<FoldingRangeKind> {
    let rest = text.trim_start().strip_prefix('/')?;
    let region = rest.strip_prefix('/').and_then(|r| {
        let r = r.trim_start().strip_prefix('#')?;
        let r = r.strip_prefix("end").unwrap_or(r).strip_prefix("region")?;
        (!r.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')).then_some(())
    });
    if region.is_some() {
        Some(FoldingRangeKind::Region)
    } else if rest.starts_with(['*', '/']) {
        Some(FoldingRangeKind::Comment)
    } else {
        None
    }
}

/// `computeInitialIndent`.
fn compute_initial_indent(document: &TextDocument, range: Range, tab_size: u32) -> u32 {
    let line_start = document.offset_at(Position::new(range.start.line, 0));
    let tab_size = if tab_size == 0 { 4 } else { tab_size };
    let mut n = 0;
    for c in document.text()[line_start..].chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += tab_size,
            _ => break,
        }
    }
    n / tab_size
}

/// `generateIndent`.
fn generate_indent(level: u32, insert_spaces: bool, tab_size: u32) -> String {
    if insert_spaces {
        " ".repeat((level * tab_size) as usize)
    } else {
        "\t".repeat(level as usize)
    }
}

/// `convertOptions`. Upstream swaps the two `placeOpenBraceOnNewLine` settings.
fn convert_options(
    insert_spaces: bool,
    tab_size: u32,
    format: Option<&Value>,
    initial_indent_level: u32,
) -> FormatCodeSettings {
    let tristate = |b: bool| if b { Tristate::True } else { Tristate::False };
    let on = |key: &str| tristate(format.is_none_or(|f| truthy(f.get(key))));
    let off = |key: &str| tristate(format.is_some_and(|f| truthy(f.get(key))));
    let mut settings = lsutil::get_default_format_code_settings();
    let editor = &mut settings.editor_settings;
    editor.convert_tabs_to_spaces = tristate(insert_spaces);
    editor.tab_size = tab_size as i32;
    editor.indent_size = tab_size as i32;
    editor.indent_style = IndentStyle::SMART;
    editor.new_line_character = "\n".to_owned();
    editor.base_indent_size = (tab_size * initial_indent_level) as i32;
    settings.insert_space_after_comma_delimiter = on("insertSpaceAfterCommaDelimiter");
    settings.insert_space_after_constructor = off("insertSpaceAfterConstructor");
    settings.insert_space_after_semicolon_in_for_statements =
        on("insertSpaceAfterSemicolonInForStatements");
    settings.insert_space_before_and_after_binary_operators =
        on("insertSpaceBeforeAndAfterBinaryOperators");
    settings.insert_space_after_keywords_in_control_flow_statements =
        on("insertSpaceAfterKeywordsInControlFlowStatements");
    settings.insert_space_after_function_keyword_for_anonymous_functions =
        on("insertSpaceAfterFunctionKeywordForAnonymousFunctions");
    settings.insert_space_before_function_parenthesis = off("insertSpaceBeforeFunctionParenthesis");
    settings.insert_space_after_opening_and_before_closing_nonempty_parenthesis =
        off("insertSpaceAfterOpeningAndBeforeClosingNonemptyParenthesis");
    settings.insert_space_after_opening_and_before_closing_nonempty_brackets =
        off("insertSpaceAfterOpeningAndBeforeClosingNonemptyBrackets");
    settings.insert_space_after_opening_and_before_closing_nonempty_braces =
        off("insertSpaceAfterOpeningAndBeforeClosingNonemptyBraces");
    settings.insert_space_after_opening_and_before_closing_empty_braces =
        on("insertSpaceAfterOpeningAndBeforeClosingEmptyBraces");
    settings.insert_space_after_opening_and_before_closing_template_string_braces =
        off("insertSpaceAfterOpeningAndBeforeClosingTemplateStringBraces");
    settings.insert_space_after_opening_and_before_closing_jsx_expression_braces =
        off("insertSpaceAfterOpeningAndBeforeClosingJsxExpressionBraces");
    settings.insert_space_after_type_assertion = off("insertSpaceAfterTypeAssertion");
    settings.place_open_brace_on_new_line_for_control_blocks =
        off("placeOpenBraceOnNewLineForFunctions");
    settings.place_open_brace_on_new_line_for_functions =
        off("placeOpenBraceOnNewLineForControlBlocks");
    settings.semicolons = match format
        .and_then(|f| f.get("semicolons"))
        .and_then(Value::as_str)
    {
        Some("insert") => SemicolonPreference::Insert,
        Some("remove") => SemicolonPreference::Remove,
        _ => SemicolonPreference::Ignore,
    };
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        assert_eq!(
            js_words("a.bc(-1.5e3, x_y) a1.5").collect::<Vec<_>>(),
            [
                (0, "a"),
                (2, "bc"),
                (5, "-1.5e3"),
                (13, "x_y"),
                (18, "a1"),
                (20, ".5")
            ]
        );
        let text = "foo.bar\nx";
        assert_eq!(word_at_text(text, 7), (4, 7));
        assert_eq!(word_at_text(text, 4), (4, 7));
        assert_eq!(word_at_text(text, 3), (0, 3));
        assert_eq!(word_at_text("a. ", 3), (3, 3));
    }

    #[test]
    fn folding_kinds() {
        for (text, kind) in [
            ("  // #region a", Some(FoldingRangeKind::Region)),
            ("//#endregion", Some(FoldingRangeKind::Region)),
            ("// #regions", Some(FoldingRangeKind::Comment)),
            ("/* a */", Some(FoldingRangeKind::Comment)),
            ("{ a }", None),
        ] {
            assert_eq!(folding_kind(text), kind, "{text}");
        }
    }
}
