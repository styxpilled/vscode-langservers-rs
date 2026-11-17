//! Port of `htmlServer.ts`, `customData.ts`, `utils/runner.ts` and
//! `utils/validation.ts`, with the tag auto insertion of Zed's fork
//! (`zed-industries/vscode`, branch `html-tags-as-workspace-edits`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use css_language_server::fs::RequestService;
use css_language_server::settings::{get, truthy};
use css_languageservice::language_facts::colors::Color;
use css_languageservice::services::links::DocumentLink;
use css_languageservice::services::navigation::ColorInformation;
use html_languageservice::{HtmlDataProvider, LanguageService};
use lsp_core::completion::{CompletionSupport, ResolveStore};
use lsp_core::server::{
    Core, Scoped, ScopedSettings, Service, capability, catch, contains_markdown, limit,
    unwrap_positional,
};
use lsp_core::{DEFAULT_RESULT_LIMIT, Writer};
use lsp_server::{ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    ClientCapabilities, Diagnostic, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentSymbolResponse, FoldingRange, Position, Range,
    SelectionRange, TextDocumentIdentifier, TextEdit, Uri,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::{RawValue, to_raw_value};
use serde_json::{Value, json};
use text_document::TextDocument;

use crate::context::WorkspaceContext;
use crate::modes::{Completions, Document, LanguageModes, Mode};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PositionParams {
    text_document: TextDocumentIdentifier,
    position: Position,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameParams {
    text_document: TextDocumentIdentifier,
    position: Position,
    new_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentParams {
    text_document: TextDocumentIdentifier,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SemanticTokenParams {
    text_document: TextDocumentIdentifier,
    ranges: Option<Vec<Range>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ColorPresentationParams {
    text_document: TextDocumentIdentifier,
    color: Color,
    range: Range,
}

/// Formatting params with the options as JSON: VS Code adds the
/// `html.format.*` settings.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FormattingParams {
    range: Option<Range>,
    options: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectionRangeParams {
    text_document: TextDocumentIdentifier,
    positions: Vec<Position>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AutoInsertParams {
    kind: String,
    text_document: TextDocumentIdentifier,
    position: Position,
}

/// An operation that waits for the settings of a document.
enum Job {
    /// The document at the time of the request. It is used if the document closes.
    Request(Request, Rc<Document>),
    Validate(Uri),
}

pub enum Outgoing {
    Configuration(Uri),
    /// `html/customDataContent` for the path at an index of a batch.
    CustomData(u32, usize),
}

struct Capabilities {
    scoped_settings: bool,
    workspace_folders: bool,
    dynamic_formatter_registration: bool,
    snippet_edit: bool,
    folding_range_limit: Option<usize>,
    formatter_edit_limit: Option<usize>,
    hierarchical_document_symbols: bool,
    completion: CompletionSupport,
}

/// The paths of a `fetchHTMLDataProviders` call and their providers.
struct CustomDataBatch {
    paths: Vec<String>,
    providers: Vec<Option<Arc<HtmlDataProvider>>>,
}

pub struct Server {
    core: Core<Outgoing>,
    capabilities: Capabilities,
    workspace_folders: Vec<String>,
    modes: LanguageModes,
    resolve: RefCell<ResolveStore>,
    /// `globalSettings`: the `settings` of the last
    /// `workspace/didChangeConfiguration`, `{}` before.
    global: Value,
    documents: HashMap<Uri, Rc<Document>>,
    settings: ScopedSettings<Job>,
    custom_data: HashMap<u32, CustomDataBatch>,
    next_batch: u32,
    /// The ids of the dynamic formatter registrations.
    formatter_registrations: Option<[String; 2]>,
}

/// A result serialized once, without a `Value` tree.
fn raw(value: &impl Serialize) -> Result<Box<RawValue>, String> {
    to_raw_value(value).map_err(|e| e.to_string())
}

/// `parseHTMLData`.
fn parse_html_data(id: &str, source: Option<&str>) -> Arc<HtmlDataProvider> {
    let raw = source.and_then(|s| serde_json::from_str::<Value>(s).ok());
    let data = match raw {
        Some(raw) if raw.is_object() => {
            let field = |k: &str| match raw.get(k) {
                Some(v) if truthy(Some(v)) => v.clone(),
                _ => json!([]),
            };
            json!({
                "tags": field("tags"),
                "globalAttributes": field("globalAttributes"),
                "valueSets": field("valueSets"),
            })
        }
        _ => json!({}),
    };
    Arc::new(HtmlDataProvider::new(id, &data))
}

impl Server {
    /// `onInitialize`. Returns the server and the `InitializeResult`.
    pub fn new(writer: Writer, receiver: Receiver<Message>, params: &Value) -> (Self, Value) {
        let empty = json!({});
        let options = params
            .get("initializationOptions")
            .filter(|o| truthy(Some(o)))
            .unwrap_or(&empty);
        let caps = params.get("capabilities").unwrap_or(&empty);
        let workspace_folders = match params.get("workspaceFolders") {
            Some(Value::Array(folders)) => folders
                .iter()
                .filter_map(|f| Some(f.get("uri")?.as_str()?.to_owned()))
                .collect(),
            _ => match params.get("rootPath").and_then(Value::as_str) {
                Some(root) if !root.is_empty() => {
                    vec![vscode_uri::Uri::file(root).to_string_encoded()]
                }
                _ => Vec::new(),
            },
        };
        let handled_schemas = match options.get("handledSchemas") {
            Some(Value::Array(s)) => s
                .iter()
                .filter_map(|s| Some(s.as_str()?.to_owned()))
                .collect(),
            _ => vec!["file".to_owned()],
        };
        let embedded = options.get("embeddedLanguages").filter(|v| truthy(Some(v)));
        let css_enabled = embedded.is_none_or(|e| truthy(e.get("css")));
        let javascript_enabled = embedded.is_none_or(|e| truthy(e.get("javascript")));
        let completion_format = capability(
            caps,
            "textDocument.completion.completionItem.documentationFormat",
        );
        let hover_format = capability(caps, "textDocument.hover.contentFormat");
        // The HTML service reads only the documentation formats.
        let html_capabilities: ClientCapabilities = serde_json::from_value(json!({
            "textDocument": {
                "completion": { "completionItem": {
                    "documentationFormat": if contains_markdown(completion_format) { json!(["markdown"]) } else { json!([]) }
                } },
                "hover": {
                    "contentFormat": if contains_markdown(hover_format) { json!(["markdown"]) } else { json!([]) }
                }
            }
        }))
        .unwrap();
        let modes = LanguageModes::new(
            css_enabled,
            javascript_enabled,
            LanguageService::new(true, Vec::new(), Some(&html_capabilities)),
            contains_markdown(completion_format),
            contains_markdown(hover_format),
            RequestService { handled_schemas },
        );
        let provide_formatter = options.get("provideFormatter");
        let snippet_support = truthy(capability(
            caps,
            "textDocument.completion.completionItem.snippetSupport",
        ));
        let capabilities = Capabilities {
            scoped_settings: truthy(capability(caps, "workspace.configuration")),
            workspace_folders: truthy(capability(caps, "workspace.workspaceFolders")),
            dynamic_formatter_registration: truthy(capability(
                caps,
                "textDocument.rangeFormatting.dynamicRegistration",
            )) && !provide_formatter.is_some_and(Value::is_boolean),
            snippet_edit: truthy(capability(
                caps,
                "workspace.workspaceEdit.snippetEditSupport",
            )),
            folding_range_limit: limit(
                capability(caps, "textDocument.foldingRange.rangeLimit"),
                Some(DEFAULT_RESULT_LIMIT),
            ),
            formatter_edit_limit: limit(
                options.pointer("/customCapabilities/rangeFormatting/editLimit"),
                None,
            ),
            completion: CompletionSupport::from_capabilities(caps),
            hierarchical_document_symbols: capability(
                caps,
                "textDocument.documentSymbol.hierarchicalDocumentSymbolSupport",
            ) == Some(&Value::Bool(true)),
        };
        let provide_formatter = provide_formatter == Some(&Value::Bool(true));
        let mut result = json!({
            "capabilities": {
                "textDocumentSync": 2,
                "hoverProvider": true,
                "documentHighlightProvider": true,
                "documentRangeFormattingProvider": provide_formatter,
                "documentFormattingProvider": provide_formatter,
                "documentLinkProvider": { "resolveProvider": false },
                "documentSymbolProvider": true,
                "definitionProvider": true,
                "signatureHelpProvider": { "triggerCharacters": ["("] },
                "referencesProvider": true,
                "colorProvider": {},
                "foldingRangeProvider": true,
                "selectionRangeProvider": true,
                "renameProvider": true,
                "linkedEditingRangeProvider": true,
                "diagnosticProvider": {
                    "documentSelector": null,
                    "interFileDependencies": false,
                    "workspaceDiagnostics": false
                }
            },
            "serverInfo": {
                "name": env!("CARGO_BIN_NAME"),
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        if cfg!(feature = "javascript") {
            result["capabilities"]["workspace"] =
                json!({ "textDocumentContent": { "schemes": ["html-server"] } });
        }
        if snippet_support {
            result["capabilities"]["completionProvider"] = json!({
                "resolveProvider": true,
                "triggerCharacters": [".", ":", "<", "\"", "=", "/"]
            });
        }
        let pull_diagnostics = capability(caps, "textDocument.diagnostic").is_some();
        let mut server = Server {
            core: Core::new(writer, receiver, pull_diagnostics),
            capabilities,
            workspace_folders,
            modes,
            resolve: RefCell::default(),
            global: json!({}),
            documents: HashMap::new(),
            settings: ScopedSettings::default(),
            custom_data: HashMap::new(),
            next_batch: 0,
            formatter_registrations: None,
        };
        let data_paths = match options.get("dataPaths") {
            Some(Value::Array(paths)) => paths.clone(),
            _ => Vec::new(),
        };
        server.fetch_custom_data(&data_paths);
        (server, result)
    }

    /// Returns whether the client sent `shutdown` before `exit`.
    pub fn run(mut self) -> bool {
        lsp_core::server::run(&mut self)
    }

    fn document(&self, uri: &Uri) -> Option<Rc<Document>> {
        self.documents.get(uri).cloned()
    }

    fn context<'a>(&'a self, document: &'a Document) -> WorkspaceContext<'a> {
        WorkspaceContext {
            document_uri: document.text.uri().as_str(),
            workspace_folders: &self.workspace_folders,
        }
    }

    /// `getDocumentSettings` when the request needs settings. `None`: `job` waits for the
    /// settings.
    fn scoped_settings(
        &mut self,
        document: &TextDocument,
        job: impl FnOnce() -> Job,
    ) -> Option<Scoped> {
        if !self.capabilities.scoped_settings {
            return Some(None);
        }
        let uri = document.uri();
        let items = || {
            ["css", "html", "javascript", "js/ts"]
                .iter()
                .map(|section| json!({ "scopeUri": uri, "section": section }))
                .collect()
        };
        self.settings.get(
            &mut self.core,
            uri,
            items,
            || Outgoing::Configuration(uri.clone()),
            job,
        )
    }

    fn on_configuration(&mut self, uri: Uri, response: Response) {
        // `then(s => ({ css: s[0], html: s[1], javascript: s[2], 'js/ts': s[3] }))`.
        // A failed request is not cached: the next request asks again.
        let scoped = match response.response_result {
            Ok(Value::Array(items)) => {
                let mut settings = serde_json::Map::new();
                for (key, item) in ["css", "html", "javascript", "js/ts"].iter().zip(items) {
                    settings.insert((*key).to_owned(), item);
                }
                Some(Some(Value::Object(settings)))
            }
            Ok(_) | Err(_) => None,
        };
        for job in self.settings.resolve(uri, &response.id, scoped.as_ref()) {
            let scoped = scoped.clone().flatten();
            match job {
                Job::Request(request, document) => {
                    let document = self.document(document.text.uri()).unwrap_or(document);
                    self.with_settings(request, &document, scoped);
                }
                Job::Validate(uri) => self.finish_validation(uri, scoped),
            }
        }
    }

    fn on_custom_data(&mut self, batch: u32, index: usize, response: Response) {
        let content = response
            .response_result
            .as_ref()
            .ok()
            .and_then(Value::as_str);
        let Some(b) = self.custom_data.get_mut(&batch) else {
            return;
        };
        b.providers[index] = Some(parse_html_data(&b.paths[index], content));
        if b.providers.iter().all(Option::is_some) {
            let b = self.custom_data.remove(&batch).unwrap();
            self.set_data_providers(b.providers.into_iter().flatten().collect());
        }
    }

    /// `fetchHTMLDataProviders`: `html/customDataContent` for each path, then
    /// `updateDataProviders` with all providers.
    fn fetch_custom_data(&mut self, paths: &[Value]) {
        if paths.is_empty() {
            return self.set_data_providers(Vec::new());
        }
        let batch = self.next_batch;
        self.next_batch += 1;
        let paths: Vec<String> = paths
            .iter()
            .map(|p| match p {
                Value::String(s) => s.clone(),
                p => p.to_string(),
            })
            .collect();
        self.custom_data.insert(
            batch,
            CustomDataBatch {
                providers: vec![None; paths.len()],
                paths: paths.clone(),
            },
        );
        for (i, path) in paths.iter().enumerate() {
            // `vscode-jsonrpc` sends a single string parameter as `[param]`.
            self.core.send_tracked(
                "html/customDataContent",
                json!([path]),
                Outgoing::CustomData(batch, i),
            );
        }
    }

    /// `updateDataProviders`. The parsed documents are dropped, so they use
    /// the new void elements.
    fn set_data_providers(&mut self, providers: Vec<Arc<HtmlDataProvider>>) {
        self.modes
            .html
            .data_manager
            .set_data_providers(true, providers);
        for document in self.documents.values_mut() {
            *document = Rc::new(Document::new(document.text.clone()));
        }
    }

    fn request(&mut self, request: Request) {
        let empty = || json!([]);
        match request.method.as_str() {
            "textDocument/completion"
            | "textDocument/hover"
            | "textDocument/formatting"
            | "textDocument/rangeFormatting" => self.needs_settings(request),
            "textDocument/diagnostic" if self.core.pull_diagnostics() => {
                self.needs_settings(request)
            }
            "completionItem/resolve" => self.resolve(request),
            "workspace/textDocumentContent" => {
                let uri = request.params.get("uri").and_then(Value::as_str);
                let text = uri.and_then(|u| self.modes.text_document_content(u));
                self.core.send(Response::new_ok(
                    request.id,
                    json!({ "text": text.unwrap_or("") }),
                ));
            }
            "textDocument/documentHighlight" => self.handle(
                request,
                empty(),
                "computing document highlights for",
                Self::document_highlights,
            ),
            "textDocument/definition" => self.handle(
                request,
                Value::Null,
                "computing definitions for",
                Self::definition,
            ),
            "textDocument/references" => self.handle(
                request,
                empty(),
                "computing references for",
                Self::references,
            ),
            "textDocument/signatureHelp" => self.handle(
                request,
                Value::Null,
                "computing signature help for",
                Self::signature_help,
            ),
            "textDocument/documentLink" => {
                self.handle(request, empty(), "document links for", Self::document_links)
            }
            "textDocument/documentSymbol" => self.handle(
                request,
                empty(),
                "computing document symbols for",
                Self::document_symbols,
            ),
            "textDocument/documentColor" => self.handle(
                request,
                empty(),
                "computing document colors for",
                Self::document_colors,
            ),
            "textDocument/colorPresentation" => self.handle(
                request,
                empty(),
                "computing color presentations for",
                Self::color_presentations,
            ),
            "html/autoInsert" => self.handle(
                request,
                Value::Null,
                "computing auto insert actions for",
                Self::auto_insert,
            ),
            "textDocument/foldingRange" => self.handle(
                request,
                Value::Null,
                "computing folding regions for",
                Self::folding_ranges,
            ),
            "textDocument/selectionRange" => self.handle(
                request,
                empty(),
                "computing selection ranges for",
                Self::selection_ranges,
            ),
            "textDocument/rename" => {
                self.handle(request, Value::Null, "computing rename for", Self::rename)
            }
            "textDocument/linkedEditingRange" => self.handle(
                request,
                Value::Null,
                "computing synced regions for",
                Self::linked_editing,
            ),
            "html/semanticTokens" => self.handle(
                request,
                Value::Null,
                "computing semantic tokens for",
                Self::semantic_tokens,
            ),
            "html/semanticTokenLegend" => self.core.send(Response::new_ok(
                request.id,
                self.modes.semantic_token_legend(),
            )),
            _ => self.core.send(Response::new_err(
                request.id,
                ErrorCode::MethodNotFound as i32,
                format!("Unhandled method {}", request.method),
            )),
        }
    }

    /// Whether a request of `needs_settings` reads the document settings.
    fn settings_needed(&self, request: &Request, document: &Document) -> bool {
        let mode_at = |path: &str| {
            let position = request
                .params
                .pointer(path)
                .and_then(|p| serde_json::from_value::<Position>(p.clone()).ok())?;
            self.modes.mode_at_position(document, position)
        };
        match request.method.as_str() {
            "textDocument/completion" => mode_at("/position").is_some(),
            "textDocument/hover" => mode_at("/position") == Some(Mode::Html),
            "textDocument/diagnostic" => self.validation_needs_settings(document),
            _ => true,
        }
    }

    /// A request that can wait for the settings of its document.
    fn needs_settings(&mut self, request: Request) {
        let uri = request
            .params
            .pointer("/textDocument/uri")
            .and_then(|u| serde_json::from_value::<Uri>(u.clone()).ok());
        let Some(document) = uri.and_then(|uri| self.document(&uri)) else {
            return self.with_settings_or_default(request, None);
        };
        if !self.settings_needed(&request, &document) {
            return self.with_settings(request, &document, None);
        }
        let job = || Job::Request(request.clone(), document.clone());
        if let Some(scoped) = self.scoped_settings(&document.text, job) {
            self.with_settings(request, &document, scoped);
        }
    }

    fn with_settings(&mut self, request: Request, document: &Rc<Document>, scoped: Scoped) {
        self.with_settings_or_default(request, Some((document, scoped)));
    }

    /// Runs a request of `needs_settings`. Without a document, the result is
    /// the value of a missing document.
    fn with_settings_or_default(
        &mut self,
        request: Request,
        context: Option<(&Rc<Document>, Scoped)>,
    ) {
        let uri = request
            .params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let (error_value, action) = match request.method.as_str() {
            "textDocument/completion" => (Value::Null, "computing completions for"),
            "textDocument/hover" => (Value::Null, "computing hover for"),
            "textDocument/formatting" => (json!([]), "formatting"),
            "textDocument/rangeFormatting" => (json!([]), "formatting range for"),
            _ => (
                json!({ "kind": "full", "items": [] }),
                "computing diagnostics for",
            ),
        };
        let Some((document, scoped)) = context else {
            let missing = match request.method.as_str() {
                "textDocument/completion" | "textDocument/hover" => Value::Null,
                _ => error_value,
            };
            return self.core.send(Response::new_ok(request.id, missing));
        };
        let result = catch(|| -> Result<Box<RawValue>, String> {
            let params = request.params.clone();
            let parse = |e: serde_json::Error| e.to_string();
            match request.method.as_str() {
                "textDocument/completion" => {
                    let p: PositionParams = serde_json::from_value(params).map_err(parse)?;
                    self.completion(document, p.position, scoped.as_ref())
                }
                "textDocument/hover" => {
                    let p: PositionParams = serde_json::from_value(params).map_err(parse)?;
                    raw(&self.hover(document, p.position, scoped.as_ref())?)
                }
                "textDocument/formatting" | "textDocument/rangeFormatting" => {
                    let p: FormattingParams = serde_json::from_value(params).map_err(parse)?;
                    raw(&self.format(document, p.range, &p.options, scoped.as_ref())?)
                }
                _ => {
                    let items = self
                        .validate_document(document, scoped.as_ref())
                        .unwrap_or_else(|message| {
                            self.core
                                .log_error(&format!("Error while validating {uri}: {message}"));
                            Vec::new()
                        });
                    raw(&json!({ "kind": "full", "items": items }))
                }
            }
        })
        .and_then(|r| r);
        match result {
            Ok(result) => self.core.respond(request.id, &result),
            Err(message) => self.core.fail(request, error_value, action, &message),
        }
    }

    /// `runSafe`: an error or a panic gives `error_value`.
    fn handle<P: DeserializeOwned, R: Serialize>(
        &mut self,
        request: Request,
        error_value: Value,
        action: &str,
        f: fn(&mut Self, P) -> Result<R, String>,
    ) {
        lsp_core::server::handle(self, request, error_value, action, f);
    }

    fn notification(&mut self, notification: Notification) -> anyhow::Result<()> {
        let params = notification.params;
        match notification.method.as_str() {
            "initialized" => {
                if self.capabilities.workspace_folders {
                    let id = self.registration_id();
                    self.core.send_request(
                        "client/registerCapability",
                        json!({ "registrations": [{ "id": id, "method": "workspace/didChangeWorkspaceFolders", "registerOptions": {} }] }),
                    );
                }
            }
            "textDocument/didOpen" => {
                let p: DidOpenTextDocumentParams = serde_json::from_value(params)?;
                let item = p.text_document;
                let text =
                    TextDocument::new(item.uri.clone(), item.language_id, item.version, item.text);
                self.documents
                    .insert(item.uri.clone(), Rc::new(Document::new(text)));
                self.core.trigger_validation(item.uri);
            }
            "textDocument/didChange" => {
                let p: DidChangeTextDocumentParams = serde_json::from_value(params)?;
                let uri = p.text_document.uri;
                let Some(document) = self.documents.remove(&uri) else {
                    return Ok(());
                };
                let text = match Rc::try_unwrap(document) {
                    Ok(document) => document.text,
                    Err(document) => document.text.clone(),
                };
                let single = (p.content_changes.len() == 1).then(|| p.content_changes[0].clone());
                let text = text.update(p.content_changes, p.text_document.version);
                self.documents
                    .insert(uri.clone(), Rc::new(Document::new(text)));
                self.core.trigger_validation(uri.clone());
                if self.capabilities.snippet_edit
                    && let Some(change) = single
                    && let Some(range) = change.range
                {
                    self.auto_insert_on_change(&uri, range.start, &change.text);
                }
            }
            "textDocument/didClose" => {
                let p: DidCloseTextDocumentParams = serde_json::from_value(params)?;
                let uri = p.text_document.uri;
                self.documents.remove(&uri);
                self.settings.remove(&uri);
                self.core.close(uri);
            }
            "workspace/didChangeConfiguration" => {
                self.global = params.get("settings").cloned().unwrap_or(Value::Null);
                self.settings.clear();
                self.core.request_refresh(self.documents.keys());
                self.update_formatter_registrations();
            }
            "workspace/didChangeWorkspaceFolders" if self.capabilities.workspace_folders => {
                let event = &params["event"];
                let uris = |key: &str| -> Vec<String> {
                    event[key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|f| Some(f.get("uri")?.as_str()?.to_owned()))
                        .collect()
                };
                let (added, removed) = (uris("added"), uris("removed"));
                self.workspace_folders
                    .retain(|f| !removed.contains(f) && !added.contains(f));
                self.workspace_folders.extend(added);
                self.core.request_refresh(self.documents.keys());
            }
            "html/customDataChanged" => {
                let paths = match unwrap_positional(params, true) {
                    Value::Array(paths) => paths,
                    params => anyhow::bail!("dataPaths.map is not a function: {params}"),
                };
                self.fetch_custom_data(&paths);
            }
            "$/cancelRequest" => {
                let id: RequestId = serde_json::from_value(params["id"].clone())?;
                if self
                    .settings
                    .cancel(|job| matches!(job, Job::Request(r, _) if r.id == id))
                {
                    self.core.respond_cancelled(id);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn registration_id(&mut self) -> String {
        format!("html-server-registration-{}", self.core.next_id())
    }

    /// The dynamic formatter registration of `onDidChangeConfiguration`.
    fn update_formatter_registrations(&mut self) {
        if !self.capabilities.dynamic_formatter_registration {
            return;
        }
        let html = get(Some(&self.global), "html");
        let enable = truthy(get(get(html, "format"), "enable"));
        if enable && self.formatter_registrations.is_none() {
            let selector = json!([{ "language": "html" }, { "language": "handlebars" }]);
            let ids = [self.registration_id(), self.registration_id()];
            for (id, method) in ids
                .iter()
                .zip(["textDocument/rangeFormatting", "textDocument/formatting"])
            {
                self.core.send_request(
                    "client/registerCapability",
                    json!({ "registrations": [{ "id": id, "method": method, "registerOptions": { "documentSelector": selector } }] }),
                );
            }
            self.formatter_registrations = Some(ids);
        } else if !enable && let Some(ids) = self.formatter_registrations.take() {
            for (id, method) in ids
                .iter()
                .zip(["textDocument/rangeFormatting", "textDocument/formatting"])
            {
                self.core.send_request(
                    "client/unregisterCapability",
                    json!({ "unregisterations": [{ "id": id, "method": method }] }),
                );
            }
        }
    }

    /// Zed's fork: after a change of one character, the tag or quote
    /// completion goes to the client as a snippet edit.
    fn auto_insert_on_change(&mut self, uri: &Uri, start: Position, text: &str) {
        let Some(document) = self.document(uri) else {
            return;
        };
        let position = Position::new(start.line, start.character + 1);
        let at = Position::new(position.line, position.character - 1);
        if self.modes.mode_at_position(&document, at) != Some(Mode::Html) {
            return;
        }
        let html = self.global.as_object().and_then(|g| g.get("html"));
        if html.is_some_and(|h| {
            h.as_object().is_none_or(|h| {
                h.get("tagAutoclosing")
                    .is_some_and(|v| v != &Value::Bool(true))
            })
        }) {
            return;
        }
        let kind = match text {
            "=" => "autoQuote",
            ">" | "/" => "autoClose",
            _ => return,
        };
        let global = Some(&self.global);
        let snippet = match catch(|| self.modes.auto_insert(&document, position, kind, global)) {
            Ok(Some(snippet)) => snippet,
            Ok(None) => return,
            Err(message) => {
                return self
                    .core
                    .log_error(&format!("Error while handling notification: {message}"));
            }
        };
        let edit = json!({
            "documentChanges": [{
                "textDocument": { "uri": uri, "version": document.text.version() },
                "edits": [{
                    "snippet": { "kind": "snippet", "value": snippet },
                    "range": Range::new(position, position)
                }]
            }]
        });
        self.core
            .send_request("workspace/applyEdit", json!({ "edit": edit }));
    }

    /// Validates the current text, so a change during the wait for the
    /// settings does not publish stale diagnostics. A closed document is not
    /// validated.
    fn finish_validation(&mut self, uri: Uri, scoped: Scoped) {
        let Some(document) = self.document(&uri) else {
            return;
        };
        match catch(|| self.validate_document(&document, scoped.as_ref())).and_then(|r| r) {
            Ok(diagnostics) => self.core.publish(uri, diagnostics),
            Err(message) => {
                self.core.log_error(&format!(
                    "Error while validating {}: {message}",
                    uri.as_str()
                ));
                self.core.publish(uri, Vec::new());
            }
        }
    }

    /// Whether `validateTextDocument` reads the document settings.
    fn validation_needs_settings(&self, document: &Document) -> bool {
        document.text.language_id() == "html"
            && self
                .modes
                .modes_in_document(document)
                .into_iter()
                .any(|m| self.modes.validates(m))
    }

    /// `validateTextDocument`. `Err` where it catches an error.
    fn validate_document(
        &self,
        document: &Document,
        scoped: Option<&Value>,
    ) -> Result<Vec<Diagnostic>, String> {
        if document.text.language_id() != "html" {
            return Ok(Vec::new());
        }
        let settings = scoped.unwrap_or(&self.global);
        let mut diagnostics = Vec::new();
        for mode in self.modes.modes_in_document(document) {
            match mode {
                Mode::Css if validation_enabled("css", settings) => {
                    diagnostics.extend(self.modes.css_validation(document, Some(settings)));
                }
                #[cfg(feature = "javascript")]
                Mode::Script(script) if validation_enabled(script.id(), settings) => {
                    diagnostics.extend(self.modes.script_validation(
                        script,
                        document,
                        Some(settings),
                    )?);
                }
                _ => {}
            }
        }
        Ok(diagnostics)
    }

    fn completion(
        &self,
        document: &Document,
        position: Position,
        scoped: Option<&Value>,
    ) -> Result<Box<RawValue>, String> {
        let Some(mode) = self.modes.mode_at_position(document, position) else {
            return raw(&json!({ "isIncomplete": true, "items": [] }));
        };
        let settings = scoped.unwrap_or(&self.global);
        let context = self.context(document);
        match self
            .modes
            .complete(mode, document, position, &context, Some(settings))?
        {
            Completions::List {
                is_incomplete,
                items,
                edit_range,
            } => raw(&self.resolve.borrow_mut().compact(
                self.capabilities.completion,
                is_incomplete,
                items,
                edit_range,
            )),
            #[cfg(feature = "javascript")]
            Completions::Raw(list) => Ok(list),
        }
    }

    fn hover(
        &self,
        document: &Document,
        position: Position,
        scoped: Option<&Value>,
    ) -> Result<Option<lsp_types::Hover>, String> {
        let Some(mode) = self.modes.mode_at_position(document, position) else {
            return Ok(None);
        };
        self.modes
            .hover(mode, document, position, scoped, Some(&self.global))
    }

    /// `onCompletionResolve`: an error gives the item.
    fn resolve(&mut self, request: Request) {
        let mut item = request.params;
        if item.get("data").is_some_and(Value::is_array) {
            self.resolve.borrow().resolve(&mut item);
            return self.core.send(Response::new_ok(request.id, item));
        }
        let data = item.get("data").filter(|d| {
            d.get("languageId").is_some_and(Value::is_string)
                && d.get("uri").is_some_and(Value::is_string)
                && d.get("offset").is_some_and(Value::is_number)
        });
        let document = data
            .and_then(|d| d["uri"].as_str()?.parse::<Uri>().ok())
            .and_then(|uri| self.document(&uri));
        let Some(document) = document else {
            return self.core.send(Response::new_ok(request.id, item));
        };
        let result = catch(|| self.modes.resolve(&document, item.clone())).and_then(|r| r);
        match result {
            Ok(resolved) => self.core.send(Response::new_ok(request.id, resolved)),
            Err(message) => {
                self.core.log_error(&format!(
                    "Error while resolving completion proposal: {message}"
                ));
                self.core.send(Response::new_ok(request.id, item));
            }
        }
    }

    /// `onFormat`.
    fn format(
        &self,
        document: &Document,
        range: Option<Range>,
        options: &Value,
        scoped: Option<&Value>,
    ) -> Result<Vec<TextEdit>, String> {
        let settings = scoped.unwrap_or(&self.global);
        let text = &document.text;
        let full_range = Range::new(Position::new(0, 0), text.position_at(text.text().len()));
        let edits = self.modes.format(
            document,
            range.unwrap_or(full_range),
            options,
            Some(settings),
        )?;
        if self
            .capabilities
            .formatter_edit_limit
            .is_some_and(|limit| edits.len() > limit)
        {
            let new_text = text.apply_edits(&edits);
            return Ok(vec![TextEdit::new(full_range, new_text)]);
        }
        Ok(edits)
    }

    fn mode_at(&self, document: &Document, position: Position) -> Option<Mode> {
        self.modes.mode_at_position(document, position)
    }

    fn document_highlights(&mut self, p: PositionParams) -> Result<Value, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(json!([]));
        };
        let Some(mode) = self.mode_at(&document, p.position) else {
            return Ok(json!([]));
        };
        Ok(json!(
            self.modes
                .document_highlights(mode, &document, p.position)?
        ))
    }

    fn definition(&mut self, p: PositionParams) -> Result<Value, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(json!([]));
        };
        let Some(mode) = self.mode_at(&document, p.position) else {
            return Ok(json!([]));
        };
        Ok(self
            .modes
            .definition(mode, &document, p.position)?
            .unwrap_or_else(|| json!([])))
    }

    fn references(&mut self, p: PositionParams) -> Result<Value, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(json!([]));
        };
        let Some(mode) = self.mode_at(&document, p.position) else {
            return Ok(json!([]));
        };
        Ok(json!(self.modes.references(mode, &document, p.position)?))
    }

    fn signature_help(&mut self, p: PositionParams) -> Result<Value, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Value::Null);
        };
        let Some(mode) = self.mode_at(&document, p.position) else {
            return Ok(Value::Null);
        };
        self.modes.signature_help(mode, &document, p.position)
    }

    fn document_links(&mut self, p: DocumentParams) -> Result<Vec<DocumentLink>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Vec::new());
        };
        let context = self.context(&document);
        let links: Vec<_> = self
            .modes
            .modes_in_document(&document)
            .into_iter()
            .flat_map(|m| self.modes.document_links(m, &document, &context))
            .collect();
        Ok(links)
    }

    fn document_symbols(&mut self, p: DocumentParams) -> Result<DocumentSymbolResponse, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(DocumentSymbolResponse::Flat(Vec::new()));
        };
        let modes = self.modes.modes_in_document(&document);
        if self.capabilities.hierarchical_document_symbols {
            let mut symbols = Vec::new();
            for mode in modes {
                symbols.extend(self.modes.document_symbols_nested(mode, &document)?);
            }
            lsp_core::cap_symbol_depth(&mut symbols);
            return Ok(DocumentSymbolResponse::Nested(symbols));
        }
        let mut symbols = Vec::new();
        for mode in modes {
            symbols.extend(self.modes.document_symbols(mode, &document)?);
        }
        Ok(DocumentSymbolResponse::Flat(symbols))
    }

    fn document_colors(&mut self, p: DocumentParams) -> Result<Vec<ColorInformation>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Vec::new());
        };
        let colors: Vec<_> = self
            .modes
            .modes_in_document(&document)
            .into_iter()
            .flat_map(|m| self.modes.document_colors(m, &document))
            .collect();
        Ok(colors)
    }

    fn color_presentations(&mut self, p: ColorPresentationParams) -> Result<Value, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(json!([]));
        };
        let Some(mode) = self.mode_at(&document, p.range.start) else {
            return Ok(json!([]));
        };
        Ok(json!(
            self.modes
                .color_presentations(mode, &document, p.color, p.range)
        ))
    }

    fn auto_insert(&mut self, p: AutoInsertParams) -> Result<Value, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Value::Null);
        };
        let pos = p.position;
        if pos.character == 0 {
            return Ok(Value::Null);
        }
        let at = Position::new(pos.line, pos.character - 1);
        if self.mode_at(&document, at) != Some(Mode::Html) {
            return Ok(Value::Null);
        }
        Ok(json!(self.modes.auto_insert(
            &document,
            pos,
            &p.kind,
            Some(&self.global)
        )))
    }

    fn folding_ranges(&mut self, p: DocumentParams) -> Result<Option<Vec<FoldingRange>>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(None);
        };
        Ok(Some(self.modes.folding_ranges(
            &document,
            self.capabilities.folding_range_limit,
        )?))
    }

    fn selection_ranges(&mut self, p: SelectionRangeParams) -> Result<Vec<SelectionRange>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Vec::new());
        };
        self.modes.selection_ranges(&document, &p.positions)
    }

    fn rename(&mut self, p: RenameParams) -> Result<Value, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Value::Null);
        };
        let Some(mode) = self.mode_at(&document, p.position) else {
            return Ok(Value::Null);
        };
        Ok(json!(self.modes.rename(
            mode,
            &document,
            p.position,
            &p.new_name
        )?))
    }

    fn linked_editing(&mut self, p: PositionParams) -> Result<Value, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Value::Null);
        };
        let pos = p.position;
        if pos.character == 0 {
            return Ok(Value::Null);
        }
        let at = Position::new(pos.line, pos.character - 1);
        let Some(mode) = self.mode_at(&document, at) else {
            return Ok(Value::Null);
        };
        Ok(match self.modes.linked_editing(mode, &document, pos) {
            Some(ranges) => json!({ "ranges": ranges }),
            None => Value::Null,
        })
    }

    fn semantic_tokens(&mut self, p: SemanticTokenParams) -> Result<Value, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Value::Null);
        };
        Ok(json!(self.modes.semantic_tokens(&document, p.ranges)?))
    }
}

