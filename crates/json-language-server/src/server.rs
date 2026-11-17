//! Port of `jsonServer.ts` and `utils/validation.ts`.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender, at, never, select};
use json_languageservice::{
    Color, ColorInformation, DocumentLanguageSettings, FormattingOptions, JsonDocument,
    LanguageService, LanguageServiceParams, LanguageSettings, SchemaConfiguration,
    SchemaRequestError, SeverityLevel, TextDocument, find_document_symbols, find_document_symbols2,
    find_links, get_color_presentations, get_folding_ranges, get_selection_ranges, normalize_id,
    parse,
};
use lsp_server::{ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    CodeActionParams, ColorPresentation, CompletionList, CompletionParams, Diagnostic,
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentColorParams, DocumentDiagnosticParams,
    DocumentFormattingParams, DocumentLink, DocumentLinkParams, DocumentRangeFormattingParams,
    DocumentSymbolParams, DocumentSymbolResponse, FoldingRange, FoldingRangeParams, Hover,
    HoverParams, Range, SelectionRange, SelectionRangeParams, TextDocumentIdentifier, TextEdit,
    Uri,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vscode_uri::resolve_relative_path;

use crate::schema_request::{
    FetchResult, HttpSettings, SchemaStore, Source, fetch_http, http_agent,
};
use lsp_transport::{DEFAULT_COLOR_LIMIT, DEFAULT_RESULT_LIMIT, Writer};

/// `ColorPresentationParams` with the `f64` `Color`: Zed sends `f32` values as
/// text, and Node reads them as doubles.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ColorPresentationParams {
    text_document: TextDocumentIdentifier,
    color: Color,
    range: Range,
}

const VALIDATION_DELAY: Duration = Duration::from_millis(500);
const DEFAULT_FORMATTER_EDIT_LIMIT: usize = 10000;
const SORT_CODE_ACTION_KIND: &str = "source.sort.json";

struct Document {
    text: TextDocument,
    json: OnceCell<JsonDocument>,
}

impl Document {
    fn new(text: TextDocument) -> Self {
        Document {
            text,
            json: OnceCell::new(),
        }
    }

    fn json(&self) -> &JsonDocument {
        self.json.get_or_init(|| parse(&self.text, true))
    }
}

/// `None` limits are `Number.MAX_VALUE` upstream.
#[derive(Default)]
struct Settings {
    schemas: Option<Value>,
    associations: Option<Value>,
    validate: bool,
    comments: Option<SeverityLevel>,
    trailing_commas: Option<SeverityLevel>,
    schema_validation: Option<SeverityLevel>,
    schema_request: Option<SeverityLevel>,
    keep_lines: bool,
    result_limit: Option<usize>,
    json_folding_limit: Option<usize>,
    jsonc_folding_limit: Option<usize>,
    json_color_limit: Option<usize>,
    jsonc_color_limit: Option<usize>,
}

struct Capabilities {
    dynamic_formatter_registration: bool,
    hierarchical_document_symbols: bool,
    folding_range_limit: Option<usize>,
    formatter_edit_limit: Option<usize>,
    pull_diagnostics: bool,
}

enum Job {
    Request(Request),
    Validate(Uri),
}

/// An operation that waits for schema content.
struct Deferred {
    waiting: HashSet<String>,
    job: Job,
}

/// `lsp-types` has no type for this response without `relatedDocuments`.
#[derive(Serialize)]
struct FullDiagnosticReport {
    kind: &'static str,
    items: Vec<Diagnostic>,
}

enum Outgoing {
    Content(String),
    Other,
}

pub struct Server {
    writer: Writer,
    receiver: Receiver<Message>,
    inbox: VecDeque<Message>,
    ls: LanguageService,
    store: SchemaStore,
    fetched_tx: Sender<(String, FetchResult)>,
    fetched_rx: Receiver<(String, FetchResult)>,
    http: HttpSettings,
    agent: ureq::Agent,
    documents: HashMap<Uri, Document>,
    capabilities: Capabilities,
    settings: Settings,
    formatter_registered: bool,
    /// Set when schema content changes. The next schema use reloads all schemas, which
    /// frees the old schema graph.
    schemas_stale: bool,
    pending_validations: HashMap<Uri, Instant>,
    deferred: Vec<Deferred>,
    outgoing: HashMap<RequestId, Outgoing>,
    next_id: i32,
    shutdown: bool,
}

