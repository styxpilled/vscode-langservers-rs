//! Port of `jsonServer.ts` and `utils/validation.ts`.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::Receiver;
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
    DidOpenTextDocumentParams, DocumentColorParams, DocumentFormattingParams, DocumentLink,
    DocumentLinkParams, DocumentRangeFormattingParams, DocumentSymbolParams,
    DocumentSymbolResponse, FoldingRange, FoldingRangeParams, Hover, HoverParams, Range,
    SelectionRange, SelectionRangeParams, TextDocumentIdentifier, TextEdit, Uri,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vscode_uri::resolve_relative_path;

use crate::schema_request::{
    Cached, FetchResult, HttpResult, HttpSettings, SchemaStore, Source, cache_dir, fetch_http,
    http_agent, write_cache,
};
use lsp_core::server::{
    Core, FullDiagnosticReport, Service, capability, catch, limit, unwrap_positional,
};
use lsp_core::{DEFAULT_COLOR_LIMIT, DEFAULT_RESULT_LIMIT, Writer};

/// `ColorPresentationParams` with the `f64` `Color`: Zed sends `f32` values as
/// text, and Node reads them as doubles.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ColorPresentationParams {
    text_document: TextDocumentIdentifier,
    color: Color,
    range: Range,
}

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
}

enum Job {
    Request(Request),
    Validate(Uri),
    Prefetch(Uri, bool),
}

/// An operation that waits for schema content.
struct Deferred {
    waiting: HashSet<String>,
    job: Job,
}

/// `vscode/content` for a URI.
pub struct Outgoing(String);

/// Schema content from a worker thread. The bool is true for a schema that changed after the
/// server used its cached copy.
pub struct Fetched(String, FetchResult, bool);

pub struct Server {
    core: Core<Outgoing, Fetched>,
    ls: LanguageService,
    store: SchemaStore,
    cache_dir: Option<PathBuf>,
    http: HttpSettings,
    agent: ureq::Agent,
    documents: HashMap<Uri, Document>,
    capabilities: Capabilities,
    settings: Settings,
    last_settings: Option<Value>,
    formatter_registered: bool,
    deferred: Vec<Deferred>,
    /// Documents whose schema is loaded and compiled when no message waits. With the flag,
    /// a refresh follows.
    prefetch: VecDeque<(Uri, bool)>,
    refresh_after_prefetch: bool,
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

fn severity(value: Option<&Value>) -> Option<SeverityLevel> {
    match value?.as_str()? {
        "error" => Some(SeverityLevel::Error),
        "warning" => Some(SeverityLevel::Warning),
        "ignore" => Some(SeverityLevel::Ignore),
        _ => None,
    }
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
    ls.do_validation(
        document,
        json,
        Some(&document_settings(settings, document)),
        None,
    )
}

fn validate_syntax(
    ls: &LanguageService,
    settings: &Settings,
    document: &TextDocument,
    json: &JsonDocument,
) -> Vec<Diagnostic> {
    if document.text().is_empty() {
        return Vec::new();
    }
    // A schema can allow comments and trailing commas.
    let settings = DocumentLanguageSettings {
        comments: Some(SeverityLevel::Ignore),
        trailing_commas: Some(SeverityLevel::Ignore),
        ..document_settings(settings, document)
    };
    ls.do_syntax_validation(document, json, Some(&settings))
}

fn document_settings(settings: &Settings, document: &TextDocument) -> DocumentLanguageSettings {
    let jsonc = document.language_id() == "jsonc";
    DocumentLanguageSettings {
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
    }
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
        let cache_dir = cache_dir();
        let store = SchemaStore::new(&handled_protocols, cache_dir.clone());
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

        let pull_diagnostics = capability(caps, "textDocument.diagnostic").is_some();
        let http = HttpSettings::default();
        let folding = capabilities.folding_range_limit;
        let server = Server {
            core: Core::new(writer, receiver, pull_diagnostics),
            ls,
            store,
            cache_dir,
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
            last_settings: None,
            formatter_registered: false,
            deferred: Vec::new(),
            prefetch: VecDeque::new(),
            refresh_after_prefetch: false,
        };
        (server, result)
    }

    /// Returns whether the client sent `shutdown` before `exit`.
    pub fn run(mut self) -> bool {
        lsp_core::server::run(&mut self)
    }

