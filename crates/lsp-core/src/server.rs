//! The part of the servers that does not depend on the language: the main loop, requests to
//! the client, push diagnostic timers, `runSafe`, and the per-document settings of
//! `workspace/configuration`.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, at, never, select, unbounded};
use lsp_server::{ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::{Diagnostic, DocumentDiagnosticParams, Uri};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::Writer;

const VALIDATION_DELAY: Duration = Duration::from_millis(500);

/// `lsp-types` has no type for this response without `relatedDocuments`.
#[derive(Serialize)]
pub struct FullDiagnosticReport {
    pub kind: &'static str,
    pub items: Vec<Diagnostic>,
}

impl FullDiagnosticReport {
    pub fn new(items: Vec<Diagnostic>) -> Self {
        FullDiagnosticReport {
            kind: "full",
            items,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum DiagnosticReport<'a> {
    Full {
        #[serde(rename = "resultId")]
        result_id: String,
        items: &'a [Diagnostic],
    },
    Unchanged {
        #[serde(rename = "resultId")]
        result_id: String,
    },
}

/// A `textDocument/diagnostic` request.
#[derive(Clone)]
pub struct Pull {
    pub id: RequestId,
    pub uri: Uri,
    previous_result_id: Option<String>,
    /// At the request: settings that arrive later can be older.
    generation: u64,
}

/// Pull diagnostics of the current text of a document, computed at `generation`.
struct Report {
    id: u64,
    generation: u64,
    items: Vec<Diagnostic>,
}

/// `O`: the kind of a request to the client whose response the server reads. `E`: an event
/// from a worker thread.
pub struct Core<O, E = ()> {
    writer: Writer,
    receiver: Receiver<Message>,
    inbox: VecDeque<Message>,
    events: (Sender<E>, Receiver<E>),
    outgoing: HashMap<RequestId, O>,
    next_id: i32,
    pull_diagnostics: bool,
    pending_validations: HashMap<Uri, Instant>,
    /// Changes when the diagnostics of all documents can change: settings, schemas, data.
    generation: u64,
    reports: HashMap<Uri, Report>,
    next_report_id: u64,
    /// Pull diagnostics: changed documents to validate when no message waits.
    idle_validations: VecDeque<Uri>,
    shutdown: bool,
}

impl<O, E> Core<O, E> {
    pub fn new(writer: Writer, receiver: Receiver<Message>, pull_diagnostics: bool) -> Self {
        Core {
            writer,
            receiver,
            inbox: VecDeque::new(),
            events: unbounded(),
            outgoing: HashMap::new(),
            next_id: 0,
            pull_diagnostics,
            pending_validations: HashMap::new(),
            generation: 0,
            reports: HashMap::new(),
            next_report_id: 0,
            idle_validations: VecDeque::new(),
            shutdown: false,
        }
    }

    pub fn pull_diagnostics(&self) -> bool {
        self.pull_diagnostics
    }

    pub fn event_sender(&self) -> Sender<E> {
        self.events.0.clone()
    }

    pub fn send(&self, message: impl Into<Message>) {
        self.writer.message(message.into());
    }

    pub fn respond(&self, id: RequestId, result: &impl Serialize) {
        self.writer.response(id, result);
    }

    pub fn notify(&self, method: &str, params: impl Serialize) {
        self.writer.notification(method, &params);
    }

    pub fn next_id(&mut self) -> i32 {
        self.next_id += 1;
        self.next_id
    }

    /// A request whose response the server does not read.
    pub fn send_request(&mut self, method: &str, params: impl Serialize) -> RequestId {
        let id = RequestId::from(self.next_id());
        self.send(Request::new(id.clone(), method.to_owned(), params));
        id
    }

    /// A request whose response goes to `Service::on_response` with `kind`.
    pub fn send_tracked(&mut self, method: &str, params: impl Serialize, kind: O) -> RequestId {
        let id = self.send_request(method, params);
        self.outgoing.insert(id.clone(), kind);
        id
    }

    pub fn log_error(&self, message: &str) {
        self.notify(
            "window/logMessage",
            json!({ "type": 1, "message": message }),
        );
    }

    pub fn respond_cancelled(&self, id: RequestId) {
        self.send(Response::new_err(
            id,
            ErrorCode::RequestCanceled as i32,
            "Request cancelled".to_owned(),
        ));
    }

    /// The params of `request`. A parse error is the response.
    pub fn params<P: DeserializeOwned>(&self, request: &Request) -> Option<P> {
        match serde_json::from_value(request.params.clone()) {
            Ok(params) => Some(params),
            Err(e) => {
                self.send(Response::new_err(
                    request.id.clone(),
                    ErrorCode::InvalidParams as i32,
                    e.to_string(),
                ));
                None
            }
        }
    }

    /// The `runSafe` error: a log message, and `error_value` as the result.
    pub fn fail(&self, request: Request, error_value: Value, action: &str, message: &str) {
        let uri = request
            .params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .unwrap_or_default();
        self.log_error(&format!("Error while {action} {uri}: {message}"));
        self.send(Response::new_ok(request.id, error_value));
    }

    pub fn publish(&self, uri: Uri, diagnostics: Vec<Diagnostic>) {
        self.notify(
            "textDocument/publishDiagnostics",
            json!({ "uri": uri, "diagnostics": diagnostics }),
        );
    }

    /// The text of `uri` changed. Push diagnostics: validates it after a delay. Pull
    /// diagnostics: drops its report and validates it when no message waits.
    pub fn trigger_validation(&mut self, uri: Uri) {
        if self.pull_diagnostics {
            self.reports.remove(&uri);
            if !self.idle_validations.contains(&uri) {
                self.idle_validations.push_back(uri);
            }
        } else {
            self.pending_validations
                .insert(uri, Instant::now() + VALIDATION_DELAY);
        }
    }

    /// `workspace/diagnostic/refresh`, or for push diagnostics, a validation of `uris`.
    pub fn request_refresh<'a>(&mut self, uris: impl Iterator<Item = &'a Uri>) {
        self.invalidate_diagnostics();
        if self.pull_diagnostics {
            self.send_request("workspace/diagnostic/refresh", Value::Null);
        } else {
            for uri in uris {
                self.trigger_validation(uri.clone());
            }
        }
    }

    /// The diagnostics of all documents can change. No refresh is sent.
    pub fn invalidate_diagnostics(&mut self) {
        self.generation += 1;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether the report of `uri` has the diagnostics of its current text and settings.
    pub fn report_is_current(&self, uri: &Uri) -> bool {
        self.reports
            .get(uri)
            .is_some_and(|r| r.generation == self.generation)
    }

    /// Keeps `items`, the diagnostics of the current text of `uri` at `generation`. The
    /// `resultId` stays when the items are equal to the last ones.
    pub fn store_report(&mut self, uri: Uri, generation: u64, items: Vec<Diagnostic>) {
        if let Some(report) = self.reports.get_mut(&uri)
            && report.items == items
        {
            report.generation = report.generation.max(generation);
            return;
        }
        self.next_report_id += 1;
        let report = Report {
            id: self.next_report_id,
            generation,
            items,
        };
        self.reports.insert(uri, report);
    }

    /// The pull of `request`. `None`: the params are not valid, and the error is the response.
    pub fn pull(&self, request: &Request) -> Option<Pull> {
        let p: DocumentDiagnosticParams = self.params(request)?;
        Some(Pull {
            id: request.id.clone(),
            uri: p.text_document.uri,
            previous_result_id: p.previous_result_id,
            generation: self.generation,
        })
    }

    /// Responds to `pull` for an open document. `items`: the diagnostics of its current
    /// text, or `None` for the current report. `unchanged` when the client has the report.
    pub fn respond_pull(&mut self, pull: Pull, items: Option<Result<Vec<Diagnostic>, String>>) {
        match items {
            Some(Ok(items)) => self.store_report(pull.uri.clone(), pull.generation, items),
            Some(Err(message)) => {
                self.log_error(&format!(
                    "Error while computing diagnostics for {}: {message}",
                    pull.uri.as_str()
                ));
                return self.respond(pull.id, &FullDiagnosticReport::new(Vec::new()));
            }
            None => {}
        }
        let Some(report) = self.reports.get(&pull.uri) else {
            return self.respond(pull.id, &FullDiagnosticReport::new(Vec::new()));
        };
        let result_id = report.id.to_string();
        let response = if pull.previous_result_id.as_ref() == Some(&result_id) {
            DiagnosticReport::Unchanged { result_id }
        } else {
            DiagnosticReport::Full {
                result_id,
                items: &report.items,
            }
        };
        self.respond(pull.id, &response);
    }

    /// The next changed document without a current report.
    pub fn next_idle_validation(&mut self) -> Option<Uri> {
        while let Some(uri) = self.idle_validations.pop_front() {
            if !self.report_is_current(&uri) {
                return Some(uri);
            }
        }
        None
    }

    /// Stops the work for a closed document. Push diagnostics: clears its diagnostics.
    pub fn close(&mut self, uri: Uri) {
        if self.pull_diagnostics {
            self.reports.remove(&uri);
            self.idle_validations.retain(|u| *u != uri);
        } else {
            self.pending_validations.remove(&uri);
            self.publish(uri, Vec::new());
        }
    }

    fn due_validations(&mut self) -> Vec<Uri> {
        let now = Instant::now();
        let due: Vec<Uri> = self
            .pending_validations
            .iter()
            .filter(|(_, deadline)| **deadline <= now)
            .map(|(uri, _)| uri.clone())
            .collect();
        for uri in &due {
            self.pending_validations.remove(uri);
        }
        due
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
}

pub trait Service: Sized {
    type Outgoing;
    type Event;

    fn core(&mut self) -> &mut Core<Self::Outgoing, Self::Event>;

    /// A request other than `shutdown`, not cancelled.
    fn dispatch(&mut self, request: Request);

    /// A notification other than `exit`. An error is logged.
    fn on_notification(&mut self, notification: Notification) -> anyhow::Result<()>;

    /// The response to a `Core::send_tracked` request.
    fn on_response(&mut self, _response: Response, _kind: Self::Outgoing) {}

    /// Push diagnostics: the delay of `Core::trigger_validation` is over.
    fn validate(&mut self, uri: Uri);

    fn on_event(&mut self, _event: Self::Event) {}

    /// Work while no message waits. Returns `false` when there is none.
    fn idle(&mut self) -> bool {
        false
    }
}

/// The main loop. Returns whether the client sent `shutdown` before `exit`.
pub fn run<S: Service>(service: &mut S) -> bool {
    loop {
        let core = service.core();
        let message = match core.inbox.pop_front() {
            Some(message) => message,
            None => {
                if core.receiver.is_empty() && core.events.1.is_empty() && service.idle() {
                    continue;
                }
                let core = service.core();
                let receiver = core.receiver.clone();
                let events = core.events.1.clone();
                let timer = match core.pending_validations.values().min() {
                    Some(&deadline) => at(deadline),
                    None => never(),
                };
                select! {
                    recv(receiver) -> message => match message {
                        Ok(message) => message,
                        Err(_) => return service.core().shutdown,
                    },
                    recv(events) -> event => {
                        if let Ok(event) = event {
                            service.on_event(event);
                        }
                        continue;
                    }
                    recv(timer) -> _ => {
                        for uri in service.core().due_validations() {
                            service.validate(uri);
                        }
                        continue;
                    }
                }
            }
        };
        let core = service.core();
        match message {
            Message::Request(request) if request.method == "shutdown" => {
                core.shutdown = true;
                core.send(Response::new_ok(request.id, Value::Null));
            }
            Message::Request(request) => {
                if core.take_cancellation(&request.id) {
                    core.respond_cancelled(request.id);
                } else {
                    service.dispatch(request);
                }
            }
            Message::Notification(notification) if notification.method == "exit" => {
                return core.shutdown;
            }
            Message::Notification(notification) => {
                let method = notification.method.clone();
                if let Err(e) = service.on_notification(notification) {
                    service
                        .core()
                        .log_error(&format!("Error while handling {method}: {e}"));
                }
            }
            Message::Response(response) => {
                if let Some(kind) = core.outgoing.remove(&response.id) {
                    service.on_response(response, kind);
                }
            }
        }
    }
}

/// `runSafe`: runs `f` on the params of `request`. An error or a panic gives `error_value`
/// and a log message.
pub fn handle<S: Service, P: DeserializeOwned, R: Serialize>(
    service: &mut S,
    request: Request,
    error_value: Value,
    action: &str,
    f: impl FnOnce(&mut S, P) -> Result<R, String>,
) {
    let Some(params) = service.core().params(&request) else {
        return;
    };
    match catch(|| f(service, params)).and_then(|r| r) {
        Ok(result) => service.core().respond(request.id, &result),
        Err(message) => service.core().fail(request, error_value, action, &message),
    }
}

/// Runs `f`; a panic gives its message.
pub fn catch<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
        payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_owned())
    })
}

