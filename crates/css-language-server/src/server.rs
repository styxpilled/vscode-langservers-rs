//! Port of `cssServer.ts`, `customData.ts`, `utils/runner.ts` and `utils/validation.ts`.

use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, at, never, select};
use css_languageservice::data::CssDataProvider;
use css_languageservice::language_facts::CssDataManager;
use css_languageservice::language_facts::colors::Color;
use css_languageservice::parser::{Dialect, Stylesheet, parse_stylesheet};
use css_languageservice::services::code_actions::do_code_actions2;
use css_languageservice::services::completion::Completion;
use css_languageservice::services::folding::get_folding_ranges;
use css_languageservice::services::formatter::format;
use css_languageservice::services::hover::do_hover;
use css_languageservice::services::links::{DocumentLink, FileSystemProvider, LinkResolver};
use css_languageservice::services::navigation::{
    ColorInformation, do_rename, find_definition, find_document_colors, find_document_highlights,
    find_document_symbols, find_references, get_color_presentations,
};
use css_languageservice::services::selection_range::get_selection_ranges;
use css_languageservice::services::validation::do_validation;
use lsp_server::{ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_transport::completion::{CompactList, CompletionSupport, ResolveStore};
use lsp_transport::{DEFAULT_RESULT_LIMIT, Writer};
use lsp_types::{
    CodeAction, CodeActionParams, ColorPresentation, CompletionParams, Diagnostic,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentColorParams, DocumentDiagnosticParams, DocumentHighlight, DocumentHighlightParams,
    DocumentLinkParams, DocumentSymbol, DocumentSymbolParams, FoldingRange, FoldingRangeParams,
    GotoDefinitionParams, Hover, HoverParams, Location, Range, ReferenceParams, RenameParams,
    SelectionRange, SelectionRangeParams, TextDocumentIdentifier, TextEdit, Uri, WorkspaceEdit,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use text_document::TextDocument;

use crate::fs::{RequestService, WorkspaceContext};
use crate::settings::{
    alias_settings, completion_settings, format_configuration, get, hover_settings, truthy,
    validation_settings,
};

const VALIDATION_DELAY: Duration = Duration::from_millis(500);

/// `ColorPresentationParams` with the `f64` `Color`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ColorPresentationParams {
    text_document: TextDocumentIdentifier,
    color: Color,
    range: Range,
}

/// Formatting params with the options as JSON: VS Code adds the `css.format.*` settings.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FormattingParams {
    text_document: TextDocumentIdentifier,
    range: Option<Range>,
    options: Value,
}

/// `lsp-types` has no type for this response without `relatedDocuments`.
#[derive(Serialize)]
struct FullDiagnosticReport {
    kind: &'static str,
    items: Vec<Diagnostic>,
}

struct Document {
    text: TextDocument,
    stylesheet: OnceCell<Stylesheet>,
}

/// The settings of a document from `workspace/configuration`. `None` selects the
/// `configure` settings.
type Scoped = Option<Value>;

enum DocumentSettings {
    Pending(RequestId),
    Ready(Scoped),
}

/// An operation that waits for the settings of a document.
enum Job {
    /// The document at the time of the request. It is used if the document closes.
    Request(Request, Rc<Document>),
    Validate(Uri),
}

enum Outgoing {
    Configuration(Uri),
    Other,
}

struct Capabilities {
    scoped_settings: bool,
    folding_range_limit: Option<usize>,
    formatter_edit_limit: Option<usize>,
    pull_diagnostics: bool,
    hover_markdown: bool,
    completion_markdown: bool,
    completion: CompletionSupport,
}