    /// Runs `f` and starts the schema fetches that it needs. Returns the URIs of the schemas
    /// that are not loaded yet, or the panic message.
    fn with_schemas<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> T,
    ) -> Result<Result<T, HashSet<String>>, String> {
        self.store.begin();
        let result = catch(|| f(self));
        let (missing, to_start) = self.store.end();
        for (uri, source) in to_start {
            self.start_fetch(uri, source);
        }
        let result = result?;
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
            Source::Http(cached) => {
                let agent = self.agent.clone();
                let proxy = self.http.proxy.as_deref().is_some_and(|p| !p.is_empty());
                let tx = self.core.event_sender();
                let cache = self.cache_dir.clone();
                std::thread::spawn(move || {
                    let etag = cached.as_ref().map(|c| c.etag.as_str());
                    let result = match fetch_http(&agent, &uri, proxy, etag) {
                        HttpResult::Ok(content, etag) => {
                            let fresh = Cached {
                                etag: etag.unwrap_or_default(),
                                content,
                            };
                            let same = cached.as_ref().is_some_and(|c| {
                                c.content == fresh.content && c.etag == fresh.etag
                            });
                            if let Some(dir) = cache.as_deref().filter(|_| !same) {
                                write_cache(dir, &uri, &fresh);
                            }
                            if cached.as_ref().is_some_and(|c| c.content == fresh.content) {
                                return;
                            }
                            Ok(fresh.content)
                        }
                        // A cached copy stays in use when the check fails.
                        _ if cached.is_some() => return,
                        HttpResult::Fetched(result) => result,
                        HttpResult::NotModified => unreachable!(),
                    };
                    let _ = tx.send(Fetched(uri, result, cached.is_some()));
                });
            }
            Source::Client => {
                // `vscode-jsonrpc` sends a single parameter that is not an object as an array.
                self.core
                    .send_tracked("vscode/content", [&uri], Outgoing(uri.clone()));
            }
        }
    }

    /// Content that replaces the cached copy, as with `json/schemaContent`.
    fn on_changed(&mut self, uri: String, result: FetchResult) {
        let reset = self.reset_schemas(&[&uri]);
        self.store.loaded(uri, result);
        if reset {
            self.request_refresh();
        }
    }

    fn on_fetched(&mut self, uri: String, result: FetchResult) {
        self.store.loaded(uri.clone(), result);
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
                Job::Prefetch(uri, refresh) => self.queue_prefetch(uri, refresh),
            }
        }
    }

    fn queue_prefetch(&mut self, uri: Uri, refresh: bool) {
        match self.prefetch.iter_mut().find(|(u, _)| *u == uri) {
            Some((_, r)) => *r |= refresh,
            None => self.prefetch.push_back((uri, refresh)),
        }
    }

    fn prefetch_all(&mut self) {
        let uris: Vec<Uri> = self.documents.keys().cloned().collect();
        for uri in uris {
            self.queue_prefetch(uri, false);
        }
    }

    /// Loads and compiles the schema of the next queued document.
    fn prefetch_next(&mut self) {
        let Some((uri, refresh)) = self.prefetch.pop_front() else {
            return;
        };
        let result = self.with_schemas(|s| {
            if let Some(document) = s.documents.get(&uri) {
                s.ls.schema_service()
                    .get_schema_for_resource(uri.as_str(), Some(document.json()));
            }
        });
        match result {
            Ok(Ok(())) => self.refresh_after_prefetch |= refresh,
            Ok(Err(waiting)) => self.defer_prefetch(uri, refresh, waiting),
            Err(message) => {
                self.refresh_after_prefetch |= refresh;
                self.core.log_error(&format!(
                    "Error while loading the schema for {}: {message}",
                    uri.as_str()
                ));
            }
        }
        if self.prefetch.is_empty() && std::mem::take(&mut self.refresh_after_prefetch) {
            self.request_refresh();
        }
    }

    fn request(&mut self, mut request: Request) {
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
            "textDocument/diagnostic" if self.core.pull_diagnostics() => {
                self.pull_diagnostics(request)
            }
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
            _ => self.core.send(Response::new_err(
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
        let Some(params) = self.core.params(&request) else {
            return;
        };
        match self.with_schemas(|s| f(s, params)) {
            Ok(Ok(result)) => self.core.respond(request.id, &result),
            Ok(Err(waiting)) => self.deferred.push(Deferred {
                waiting,
                job: Job::Request(request),
            }),
            Err(message) => {
                self.core
                    .fail(request, error_value, &format!("{action} for"), &message)
            }
        }
    }

    fn pull_diagnostics(&mut self, request: Request) {
        let Some(pull) = self.core.pull(&request) else {
            return;
        };
        if !self.documents.contains_key(&pull.uri) {
            return self
                .core
                .respond(pull.id, &FullDiagnosticReport::new(Vec::new()));
        }
        if self.core.report_is_current(&pull.uri) {
            return self.core.respond_pull(pull, None);
        }
        match self.validate_open(&pull.uri) {
            Ok(Ok(items)) => self.core.respond_pull(pull, Some(Ok(items))),
            Ok(Err(waiting)) => self.diagnostics_without_schema(pull.id, pull.uri, waiting),
            Err(message) => self.core.respond_pull(pull, Some(Err(message))),
        }
    }

    /// `Err`: the panic message. `Ok(Err)`: the schemas that are not loaded yet.
    fn validate_open(
        &mut self,
        uri: &Uri,
    ) -> Result<Result<Vec<Diagnostic>, HashSet<String>>, String> {
        self.with_schemas(|s| {
            let document = &s.documents[uri];
            validate(&mut s.ls, &s.settings, &document.text, document.json())
        })
    }

    /// A pull does not wait for schema content: it gets the syntax diagnostics without a
    /// `resultId`, and a refresh follows when the schema is loaded and compiled.
    fn diagnostics_without_schema(&mut self, id: RequestId, uri: Uri, waiting: HashSet<String>) {
        let document = &self.documents[&uri];
        let items = validate_syntax(&self.ls, &self.settings, &document.text, document.json());
        self.core.respond(id, &FullDiagnosticReport::new(items));
        self.defer_prefetch(uri, true, waiting);
    }

    fn defer_prefetch(&mut self, uri: Uri, refresh: bool, waiting: HashSet<String>) {
        let existing = self
            .deferred
            .iter_mut()
            .find(|d| matches!(&d.job, Job::Prefetch(u, _) if *u == uri));
        match existing {
            Some(Deferred {
                waiting: w,
                job: Job::Prefetch(_, r),
            }) => {
                w.extend(waiting);
                *r |= refresh;
            }
            _ => self.deferred.push(Deferred {
                waiting,
                job: Job::Prefetch(uri, refresh),
            }),
        }
    }

    fn notification(&mut self, notification: Notification) -> anyhow::Result<()> {
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
                self.queue_prefetch(item.uri.clone(), false);
                self.core.trigger_validation(item.uri);
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
                self.core.trigger_validation(uri);
            }
            "textDocument/didClose" => {
                let p: DidCloseTextDocumentParams = serde_json::from_value(params)?;
                let uri = p.text_document.uri;
                self.documents.remove(&uri);
                self.deferred.retain(
                    |d| !matches!(&d.job, Job::Validate(u) | Job::Prefetch(u, _) if *u == uri),
                );
                self.prefetch.retain(|(u, _)| *u != uri);
                self.core.close(uri);
            }
            "workspace/didChangeConfiguration" => self.on_configuration(&params["settings"]),
            "json/schemaAssociations" => {
                self.settings.associations = Some(params);
                self.update_configuration(Vec::new());
                self.prefetch_all();
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
                    self.core.respond_cancelled(id);
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
        if needs_revalidation {
            self.prefetch_all();
        }
        needs_revalidation
    }

    fn on_configuration(&mut self, settings: &Value) {
        if self.last_settings.as_ref() == Some(settings) {
            return;
        }
        self.last_settings = Some(settings.clone());
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
        self.prefetch_all();

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
                    self.core.send_request(
                        "client/registerCapability",
                        json!({ "registrations": registrations }),
                    );
                }
            } else if self.formatter_registered {
                self.formatter_registered = false;
                let unregistrations: Vec<Value> = methods
                    .iter()
                    .map(|(id, method)| json!({ "id": id, "method": method }))
                    .collect();
                // The misspelling is from the LSP specification.
                self.core.send_request(
                    "client/unregisterCapability",
                    json!({ "unregisterations": unregistrations }),
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
        self.core.request_refresh(self.documents.keys());
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
            Ok(Ok(Some(diagnostics))) => self.core.publish(uri, diagnostics),
            Ok(Ok(None)) => {}
            Ok(Err(waiting)) => self.deferred.push(Deferred {
                waiting,
                job: Job::Validate(uri),
            }),
            Err(message) => self.core.log_error(&format!(
                "Error while validating {}: {message}",
                uri.as_str()
            )),
        }
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
            lsp_core::cap_symbol_depth(&mut symbols);
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

impl Service for Server {
    type Outgoing = Outgoing;
    type Event = Fetched;

    fn core(&mut self) -> &mut Core<Outgoing, Fetched> {
        &mut self.core
    }

    fn dispatch(&mut self, request: Request) {
        self.request(request);
    }

    fn on_notification(&mut self, notification: Notification) -> anyhow::Result<()> {
        self.notification(notification)
    }

    fn on_response(&mut self, response: Response, Outgoing(uri): Outgoing) {
        let result = match response.response_result {
            Err(error) => Err(SchemaRequestError {
                message: error.message,
                code: Some(error.code.into()),
            }),
            Ok(result) => Ok(result.as_str().unwrap_or_default().to_owned()),
        };
        self.on_fetched(uri, result);
    }

    fn validate(&mut self, uri: Uri) {
        self.validate_and_publish(uri);
    }

    fn on_event(&mut self, Fetched(uri, result, changed): Fetched) {
        if changed {
            self.on_changed(uri, result);
        } else {
            self.on_fetched(uri, result);
        }
    }

    /// Schema loads first: a validation needs the schema.
    fn idle(&mut self) -> bool {
        if !self.prefetch.is_empty() {
            self.prefetch_next();
            return true;
        }
        let Some(uri) = self.core.next_idle_validation() else {
            return false;
        };
        // The pull logs an error. A missing schema loads, and the pull then validates.
        if self.documents.contains_key(&uri) {
            let generation = self.core.generation();
            if let Ok(Ok(items)) = self.validate_open(&uri) {
                self.core.store_report(uri, generation, items);
            }
        }
        true
    }
}