/// `getClientCapability`.
pub fn capability<'a>(capabilities: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(capabilities, |c, key| c.as_object()?.get(key))
}

pub fn contains_markdown(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|f| f == "markdown"))
}

/// `sanitizeLimitSetting(value || default)`: `0` and non-numbers are `default`; `None` is
/// `Number.MAX_VALUE`.
pub fn limit(value: Option<&Value>, default: Option<usize>) -> Option<usize> {
    match value.and_then(Value::as_f64) {
        Some(n) if n != 0.0 && !n.is_nan() => {
            let n = n.max(0.0).trunc();
            (n < usize::MAX as f64).then_some(n as usize)
        }
        _ => default,
    }
}

/// A `vscode-jsonrpc` client sends a single parameter that is not an object as `[param]`.
/// With `only_arrays`, only `[[...]]` is unwrapped.
pub fn unwrap_positional(params: Value, only_arrays: bool) -> Value {
    match params {
        Value::Array(mut a) if a.len() == 1 && (!only_arrays || a[0].is_array()) => a.remove(0),
        params => params,
    }
}

/// The settings of a document from `workspace/configuration`. `None` selects the global
/// settings.
pub type Scoped = Option<Value>;

enum DocumentSettings {
    Pending(RequestId),
    Ready(Scoped),
}