/// `getClientCapability`.
fn capability<'a>(capabilities: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(capabilities, |c, key| c.as_object()?.get(key))
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `sanitizeLimitSetting(value || default)`.
fn limit(value: Option<&Value>, default: Option<usize>) -> Option<usize> {
    match value.and_then(Value::as_f64) {
        Some(n) if n != 0.0 && !n.is_nan() => {
            let n = n.max(0.0).trunc();
            (n < usize::MAX as f64).then_some(n as usize)
        }
        _ => default,
    }
}

fn severity(value: Option<&Value>) -> Option<SeverityLevel> {
    match value?.as_str()? {
        "error" => Some(SeverityLevel::Error),
        "warning" => Some(SeverityLevel::Warning),
        "ignore" => Some(SeverityLevel::Ignore),
        _ => None,
    }
}

/// A `vscode-jsonrpc` client sends a single parameter that is not an object as `[param]`.
/// With `only_arrays`, only `[[...]]` is unwrapped.
fn unwrap_positional(params: Value, only_arrays: bool) -> Value {
    match params {
        Value::Array(mut a) if a.len() == 1 && (!only_arrays || a[0].is_array()) => a.remove(0),
        params => params,
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".to_owned())
}

fn schema_configuration(value: &Value) -> Option<SchemaConfiguration> {
    Some(SchemaConfiguration {
        uri: value.get("uri")?.as_str()?.to_owned(),
        file_match: string_array(value.get("fileMatch")),
        schema: value.get("schema").cloned(),
        folder_uri: value
            .get("folderUri")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    Some(
        value?
            .as_array()?
            .iter()
            .filter_map(|s| Some(s.as_str()?.to_owned()))
            .collect(),
    )
}

fn validate(
    ls: &mut LanguageService,
    settings: &Settings,
    document: &TextDocument,
    json: &JsonDocument,
) -> Vec<Diagnostic> {
    if document.text().is_empty() {
        return Vec::new();
    }
    let jsonc = document.language_id() == "jsonc";
    let document_settings = DocumentLanguageSettings {
        comments: settings.comments.or(Some(if jsonc {
            SeverityLevel::Ignore
        } else {
            SeverityLevel::Error
        })),
        trailing_commas: settings.trailing_commas.or(Some(if jsonc {
            SeverityLevel::Warning
        } else {
            SeverityLevel::Error
        })),
        schema_validation: settings.schema_validation,
        schema_request: settings.schema_request,
        schema_draft: None,
    };
    ls.do_validation(document, json, Some(&document_settings), None)
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
        let handled_protocols = string_array(options.get("handledSchemaProtocols"))
            .unwrap_or_else(|| vec!["https".into(), "http".into(), "file".into()]);
        let store = SchemaStore::new(&handled_protocols);
        let ls = LanguageService::new(LanguageServiceParams {
            schema_request_service: Some(store.request_service()),
            workspace_context: Some(Box::new(resolve_relative_path)),
            fallback_schema_provider: None,
            client_capabilities: serde_json::from_value(caps.clone()).unwrap_or_default(),
        });

        let snippet_support = truthy(capability(
            caps,
            "textDocument.completion.completionItem.snippetSupport",
        ));
        let provide_formatter = options.get("provideFormatter");
        let capabilities = Capabilities {
            dynamic_formatter_registration: truthy(capability(
                caps,
                "textDocument.rangeFormatting.dynamicRegistration",
            )) && !provide_formatter.is_some_and(Value::is_boolean),
            hierarchical_document_symbols: truthy(capability(
                caps,
                "textDocument.documentSymbol.hierarchicalDocumentSymbolSupport",
            )),
            folding_range_limit: limit(
                capability(caps, "textDocument.foldingRange.rangeLimit"),
                Some(DEFAULT_RESULT_LIMIT),
            ),
            // VS Code sends 10000. Other clients get the same limit, because a response with
            // millions of edits is too large for them.
            formatter_edit_limit: limit(
                options.pointer("/customCapabilities/rangeFormatting/editLimit"),
                Some(DEFAULT_FORMATTER_EDIT_LIMIT),
            ),
            pull_diagnostics: capability(caps, "textDocument.diagnostic").is_some(),
        };
        let provide_formatter = provide_formatter == Some(&Value::Bool(true));
        let mut result = json!({
            "capabilities": {
                "textDocumentSync": 2,
                "hoverProvider": true,
                "documentSymbolProvider": true,
                "documentRangeFormattingProvider": provide_formatter,
                "documentFormattingProvider": provide_formatter,
                "colorProvider": {},
                "foldingRangeProvider": true,
                "selectionRangeProvider": true,
                "documentLinkProvider": {},
                "diagnosticProvider": {
                    "documentSelector": null,
                    "interFileDependencies": false,
                    "workspaceDiagnostics": false
                },
                "codeActionProvider": { "codeActionKinds": [SORT_CODE_ACTION_KIND] }
            },
            "serverInfo": {
                "name": env!("CARGO_BIN_NAME"),
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        if snippet_support {
            result["capabilities"]["completionProvider"] = json!({
                "resolveProvider": false,
                "triggerCharacters": ["\"", ":"]
            });
        }

        let (fetched_tx, fetched_rx) = crossbeam_channel::unbounded();
        let http = HttpSettings::default();
        let folding = capabilities.folding_range_limit;
        let server = Server {
            writer,
            receiver,
            inbox: VecDeque::new(),
            ls,
            store,
            fetched_tx,
            fetched_rx,
            agent: http_agent(&http),
            http,
            documents: HashMap::new(),
            capabilities,
            settings: Settings {
                validate: true,
                result_limit: Some(DEFAULT_RESULT_LIMIT),
                json_folding_limit: folding,
                jsonc_folding_limit: folding,
                json_color_limit: Some(DEFAULT_COLOR_LIMIT),
                jsonc_color_limit: Some(DEFAULT_COLOR_LIMIT),
                ..Default::default()
            },
            formatter_registered: false,
            schemas_stale: false,
            pending_validations: HashMap::new(),
            deferred: Vec::new(),
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
                    let fetched = self.fetched_rx.clone();
                    let timer = match self.pending_validations.values().min() {
                        Some(&deadline) => at(deadline),
                        None => never(),
                    };
                    select! {
                        recv(receiver) -> message => match message {
                            Ok(message) => message,
                            Err(_) => return self.shutdown,
                        },
                        recv(fetched) -> result => {
                            if let Ok((uri, result)) = result {
                                self.on_fetched(uri, result);
                            }
                            continue;
                        }
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

    fn send_request(&mut self, method: &str, params: impl Serialize, kind: Outgoing) {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.outgoing.insert(id.clone(), kind);
        self.send(Request::new(id, method.to_owned(), params));
    }

    fn log_error(&self, message: &str) {
        self.notify(
            "window/logMessage",
            json!({ "type": 1, "message": message }),
        );
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

    /// Runs `f` and starts the schema fetches that it needs. Returns the URIs of the schemas
    /// that are not loaded yet, or the panic message.
    fn with_schemas<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> T,
    ) -> Result<Result<T, HashSet<String>>, String> {
        if std::mem::take(&mut self.schemas_stale) {
            self.ls.reload_schemas();
        }
        self.store.begin();
        let result = catch_unwind(AssertUnwindSafe(|| f(self)));
        let (missing, to_start) = self.store.end();
        for (uri, source) in to_start {
            self.start_fetch(uri, source);
        }
        let result = result.map_err(panic_message)?;
        if missing.is_empty() {
            return Ok(Ok(result));
        }
        // The failed loads are cached in the schema handles.
        for uri in &missing {
            self.ls.reset_schema(uri);
        }
        Ok(Err(missing))
    }

    fn start_fetch(&mut self, uri: String, source: Source) {
        match source {
            Source::Http => {
                let agent = self.agent.clone();
                let proxy = self.http.proxy.as_deref().is_some_and(|p| !p.is_empty());
                let tx = self.fetched_tx.clone();
                std::thread::spawn(move || {
                    let result = fetch_http(&agent, &uri, proxy);
                    let _ = tx.send((uri, result));
                });
            }
            Source::Client => {
                // `vscode-jsonrpc` sends a single parameter that is not an object as an array.
                self.send_request("vscode/content", [&uri], Outgoing::Content(uri.clone()));
            }
        }
    }

    fn on_fetched(&mut self, uri: String, result: FetchResult) {
        self.store.loaded(uri.clone(), result);
        // `reset_schema` would load the dependent schemas again into the graph and keep the
        // old copies.
        self.schemas_stale = true;
        let mut ready = Vec::new();
        for deferred in std::mem::take(&mut self.deferred) {
            let mut deferred = deferred;
            deferred.waiting.remove(&uri);
            if deferred.waiting.is_empty() {
                ready.push(deferred.job);
            } else {
                self.deferred.push(deferred);
            }
        }
        for job in ready {
            match job {
                Job::Request(request) => self.dispatch(request),
                Job::Validate(uri) => self.validate_and_publish(uri),
            }
        }
    }

    fn on_response(&mut self, response: Response) {
        let Some(Outgoing::Content(uri)) = self.outgoing.remove(&response.id) else {
            return;
        };
        let result = match response.response_result {
            Err(error) => Err(SchemaRequestError {
                message: error.message,
                code: Some(error.code.into()),
            }),
            Ok(result) => Ok(result.as_str().unwrap_or_default().to_owned()),
        };
        self.on_fetched(uri, result);
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

    fn dispatch(&mut self, mut request: Request) {
        if matches!(
            request.method.as_str(),
            "json/validate" | "json/languageStatus"
        ) {
            request.params = unwrap_positional(request.params, false);
        }
        let empty = || json!([]);
        match request.method.as_str() {
            "textDocument/completion" => self.handle(
                request,
                Value::Null,
                "computing completions",
                Self::completion,
            ),
            "textDocument/hover" => {
                self.handle(request, Value::Null, "computing hover", Self::hover)
            }
            "textDocument/documentSymbol" => self.handle(
                request,
                empty(),
                "computing document symbols",
                Self::document_symbols,
            ),
            "textDocument/codeAction" => self.handle(
                request,
                empty(),
                "computing code actions",
                Self::code_actions,
            ),
            "textDocument/rangeFormatting" => {
                self.handle(request, empty(), "formatting range", Self::range_formatting)
            }
            "textDocument/formatting" => {
                self.handle(request, empty(), "formatting", Self::formatting)
            }
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
            "textDocument/documentLink" => {
                self.handle(request, empty(), "computing links", Self::document_links)
            }
            "textDocument/diagnostic" if self.capabilities.pull_diagnostics => self.handle(
                request,
                json!({ "kind": "full", "items": [] }),
                "computing diagnostics",
                Self::pull_diagnostics,
            ),
            "json/validate" => self.handle(request, empty(), "validating", Self::force_validate),
            "json/validateAll" => self.handle(request, Value::Null, "validating", |s, _: Value| {
                s.request_refresh()
            }),
            "json/validateContent" => {
                self.handle(request, empty(), "validating", Self::validate_content)
            }
            "json/languageStatus" => self.handle(
                request,
                json!({ "schemas": [] }),
                "computing language status",
                Self::language_status,
            ),
            "json/sort" => self.handle(request, empty(), "sorting", Self::sort),
            _ => self.send(Response::new_err(
                request.id,
                ErrorCode::MethodNotFound as i32,
                format!("Unhandled method {}", request.method),
            )),
        }
    }

    /// `runSafe`: a panic gives `error_value`.
    fn handle<P: DeserializeOwned, R: Serialize>(
        &mut self,
        request: Request,
        error_value: Value,
        action: &str,
        f: fn(&mut Self, P) -> R,
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
        match self.with_schemas(|s| f(s, params)) {
            Ok(Ok(result)) => self.writer.response(request.id, &result),
            Ok(Err(waiting)) => self.deferred.push(Deferred {
                waiting,
                job: Job::Request(request),
            }),
            Err(message) => {
                let uri = request
                    .params
                    .pointer("/textDocument/uri")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.log_error(&format!("Error while {action} for {uri}: {message}"));
                self.send(Response::new_ok(request.id, error_value));
            }
        }
    }

    fn on_notification(&mut self, notification: Notification) {
        let method = notification.method.clone();
        if let Err(e) = self.try_on_notification(notification) {
            self.log_error(&format!("Error while handling {method}: {e}"));
        }
    }

    fn try_on_notification(&mut self, notification: Notification) -> anyhow::Result<()> {
        let params = match notification.method.as_str() {
            "json/schemaContent" => unwrap_positional(notification.params, false),
            "json/schemaAssociations" => unwrap_positional(notification.params, true),
            _ => notification.params,
        };
        match notification.method.as_str() {
            "textDocument/didOpen" => {
                let p: DidOpenTextDocumentParams = serde_json::from_value(params)?;
                let item = p.text_document;
                let text =
                    TextDocument::new(item.uri.clone(), item.language_id, item.version, item.text);
                self.documents.insert(item.uri.clone(), Document::new(text));
                self.trigger_validation(item.uri);
            }
            "textDocument/didChange" => {
                let p: DidChangeTextDocumentParams = serde_json::from_value(params)?;
                let uri = p.text_document.uri;
                let Some(document) = self.documents.remove(&uri) else {
                    return Ok(());
                };
                let text = document
                    .text
                    .update(p.content_changes, p.text_document.version);
                self.documents.insert(uri.clone(), Document::new(text));
                self.trigger_validation(uri);
            }
            "textDocument/didClose" => {
                let p: DidCloseTextDocumentParams = serde_json::from_value(params)?;
                let uri = p.text_document.uri;
                self.documents.remove(&uri);
                self.deferred
                    .retain(|d| !matches!(&d.job, Job::Validate(u) if *u == uri));
                if !self.capabilities.pull_diagnostics {
                    self.pending_validations.remove(&uri);
                    self.publish(uri, Vec::new());
                }
            }
            "workspace/didChangeConfiguration" => self.on_configuration(&params["settings"]),
            "json/schemaAssociations" => {
                self.settings.associations = Some(params);
                self.update_configuration(Vec::new());
            }
            "json/schemaContent" => {
                let uris = match params {
                    Value::Array(uris) => uris,
                    uri => vec![uri],
                };
                let uris: Vec<&str> = uris.iter().filter_map(Value::as_str).collect();
                if self.reset_schemas(&uris) {
                    self.request_refresh();
                }
            }
            "workspace/didChangeWatchedFiles" => {
                let p: DidChangeWatchedFilesParams = serde_json::from_value(params)?;
                let uris: Vec<&str> = p.changes.iter().map(|c| c.uri.as_str()).collect();
                if self.reset_schemas(&uris) {
                    self.request_refresh();
                }
            }
            "$/cancelRequest" => {
                let id: RequestId = serde_json::from_value(params["id"].clone())?;
                let before = self.deferred.len();
                self.deferred
                    .retain(|d| !matches!(&d.job, Job::Request(r) if r.id == id));
                if self.deferred.len() < before {
                    self.respond_cancelled(id);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn reset_schemas(&mut self, uris: &[&str]) -> bool {
        let mut needs_revalidation = false;
        for uri in uris {
            self.store.forget(uri);
            self.store.forget(&normalize_id(uri));
            if self.ls.reset_schema(uri) {
                needs_revalidation = true;
            }
        }
        self.schemas_stale |= needs_revalidation;
        needs_revalidation
    }

    fn on_configuration(&mut self, settings: &Value) {
        let http = HttpSettings {
            proxy: settings
                .pointer("/http/proxy")
                .and_then(Value::as_str)
                .map(str::to_owned),
            proxy_strict_ssl: truthy(settings.pointer("/http/proxyStrictSSL")),
        };
        if http != self.http {
            self.agent = http_agent(&http);
            self.http = http;
        }
        let get = |path: &str| settings.pointer(&format!("/json{path}"));
        self.settings.schemas = get("/schemas").cloned();
        self.settings.validate = truthy(get("/validate/enable"));
        self.settings.comments = severity(get("/validate/comments"));
        self.settings.trailing_commas = severity(get("/validate/trailingCommas"));
        self.settings.schema_validation = severity(get("/validate/schemaValidation"));
        self.settings.schema_request = severity(get("/validate/schemaRequest"));
        self.settings.keep_lines = truthy(get("/keepLines/enable"));
        self.update_configuration(Vec::new());

        let folding = self.capabilities.folding_range_limit;
        self.settings.result_limit = limit(get("/resultLimit"), Some(DEFAULT_RESULT_LIMIT));
        self.settings.json_folding_limit = limit(get("/jsonFoldingLimit"), folding);
        self.settings.jsonc_folding_limit = limit(get("/jsoncFoldingLimit"), folding);
        self.settings.json_color_limit =
            limit(get("/jsonColorDecoratorLimit"), Some(DEFAULT_COLOR_LIMIT));
        self.settings.jsonc_color_limit =
            limit(get("/jsoncColorDecoratorLimit"), Some(DEFAULT_COLOR_LIMIT));

        if self.capabilities.dynamic_formatter_registration {
            let methods = [
                ("json-range-formatting", "textDocument/rangeFormatting"),
                ("json-formatting", "textDocument/formatting"),
            ];
            if truthy(get("/format/enable")) {
                if !self.formatter_registered {
                    self.formatter_registered = true;
                    let selector = json!([{ "language": "json" }, { "language": "jsonc" }]);
                    let registrations: Vec<Value> = methods
                        .iter()
                        .map(|(id, method)| {
                            json!({ "id": id, "method": method, "registerOptions": { "documentSelector": selector } })
                        })
                        .collect();
                    self.send_request(
                        "client/registerCapability",
                        json!({ "registrations": registrations }),
                        Outgoing::Other,
                    );
                }
            } else if self.formatter_registered {
                self.formatter_registered = false;
                let unregistrations: Vec<Value> = methods
                    .iter()
                    .map(|(id, method)| json!({ "id": id, "method": method }))
                    .collect();
                // The misspelling is from the LSP specification.
                self.send_request(
                    "client/unregisterCapability",
                    json!({ "unregisterations": unregistrations }),
                    Outgoing::Other,
                );
            }
        }
    }

    fn update_configuration(&mut self, extra_schemas: Vec<SchemaConfiguration>) {
        let mut schemas = Vec::new();
        match &self.settings.associations {
            Some(Value::Array(associations)) => {
                schemas.extend(associations.iter().filter_map(schema_configuration));
            }
            Some(Value::Object(associations)) => {
                for (pattern, uris) in associations {
                    for uri in uris
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        schemas.push(SchemaConfiguration {
                            uri: uri.to_owned(),
                            file_match: Some(vec![pattern.clone()]),
                            ..Default::default()
                        });
                    }
                }
            }
            _ => {}
        }
        if let Some(Value::Array(settings)) = &self.settings.schemas {
            for (index, setting) in settings.iter().enumerate() {
                let schema = setting.get("schema").filter(|s| !s.is_null());
                let mut uri = setting
                    .get("url")
                    .and_then(Value::as_str)
                    .filter(|u| !u.is_empty())
                    .map(str::to_owned);
                if uri.is_none()
                    && let Some(schema) = schema
                {
                    uri = Some(
                        schema
                            .get("id")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                            .map_or_else(
                                || format!("vscode://schemas/custom/{index}"),
                                str::to_owned,
                            ),
                    );
                }
                if let Some(uri) = uri {
                    schemas.push(SchemaConfiguration {
                        uri,
                        file_match: string_array(setting.get("fileMatch")),
                        schema: schema.cloned(),
                        folder_uri: setting
                            .get("folderUri")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    });
                }
            }
        }
        schemas.extend(extra_schemas);
        self.ls.configure(&LanguageSettings {
            validate: Some(self.settings.validate),
            allow_comments: Some(true),
            schemas,
        });
        self.request_refresh();
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
        if !self.capabilities.pull_diagnostics {
            self.pending_validations
                .insert(uri, Instant::now() + VALIDATION_DELAY);
        }
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
            self.validate_and_publish(uri);
        }
    }

    fn validate_and_publish(&mut self, uri: Uri) {
        let result = self.with_schemas(|s| {
            let document = s.documents.get(&uri)?;
            Some(validate(
                &mut s.ls,
                &s.settings,
                &document.text,
                document.json(),
            ))
        });
        match result {
            Ok(Ok(Some(diagnostics))) => self.publish(uri, diagnostics),
            Ok(Ok(None)) => {}
            Ok(Err(waiting)) => self.deferred.push(Deferred {
                waiting,
                job: Job::Validate(uri),
            }),
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

    fn completion(&mut self, p: CompletionParams) -> Option<CompletionList> {
        let position = p.text_document_position;
        let document = self.documents.get(&position.text_document.uri)?;
        Some(
            self.ls
                .do_complete(&document.text, position.position, document.json()),
        )
    }

    fn hover(&mut self, p: HoverParams) -> Option<Hover> {
        let position = p.text_document_position_params;
        let document = self.documents.get(&position.text_document.uri)?;
        self.ls
            .do_hover(&document.text, position.position, document.json())
    }

    fn document_symbols(&mut self, p: DocumentSymbolParams) -> DocumentSymbolResponse {
        let Some(document) = self.documents.get(&p.text_document.uri) else {
            return DocumentSymbolResponse::Flat(Vec::new());
        };
        let limit = self.settings.result_limit;
        if self.capabilities.hierarchical_document_symbols {
            let mut symbols = find_document_symbols2(&document.text, document.json(), limit);
            lsp_transport::cap_symbol_depth(&mut symbols);
            DocumentSymbolResponse::Nested(symbols)
        } else {
            DocumentSymbolResponse::Flat(find_document_symbols(
                &document.text,
                document.json(),
                limit,
            ))
        }
    }

    fn code_actions(&mut self, p: CodeActionParams) -> Value {
        if !self.documents.contains_key(&p.text_document.uri) {
            return json!([]);
        }
        json!([{
            "title": "Sort JSON",
            "kind": SORT_CODE_ACTION_KIND,
            "command": { "title": "Sort JSON", "command": "json.sort" }
        }])
    }

    fn format(
        &self,
        uri: &Uri,
        range: Option<Range>,
        options: lsp_types::FormattingOptions,
    ) -> Vec<TextEdit> {
        let Some(document) = self.documents.get(uri) else {
            return Vec::new();
        };
        let full_range = document.text.full_range();
        let options = FormattingOptions {
            tab_size: options.tab_size,
            insert_spaces: options.insert_spaces,
            insert_final_newline: options.insert_final_newline == Some(true),
            keep_lines: self.settings.keep_lines,
        };
        let edits = json_languageservice::format(
            &document.text,
            Some(range.unwrap_or(full_range)),
            &options,
        );
        if self
            .capabilities
            .formatter_edit_limit
            .is_some_and(|limit| edits.len() > limit)
        {
            let new_text = document.text.apply_edits(&edits);
            return vec![TextEdit {
                range: full_range,
                new_text,
            }];
        }
        edits
    }

    fn range_formatting(&mut self, p: DocumentRangeFormattingParams) -> Vec<TextEdit> {
        self.format(&p.text_document.uri, Some(p.range), p.options)
    }

    fn formatting(&mut self, p: DocumentFormattingParams) -> Vec<TextEdit> {
        self.format(&p.text_document.uri, None, p.options)
    }

    fn document_colors(&mut self, p: DocumentColorParams) -> Vec<ColorInformation> {
        let Some(document) = self.documents.get(&p.text_document.uri) else {
            return Vec::new();
        };
        let limit = if document.text.language_id() == "jsonc" {
            self.settings.jsonc_color_limit
        } else {
            self.settings.json_color_limit
        };
        self.ls
            .find_document_colors(&document.text, document.json(), limit)
    }

    fn color_presentations(&mut self, p: ColorPresentationParams) -> Vec<ColorPresentation> {
        if !self.documents.contains_key(&p.text_document.uri) {
            return Vec::new();
        }
        get_color_presentations(p.color, p.range)
    }

    fn folding_ranges(&mut self, p: FoldingRangeParams) -> Option<Vec<FoldingRange>> {
        let document = self.documents.get(&p.text_document.uri)?;
        let limit = if document.text.language_id() == "jsonc" {
            self.settings.jsonc_folding_limit
        } else {
            self.settings.json_folding_limit
        };
        Some(get_folding_ranges(&document.text, limit))
    }

    fn selection_ranges(&mut self, p: SelectionRangeParams) -> Vec<SelectionRange> {
        let Some(document) = self.documents.get(&p.text_document.uri) else {
            return Vec::new();
        };
        get_selection_ranges(&document.text, &p.positions, document.json())
    }

    fn document_links(&mut self, p: DocumentLinkParams) -> Vec<DocumentLink> {
        let Some(document) = self.documents.get(&p.text_document.uri) else {
            return Vec::new();
        };
        find_links(&document.text, document.json())
    }

    fn pull_diagnostics(&mut self, p: DocumentDiagnosticParams) -> FullDiagnosticReport {
        let items = match self.documents.get(&p.text_document.uri) {
            Some(document) => validate(
                &mut self.ls,
                &self.settings,
                &document.text,
                document.json(),
            ),
            None => Vec::new(),
        };
        FullDiagnosticReport {
            kind: "full",
            items,
        }
    }

    fn force_validate(&mut self, uri: Uri) -> Vec<Diagnostic> {
        if !self.documents.contains_key(&uri) {
            return Vec::new();
        }
        self.update_configuration(Vec::new());
        let document = &self.documents[&uri];
        validate(
            &mut self.ls,
            &self.settings,
            &document.text,
            document.json(),
        )
    }

    fn validate_content(&mut self, p: Value) -> Vec<Diagnostic> {
        let (Some(schema_uri), Some(content)) = (p["schemaUri"].as_str(), p["content"].as_str())
        else {
            return Vec::new();
        };
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let doc_uri = format!("vscode://schemas/temp/{millis}");
        let Ok(uri) = doc_uri.parse() else {
            return Vec::new();
        };
        let document = TextDocument::new(uri, "json", 1, content.to_owned());
        self.update_configuration(vec![SchemaConfiguration {
            uri: schema_uri.to_owned(),
            file_match: Some(vec![doc_uri]),
            ..Default::default()
        }]);
        let json = parse(&document, true);
        validate(&mut self.ls, &self.settings, &document, &json)
    }

    fn language_status(&mut self, uri: Uri) -> Value {
        let schemas = match self.documents.get(&uri) {
            Some(document) => self.ls.get_language_status(&document.text, document.json()),
            None => Vec::new(),
        };
        json!({ "schemas": schemas })
    }

    fn sort(&mut self, p: Value) -> Vec<TextEdit> {
        let Some(uri) = p["uri"].as_str().and_then(|u| u.parse::<Uri>().ok()) else {
            return Vec::new();
        };
        let Some(document) = self.documents.get(&uri) else {
            return Vec::new();
        };
        let options = &p["options"];
        let options = FormattingOptions {
            tab_size: options["tabSize"].as_u64().unwrap_or(0) as u32,
            insert_spaces: options["insertSpaces"] == true,
            insert_final_newline: options["insertFinalNewline"] == true,
            keep_lines: options["keepLines"] == true,
        };
        json_languageservice::sort(&document.text, &options)
    }
}