/// `isValidationEnabled`.
fn validation_enabled(language_id: &str, settings: &Value) -> bool {
    let validate = get(get(Some(settings), "html"), "validate");
    if truthy(validate) {
        let enabled = |key| get(validate, key) != Some(&Value::Bool(false));
        return match language_id {
            "css" => enabled("styles"),
            "javascript" => enabled("scripts"),
            _ => false,
        };
    }
    true
}

impl Service for Server {
    type Outgoing = Outgoing;
    type Event = ();

    fn core(&mut self) -> &mut Core<Outgoing> {
        &mut self.core
    }

    fn dispatch(&mut self, request: Request) {
        self.request(request);
    }

    fn on_notification(&mut self, notification: Notification) -> anyhow::Result<()> {
        self.notification(notification)
    }

    fn on_response(&mut self, response: Response, kind: Outgoing) {
        match kind {
            Outgoing::Configuration(uri) => self.on_configuration(uri, response),
            Outgoing::CustomData(batch, index) => self.on_custom_data(batch, index, response),
        }
    }

    fn validate(&mut self, uri: Uri) {
        let Some(document) = self.document(&uri) else {
            return;
        };
        if !self.validation_needs_settings(&document) {
            return self.finish_validation(uri, None);
        }
        let job = || Job::Validate(uri.clone());
        if let Some(scoped) = self.scoped_settings(&document.text, job) {
            self.finish_validation(uri, scoped);
        }
    }
}