/// `getDocumentSettings`: the `workspace/configuration` result of each document, and the
/// jobs `J` that wait for it.
pub struct ScopedSettings<J> {
    documents: HashMap<Uri, DocumentSettings>,
    waiting: Vec<(RequestId, J)>,
}

impl<J> Default for ScopedSettings<J> {
    fn default() -> Self {
        ScopedSettings {
            documents: HashMap::new(),
            waiting: Vec::new(),
        }
    }
}

impl<J> ScopedSettings<J> {
    /// The settings of `uri`, or `job` waits for them. The first call for a document sends
    /// `workspace/configuration` with `items`, tracked as `kind`.
    pub fn get<O, E>(
        &mut self,
        core: &mut Core<O, E>,
        uri: &Uri,
        items: impl FnOnce() -> Vec<Value>,
        kind: impl FnOnce() -> O,
        job: impl FnOnce() -> J,
    ) -> Option<Scoped> {
        let id = match self.documents.get(uri) {
            Some(DocumentSettings::Ready(scoped)) => return Some(scoped.clone()),
            Some(DocumentSettings::Pending(id)) => id.clone(),
            None => {
                let id = core.send_tracked(
                    "workspace/configuration",
                    json!({ "items": items() }),
                    kind(),
                );
                self.documents
                    .insert(uri.clone(), DocumentSettings::Pending(id.clone()));
                id
            }
        };
        self.waiting.push((id, job()));
        None
    }