pub struct Server {
    writer: Writer,
    receiver: Receiver<Message>,
    inbox: VecDeque<Message>,
    capabilities: Capabilities,
    workspace_folders: Vec<String>,
    request_service: RequestService,
    data: CssDataManager,
    resolve: RefCell<ResolveStore>,
    /// The `settings` of the last `workspace/didChangeConfiguration`. The language services
    /// read their `configure` settings from it.
    configured: Value,
    documents: HashMap<Uri, Rc<Document>>,
    document_settings: HashMap<Uri, DocumentSettings>,
    waiting: Vec<(RequestId, Job)>,
    pending_validations: HashMap<Uri, Instant>,
    outgoing: HashMap<RequestId, Outgoing>,
    next_id: i32,
    shutdown: bool,
}

/// `getClientCapability`.
fn capability<'a>(capabilities: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(capabilities, |c, key| c.as_object()?.get(key))
}

fn contains_markdown(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|f| f == "markdown"))
}

/// A number setting where `0` and non-numbers are `Number.MAX_VALUE`.
fn limit(value: Option<&Value>) -> Option<usize> {
    match value.and_then(Value::as_f64) {
        Some(n) if n != 0.0 => {
            let n = n.max(0.0).trunc();
            (n < usize::MAX as f64).then_some(n as usize)
        }
        _ => None,
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".to_owned())
}

/// `fetchDataProviders` for one path.
fn data_provider(request_service: &RequestService, path: &str) -> CssDataProvider {
    let data = request_service
        .get_content(path)
        .and_then(|content| serde_json::from_str::<Value>(&content).ok());
    CssDataProvider::new(&data.unwrap_or(Value::Null))
}

impl Server {
    /// `onInitialize`. Returns the server and the `InitializeResult`.
    pub fn new(writer: Writer, receiver: Receiver<Message>, params: &Value) -> (Self, Value) {
        let empty = json!({});
        let options = params
            .get("initializationOptions")
            .filter(|o| !o.is_null())
            .unwrap_or(&empty);
        let caps = params.get("capabilities").unwrap_or(&empty);
        let workspace_folders = match params.get("workspaceFolders") {
            Some(Value::Array(folders)) => folders
                .iter()
                .filter_map(|f| Some(f.get("uri")?.as_str()?.to_owned()))
                .collect(),
            _ => match params.get("rootPath").and_then(Value::as_str) {
                Some(root) if !root.is_empty() => {
                    vec![vscode_uri::Uri::file(root).to_string_unencoded()]
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
        let snippet_support = truthy(capability(
            caps,
            "textDocument.completion.completionItem.snippetSupport",
        ));
        let capabilities = Capabilities {
            scoped_settings: truthy(capability(caps, "workspace.configuration")),
            folding_range_limit: limit(capability(caps, "textDocument.foldingRange.rangeLimit"))
                .or(Some(DEFAULT_RESULT_LIMIT)),
            formatter_edit_limit: limit(
                options.pointer("/customCapabilities/rangeFormatting/editLimit"),
            ),
            pull_diagnostics: capability(caps, "textDocument.diagnostic").is_some(),
            hover_markdown: contains_markdown(capability(caps, "textDocument.hover.contentFormat")),
            completion_markdown: contains_markdown(capability(
                caps,
                "textDocument.completion.completionItem.documentationFormat",
            )),
            completion: CompletionSupport::from_capabilities(caps),
        };
        let provide_formatter = options.get("provideFormatter") == Some(&Value::Bool(true));
        let mut result = json!({
            "capabilities": {
                "textDocumentSync": 2,
                "hoverProvider": true,
                "documentSymbolProvider": true,
                "referencesProvider": true,
                "definitionProvider": true,
                "documentHighlightProvider": true,
                "documentLinkProvider": { "resolveProvider": false },
                "codeActionProvider": { "codeActionKinds": ["quickfix"] },
                "renameProvider": true,
                "colorProvider": {},
                "foldingRangeProvider": true,
                "selectionRangeProvider": true,
                "diagnosticProvider": {
                    "documentSelector": null,
                    "interFileDependencies": false,
                    "workspaceDiagnostics": false
                },
                "documentRangeFormattingProvider": provide_formatter,
                "documentFormattingProvider": provide_formatter
            },
            "serverInfo": {
                "name": "vscode-css-language-server",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        if snippet_support {
            result["capabilities"]["completionProvider"] = json!({
                "resolveProvider": capabilities.completion.resolves(),
                "triggerCharacters": ["/", "-", ":"]
            });
        }
        let server = Server {
            writer,
            receiver,
            inbox: VecDeque::new(),
            capabilities,
            workspace_folders,
            request_service: RequestService { handled_schemas },
            data: CssDataManager::new(true, &[]),
            resolve: RefCell::default(),
            configured: Value::Null,
            documents: HashMap::new(),
            document_settings: HashMap::new(),
            waiting: Vec::new(),
            pending_validations: HashMap::new(),
            outgoing: HashMap::new(),
            next_id: 0,
            shutdown: false,
        };
        (server, result)
    }

    /// Returns whether the client sent `shutdown` before `exit`.
    pub fn run(mut self) -> bool {
        loop {
            let message = match self.inbox.pop_front() {
                Some(message) => message,
                None => {
                    let receiver = self.receiver.clone();
                    let timer = match self.pending_validations.values().min() {
                        Some(&deadline) => at(deadline),
                        None => never(),
                    };
                    select! {
                        recv(receiver) -> message => match message {
                            Ok(message) => message,
                            Err(_) => return self.shutdown,
                        },
                        recv(timer) -> _ => {
                            self.run_due_validations();
                            continue;
                        }
                    }
                }
            };
            match message {
                Message::Request(request) => self.on_request(request),
                Message::Notification(notification) if notification.method == "exit" => {
                    return self.shutdown;
                }
                Message::Notification(notification) => self.on_notification(notification),
                Message::Response(response) => self.on_response(response),
            }
        }
    }

    fn send(&self, message: impl Into<Message>) {
        self.writer.message(message.into());
    }

    fn notify(&self, method: &str, params: impl Serialize) {
        self.writer.notification(method, &params);
    }

    fn send_request(&mut self, method: &str, params: impl Serialize, kind: Outgoing) -> RequestId {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.outgoing.insert(id.clone(), kind);
        self.send(Request::new(id.clone(), method.to_owned(), params));
        id
    }

    fn log(&self, kind: u8, message: &str) {
        self.notify(
            "window/logMessage",
            json!({ "type": kind, "message": message }),
        );
    }

    fn log_error(&self, message: &str) {
        self.log(1, message);
    }

    fn respond_cancelled(&self, id: RequestId) {
        self.send(Response::new_err(
            id,
            ErrorCode::RequestCanceled as i32,
            "Request cancelled".to_owned(),
        ));
    }

    /// Looks ahead in the received messages for a `$/cancelRequest` of `id`.
    fn take_cancellation(&mut self, id: &RequestId) -> bool {
        while let Ok(message) = self.receiver.try_recv() {
            self.inbox.push_back(message);
        }
        let position = self.inbox.iter().position(|m| {
            matches!(m, Message::Notification(n) if n.method == "$/cancelRequest"
                && serde_json::from_value::<RequestId>(n.params["id"].clone()).ok().as_ref() == Some(id))
        });
        position.map(|i| self.inbox.remove(i)).is_some()
    }

    /// `getLanguageService`.
    fn dialect(&self, document: &TextDocument) -> Dialect {
        match document.language_id() {
            "css" => Dialect::Css,
            "scss" => Dialect::Scss,
            "less" => Dialect::Less,
            id => {
                self.log(4, &format!("Document type is {id}, using css instead."));
                Dialect::Css
            }
        }
    }

    /// The `configure` settings of the language service of `document`.
    fn configured(&self, document: &TextDocument) -> Option<&Value> {
        let id = match document.language_id() {
            id @ ("scss" | "less") => id,
            _ => "css",
        };
        get(Some(&self.configured), id)
    }

    fn stylesheet<'a>(&self, document: &'a Document) -> &'a Stylesheet {
        document
            .stylesheet
            .get_or_init(|| parse_stylesheet(self.dialect(&document.text), document.text.text()))
    }

    fn document(&self, uri: &Uri) -> Option<Rc<Document>> {
        self.documents.get(uri).cloned()
    }

    /// `getDocumentSettings`. `Err` is the pending `workspace/configuration` request.
    fn scoped_settings(&mut self, document: &TextDocument) -> Result<Scoped, RequestId> {
        if !self.capabilities.scoped_settings {
            return Ok(None);
        }
        match self.document_settings.get(document.uri()) {
            Some(DocumentSettings::Ready(scoped)) => Ok(scoped.clone()),
            Some(DocumentSettings::Pending(id)) => Err(id.clone()),
            None => {
                let params = json!({
                    "items": [{ "scopeUri": document.uri(), "section": document.language_id() }]
                });
                let uri = document.uri().clone();
                let id = self.send_request(
                    "workspace/configuration",
                    params,
                    Outgoing::Configuration(uri.clone()),
                );
                self.document_settings
                    .insert(uri, DocumentSettings::Pending(id.clone()));
                Err(id)
            }
        }
    }

    fn on_response(&mut self, response: Response) {
        let Some(Outgoing::Configuration(uri)) = self.outgoing.remove(&response.id) else {
            return;
        };
        let pending = matches!(self.document_settings.get(&uri), Some(DocumentSettings::Pending(id)) if *id == response.id);
        // A failed request is not cached: the next request asks again.
        let scoped = match response.response_result {
            Ok(Value::Array(mut items)) => {
                let scoped = (!items.is_empty()).then(|| items.swap_remove(0));
                if pending {
                    self.document_settings
                        .insert(uri, DocumentSettings::Ready(scoped.clone()));
                }
                scoped
            }
            Ok(_) | Err(_) => {
                if pending {
                    self.document_settings.remove(&uri);
                }
                None
            }
        };
        let (ready, waiting) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|(id, _)| *id == response.id);
        self.waiting = waiting;
        for (_, job) in ready {
            match job {
                Job::Request(request, document) => {
                    let document = self.document(document.text.uri()).unwrap_or(document);
                    self.with_settings(request, &document, scoped.clone());
                }
                Job::Validate(uri) => self.finish_validation(uri, scoped.clone()),
            }
        }
    }

    fn on_request(&mut self, request: Request) {
        if request.method == "shutdown" {
            self.shutdown = true;
            self.send(Response::new_ok(request.id, Value::Null));
            return;
        }
        if self.take_cancellation(&request.id) {
            self.respond_cancelled(request.id);
            return;
        }
        self.dispatch(request);
    }

    fn dispatch(&mut self, request: Request) {
        let empty = || json!([]);
        match request.method.as_str() {
            "textDocument/completion" | "textDocument/hover" => self.needs_settings(request),
            "completionItem/resolve" => {
                let mut item = request.params;
                self.resolve.borrow().resolve(&mut item);
                self.send(Response::new_ok(request.id, item));
            }
            "textDocument/diagnostic" if self.capabilities.pull_diagnostics => {
                self.needs_settings(request)
            }
            "textDocument/documentSymbol" => self.handle(
                request,
                empty(),
                "computing document symbols",
                Self::document_symbols,
            ),
            "textDocument/definition" => self.handle(
                request,
                Value::Null,
                "computing definitions",
                Self::definition,
            ),
            "textDocument/documentHighlight" => self.handle(
                request,
                empty(),
                "computing document highlights",
                Self::document_highlights,
            ),
            "textDocument/documentLink" => self.handle(
                request,
                empty(),
                "computing document links",
                Self::document_links,
            ),
            "textDocument/references" => {
                self.handle(request, empty(), "computing references", Self::references)
            }
            "textDocument/codeAction" => self.handle(
                request,
                empty(),
                "computing code actions",
                Self::code_actions,
            ),
            "textDocument/documentColor" => self.handle(
                request,
                empty(),
                "computing document colors",
                Self::document_colors,
            ),
            "textDocument/colorPresentation" => self.handle(
                request,
                empty(),
                "computing color presentations",
                Self::color_presentations,
            ),
            "textDocument/rename" => {
                self.handle(request, Value::Null, "computing renames", Self::rename)
            }
            "textDocument/foldingRange" => self.handle(
                request,
                Value::Null,
                "computing folding ranges",
                Self::folding_ranges,
            ),
            "textDocument/selectionRange" => self.handle(
                request,
                empty(),
                "computing selection ranges",
                Self::selection_ranges,
            ),
            "textDocument/rangeFormatting" => {
                self.handle(request, empty(), "formatting range", Self::formatting)
            }
            "textDocument/formatting" => {
                self.handle(request, empty(), "formatting", Self::formatting)
            }
            _ => self.send(Response::new_err(
                request.id,
                ErrorCode::MethodNotFound as i32,
                format!("Unhandled method {}", request.method),
            )),
        }
    }

    /// A request that waits for the settings of its document.
    fn needs_settings(&mut self, request: Request) {
        let uri = request
            .params
            .pointer("/textDocument/uri")
            .and_then(|u| serde_json::from_value::<Uri>(u.clone()).ok());
        let Some(document) = uri.and_then(|uri| self.document(&uri)) else {
            return self.with_settings_or_default(request, None);
        };
        match self.scoped_settings(&document.text) {
            Ok(scoped) => self.with_settings(request, &document, scoped),
            Err(id) => self.waiting.push((id, Job::Request(request, document))),
        }
    }

    fn with_settings(&mut self, request: Request, document: &Rc<Document>, scoped: Scoped) {
        self.with_settings_or_default(request, Some((document, scoped)));
    }

    /// Runs a request of `needs_settings`. Without a document, the result is the default.
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
            "textDocument/completion" => (Value::Null, "computing completions"),
            "textDocument/hover" => (Value::Null, "computing hover"),
            _ => (
                json!({ "kind": "full", "items": [] }),
                "computing diagnostics",
            ),
        };
        let Some((document, scoped)) = context else {
            return self.send(Response::new_ok(request.id, error_value));
        };
        let result = catch_unwind(AssertUnwindSafe(|| -> serde_json::Result<Value> {
            let params = request.params.clone();
            Ok(match request.method.as_str() {
                "textDocument/completion" => json!(self.completion(
                    serde_json::from_value(params)?,
                    document,
                    scoped.as_ref()
                )),
                "textDocument/hover" => {
                    json!(self.hover(serde_json::from_value(params)?, document, scoped.as_ref()))
                }
                _ => {
                    let _: DocumentDiagnosticParams = serde_json::from_value(params)?;
                    json!(FullDiagnosticReport {
                        kind: "full",
                        items: self.validate(document, scoped.as_ref()),
                    })
                }
            })
        }));
        match result {
            Ok(Ok(result)) => self.writer.response(request.id, &result),
            Ok(Err(e)) => self.send(Response::new_err(
                request.id,
                ErrorCode::InvalidParams as i32,
                e.to_string(),
            )),
            Err(payload) => {
                let message = panic_message(payload);
                self.log_error(&format!("Error while {action} for {uri}: {message}"));
                self.send(Response::new_ok(request.id, error_value));
            }
        }
    }

    /// `runSafeAsync`: a panic gives `error_value`.
    fn handle<P: DeserializeOwned, R: Serialize>(
        &mut self,
        request: Request,
        error_value: Value,
        action: &str,
        f: fn(&mut Self, P) -> Result<R, String>,
    ) {
        let params: P = match serde_json::from_value(request.params.clone()) {
            Ok(params) => params,
            Err(e) => {
                return self.send(Response::new_err(
                    request.id,
                    ErrorCode::InvalidParams as i32,
                    e.to_string(),
                ));
            }
        };
        let message = match catch_unwind(AssertUnwindSafe(|| f(self, params))) {
            Ok(Ok(result)) => return self.writer.response(request.id, &result),
            Ok(Err(message)) => message,
            Err(payload) => panic_message(payload),
        };
        let uri = request
            .params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .unwrap_or_default();
        self.log_error(&format!("Error while {action} for {uri}: {message}"));
        self.send(Response::new_ok(request.id, error_value));
    }

    fn on_notification(&mut self, notification: Notification) {
        let method = notification.method.clone();
        if let Err(e) = self.try_on_notification(notification) {
            self.log_error(&format!("Error while handling {method}: {e}"));
        }
    }

    fn try_on_notification(&mut self, notification: Notification) -> anyhow::Result<()> {
        let params = notification.params;
        match notification.method.as_str() {
            "textDocument/didOpen" => {
                let p: DidOpenTextDocumentParams = serde_json::from_value(params)?;
                let item = p.text_document;
                let text =
                    TextDocument::new(item.uri.clone(), item.language_id, item.version, item.text);
                self.documents.insert(
                    item.uri.clone(),
                    Rc::new(Document {
                        text,
                        stylesheet: OnceCell::new(),
                    }),
                );
                self.trigger_validation(item.uri);
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
                let text = text.update(p.content_changes, p.text_document.version);
                self.documents.insert(
                    uri.clone(),
                    Rc::new(Document {
                        text,
                        stylesheet: OnceCell::new(),
                    }),
                );
                self.trigger_validation(uri);
            }
            "textDocument/didClose" => {
                let p: DidCloseTextDocumentParams = serde_json::from_value(params)?;
                let uri = p.text_document.uri;
                self.documents.remove(&uri);
                self.document_settings.remove(&uri);
                if !self.capabilities.pull_diagnostics {
                    self.pending_validations.remove(&uri);
                    self.publish(uri, Vec::new());
                }
            }
            "workspace/didChangeConfiguration" => {
                let settings = params.get("settings").cloned().unwrap_or(Value::Null);
                if settings.is_null() {
                    anyhow::bail!("Cannot read properties of {settings} (reading 'css')");
                }
                self.configured = settings;
                self.document_settings.clear();
                self.request_refresh();
            }
            "css/customDataChanged" => {
                // `vscode-jsonrpc` sends an array parameter as `[array]`.
                let params = match params {
                    Value::Array(mut a) if a.len() == 1 && a[0].is_array() => a.remove(0),
                    params => params,
                };
                let paths: Vec<String> = serde_json::from_value(params)?;
                let providers: Vec<CssDataProvider> = paths
                    .iter()
                    .map(|p| data_provider(&self.request_service, p))
                    .collect();
                self.data.set_data_providers(true, &providers);
            }
            "$/cancelRequest" => {
                let id: RequestId = serde_json::from_value(params["id"].clone())?;
                let before = self.waiting.len();
                self.waiting
                    .retain(|(_, job)| !matches!(job, Job::Request(r, _) if r.id == id));
                if self.waiting.len() < before {
                    self.respond_cancelled(id);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn request_refresh(&mut self) {
        if self.capabilities.pull_diagnostics {
            self.send_request("workspace/diagnostic/refresh", Value::Null, Outgoing::Other);
        } else {
            let uris: Vec<Uri> = self.documents.keys().cloned().collect();
            for uri in uris {
                self.trigger_validation(uri);
            }
        }
    }

    fn trigger_validation(&mut self, uri: Uri) {
        if self.capabilities.pull_diagnostics {
            return;
        }
        self.pending_validations
            .insert(uri, Instant::now() + VALIDATION_DELAY);
    }

    fn run_due_validations(&mut self) {
        let now = Instant::now();
        let due: Vec<Uri> = self
            .pending_validations
            .iter()
            .filter(|(_, deadline)| **deadline <= now)
            .map(|(uri, _)| uri.clone())
            .collect();
        for uri in due {
            self.pending_validations.remove(&uri);
            let Some(document) = self.document(&uri) else {
                continue;
            };
            match self.scoped_settings(&document.text) {
                Ok(scoped) => self.finish_validation(uri, scoped),
                Err(id) => self.waiting.push((id, Job::Validate(uri))),
            }
        }
    }

    /// Validates the current text, so a change during the wait for the settings does not
    /// publish stale diagnostics. A closed document is not validated.
    fn finish_validation(&mut self, uri: Uri, scoped: Scoped) {
        let Some(document) = self.document(&uri) else {
            return;
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.validate(&document, scoped.as_ref())
        }))
        .map_err(panic_message);
        match result {
            Ok(diagnostics) => self.publish(uri, diagnostics),
            Err(message) => self.log_error(&format!(
                "Error while validating {}: {message}",
                uri.as_str()
            )),
        }
    }

    fn publish(&self, uri: Uri, diagnostics: Vec<Diagnostic>) {
        self.notify(
            "textDocument/publishDiagnostics",
            json!({ "uri": uri, "diagnostics": diagnostics }),
        );
    }

    /// `validateTextDocument`.
    fn validate(&self, document: &Document, scoped: Option<&Value>) -> Vec<Diagnostic> {
        let settings = match scoped {
            None => self.configured(&document.text),
            scoped => scoped,
        };
        let stylesheet = self.stylesheet(document);
        self.dialect(&document.text);
        do_validation(
            &document.text,
            stylesheet,
            validation_settings(settings).as_ref(),
            &self.data,
        )
    }

    fn completion(
        &self,
        p: CompletionParams,
        document: &Document,
        scoped: Option<&Value>,
    ) -> Option<CompactList> {
        let settings = get(scoped, "completion")
            .or_else(|| get(self.configured(&document.text), "completion"));
        let stylesheet = self.stylesheet(document);
        let context = WorkspaceContext {
            document_uri: document.text.uri().as_str(),
            workspace_folders: &self.workspace_folders,
        };
        let completion = Completion {
            dialect: self.dialect(&document.text),
            data: &self.data,
            markdown: self.capabilities.completion_markdown,
        };
        let list = completion.do_complete2(
            &document.text,
            p.text_document_position.position,
            stylesheet,
            &context,
            Some(&self.request_service),
            completion_settings(settings).as_ref(),
            &mut [],
        )?;
        Some(self.resolve.borrow_mut().compact(
            self.capabilities.completion,
            list.is_incomplete,
            list.items,
            list.item_defaults.map(|d| d.edit_range),
        ))
    }

    fn hover(&self, p: HoverParams, document: &Document, scoped: Option<&Value>) -> Option<Hover> {
        let settings =
            get(scoped, "hover").or_else(|| get(self.configured(&document.text), "hover"));
        let stylesheet = self.stylesheet(document);
        self.dialect(&document.text);
        do_hover(
            &document.text,
            p.text_document_position_params.position,
            stylesheet,
            &self.data,
            self.capabilities.hover_markdown,
            hover_settings(settings).as_ref(),
        )
    }

    fn document_symbols(&mut self, p: DocumentSymbolParams) -> Result<Vec<DocumentSymbol>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Vec::new());
        };
        self.dialect(&document.text);
        let mut symbols = find_document_symbols(&document.text, self.stylesheet(&document));
        lsp_transport::cap_symbol_depth(&mut symbols);
        Ok(symbols)
    }

    fn definition(&mut self, p: GotoDefinitionParams) -> Result<Option<Location>, String> {
        let position = p.text_document_position_params;
        let Some(document) = self.document(&position.text_document.uri) else {
            return Ok(None);
        };
        self.dialect(&document.text);
        Ok(find_definition(
            &document.text,
            position.position,
            self.stylesheet(&document),
        ))
    }

    fn document_highlights(
        &mut self,
        p: DocumentHighlightParams,
    ) -> Result<Vec<DocumentHighlight>, String> {
        let position = p.text_document_position_params;
        let Some(document) = self.document(&position.text_document.uri) else {
            return Ok(Vec::new());
        };
        self.dialect(&document.text);
        Ok(find_document_highlights(
            &document.text,
            position.position,
            self.stylesheet(&document),
        ))
    }

    fn document_links(&mut self, p: DocumentLinkParams) -> Result<Vec<DocumentLink>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Vec::new());
        };
        let context = WorkspaceContext {
            document_uri: document.text.uri().as_str(),
            workspace_folders: &self.workspace_folders,
        };
        let stylesheet = self.stylesheet(&document);
        let aliases = alias_settings(get(self.configured(&document.text), "importAliases"));
        let resolver = LinkResolver {
            dialect: self.dialect(&document.text),
            file_system: Some(&self.request_service),
            aliases: aliases.as_ref(),
        };
        Ok(resolver.find_document_links2(&document.text, stylesheet, &context))
    }

    fn references(&mut self, p: ReferenceParams) -> Result<Vec<Location>, String> {
        let position = p.text_document_position;
        let Some(document) = self.document(&position.text_document.uri) else {
            return Ok(Vec::new());
        };
        self.dialect(&document.text);
        Ok(find_references(
            &document.text,
            position.position,
            self.stylesheet(&document),
        ))
    }

    fn code_actions(&mut self, p: CodeActionParams) -> Result<Vec<CodeAction>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Vec::new());
        };
        self.dialect(&document.text);
        Ok(do_code_actions2(
            &document.text,
            &p.context.diagnostics,
            self.stylesheet(&document),
            &self.data,
        ))
    }

    fn document_colors(&mut self, p: DocumentColorParams) -> Result<Vec<ColorInformation>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Vec::new());
        };
        self.dialect(&document.text);
        Ok(find_document_colors(
            &document.text,
            self.stylesheet(&document),
        ))
    }

    fn color_presentations(
        &mut self,
        p: ColorPresentationParams,
    ) -> Result<Vec<ColorPresentation>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Vec::new());
        };
        self.dialect(&document.text);
        self.stylesheet(&document);
        Ok(get_color_presentations(p.color, p.range))
    }

    fn rename(&mut self, p: RenameParams) -> Result<Option<WorkspaceEdit>, String> {
        let position = p.text_document_position;
        let Some(document) = self.document(&position.text_document.uri) else {
            return Ok(None);
        };
        self.dialect(&document.text);
        Ok(Some(do_rename(
            &document.text,
            position.position,
            &p.new_name,
            self.stylesheet(&document),
        )))
    }

    fn folding_ranges(
        &mut self,
        p: FoldingRangeParams,
    ) -> Result<Option<Vec<FoldingRange>>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(None);
        };
        self.dialect(&document.text);
        Ok(Some(get_folding_ranges(
            &document.text,
            self.capabilities.folding_range_limit,
        )))
    }

    fn selection_ranges(&mut self, p: SelectionRangeParams) -> Result<Vec<SelectionRange>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Vec::new());
        };
        self.dialect(&document.text);
        Ok(get_selection_ranges(
            &document.text,
            &p.positions,
            self.stylesheet(&document),
        ))
    }

    /// `onFormat`.
    fn formatting(&mut self, p: FormattingParams) -> Result<Vec<TextEdit>, String> {
        let Some(document) = self.document(&p.text_document.uri) else {
            return Ok(Vec::new());
        };
        let text = &document.text;
        let full_range = text.full_range();
        self.dialect(text);
        let options = format_configuration(&p.options);
        let edits = format(text, Some(p.range.unwrap_or(full_range)), &options);
        if self
            .capabilities
            .formatter_edit_limit
            .is_some_and(|limit| edits.len() > limit)
        {
            let new_text = text.apply_edits(&edits);
            return Ok(vec![TextEdit {
                range: full_range,
                new_text,
            }]);
        }
        Ok(edits)
    }
}