    /// The settings of `uri` if the client sent them. No request is sent.
    pub fn ready(&self, uri: &Uri) -> Option<Scoped> {
        match self.documents.get(uri) {
            Some(DocumentSettings::Ready(scoped)) => Some(scoped.clone()),
            _ => None,
        }
    }

    /// The response `id` for `uri`: `None` when the request failed, which is not cached, so
    /// the next request asks again. Returns the jobs that waited for it.
    pub fn resolve(&mut self, uri: Uri, id: &RequestId, scoped: Option<&Scoped>) -> Vec<J> {
        if matches!(self.documents.get(&uri), Some(DocumentSettings::Pending(p)) if p == id) {
            match scoped {
                Some(scoped) => {
                    self.documents
                        .insert(uri, DocumentSettings::Ready(scoped.clone()));
                }
                None => {
                    self.documents.remove(&uri);
                }
            }
        }
        let (ready, waiting) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|(i, _)| i == id);
        self.waiting = waiting;
        ready.into_iter().map(|(_, job)| job).collect()
    }

    /// Removes the waiting jobs that `cancelled` selects. Returns whether there was one.
    pub fn cancel(&mut self, cancelled: impl Fn(&J) -> bool) -> bool {
        let before = self.waiting.len();
        self.waiting.retain(|(_, job)| !cancelled(job));
        self.waiting.len() < before
    }

    pub fn remove(&mut self, uri: &Uri) {
        self.documents.remove(uri);
    }

    pub fn clear(&mut self) {
        self.documents.clear();
    }
}
