//! Schema content for the language service. Upstream loads schemas with promises. Here the
//! language service runs synchronously, so a request for content that is not loaded yet fails.
//! The server then runs the operation again when the content arrives.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use json_languageservice::{SchemaRequestError, SchemaRequestService};
use vscode_uri::Uri;

pub type FetchResult = Result<String, SchemaRequestError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Http,
    Client,
}

#[derive(Default)]
struct State {
    content: HashMap<String, FetchResult>,
    in_flight: HashSet<String>,
    to_start: Vec<(String, Source)>,
    missing: HashSet<String>,
}

#[derive(Clone)]
pub struct SchemaStore {
    state: Arc<Mutex<State>>,
    http: bool,
    file: bool,
}

impl SchemaStore {
    /// `handled_protocols`: the `handledSchemaProtocols` initialization option.
    pub fn new(handled_protocols: &[String]) -> Self {
        let handles = |p: &str| handled_protocols.iter().any(|h| h == p);
        SchemaStore {
            state: Arc::default(),
            http: handles("http") || handles("https"),
            file: handles("file"),
        }
    }

    pub fn request_service(&self) -> SchemaRequestService {
        let store = self.clone();
        Box::new(move |uri| store.request(uri))
    }

    fn request(&self, uri: &str) -> FetchResult {
        let scheme = uri.find(':').map_or("", |i| &uri[..i]);
        let http = matches!(scheme, "http" | "https") && self.http;
        if scheme == "file" && self.file {
            return read_file(uri);
        }
        let mut state = self.state.lock().unwrap();
        if let Some(result) = state.content.get(uri) {
            return result.clone();
        }
        state.missing.insert(uri.to_owned());
        if state.in_flight.insert(uri.to_owned()) {
            let source = if http { Source::Http } else { Source::Client };
            state.to_start.push((uri.to_owned(), source));
        }
        Err(SchemaRequestError {
            message: "Loading".to_owned(),
            code: None,
        })
    }

    /// Clears the content requests that were not answered. Call before an operation.
    pub fn begin(&self) {
        self.state.lock().unwrap().missing.clear();
    }

    /// The URIs that the last operation requested but that are not loaded, and the fetches to
    /// start.
    pub fn end(&self) -> (HashSet<String>, Vec<(String, Source)>) {
        let mut state = self.state.lock().unwrap();
        let missing = std::mem::take(&mut state.missing);
        (missing, std::mem::take(&mut state.to_start))
    }

    pub fn loaded(&self, uri: String, result: FetchResult) {
        let mut state = self.state.lock().unwrap();
        state.in_flight.remove(&uri);
        state.content.insert(uri, result);
    }

    /// Forgets the content, so that the next request fetches it again.
    pub fn forget(&self, uri: &str) {
        self.state.lock().unwrap().content.remove(uri);
    }
}

/// `getFileRequestService` of `node/jsonServerMain.ts`.
fn read_file(location: &str) -> FetchResult {
    let error = |message: String| SchemaRequestError {
        message,
        code: None,
    };
    let Some(uri) = Uri::parse(location) else {
        return Err(error(format!("URI malformed: {location}")));
    };
    let path = uri.fs_path();
    match std::fs::read(&path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(error(format!("Schema not found: {location}")))
        }
        Err(_) if std::path::Path::new(&path).is_dir() => {
            Err(error(format!("{location} is a directory, not a file")))
        }
        Err(e) => Err(error(e.to_string())),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpSettings {
    pub proxy: Option<String>,
    pub proxy_strict_ssl: bool,
}

pub fn http_agent(settings: &HttpSettings) -> ureq::Agent {
    let mut config = ureq::Agent::config_builder()
        .max_redirects(5)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(60)));
    if let Some(proxy) = settings.proxy.as_deref().filter(|p| !p.is_empty()) {
        match ureq::Proxy::new(proxy) {
            Ok(proxy) => config = config.proxy(Some(proxy)),
            Err(e) => eprintln!("Invalid proxy '{proxy}': {e}"),
        }
        if !settings.proxy_strict_ssl {
            config = config.tls_config(
                ureq::tls::TlsConfig::builder()
                    .disable_verification(true)
                    .build(),
            );
        }
    }
    config.build().into()
}

/// `getHTTPRequestService` of `node/jsonServerMain.ts`, with the `request-light` messages.
pub fn fetch_http(agent: &ureq::Agent, uri: &str, proxy: bool) -> FetchResult {
    let mut response = match agent.get(uri).call() {
        Ok(response) => response,
        Err(e) => {
            let message = if proxy {
                format!("Unable to connect to {uri} through a proxy. Error: {e}")
            } else {
                format!("Unable to connect to {uri}. Error: {e}")
            };
            return Err(rejection(&message));
        }
    };
    let status = response.status().as_u16();
    let text = match response
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_to_vec()
    {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) => return Err(rejection(&format!("Unable to read {uri}. Error: {e}"))),
    };
    if (200..300).contains(&status) || status == 1223 {
        return Ok(text);
    }
    let message = if !text.is_empty() {
        text
    } else {
        error_status_description(status).unwrap_or_else(|| "[object Object]".to_owned())
    };
    Err(rejection(&message))
}

/// The schema service gets a string rejection, so it takes the text after `Error: ` and
/// removes a final period.
fn rejection(message: &str) -> SchemaRequestError {
    let mut message = message.split("Error: ").nth(1).unwrap_or(message);
    message = message.strip_suffix('.').unwrap_or(message);
    SchemaRequestError {
        message: message.to_owned(),
        code: None,
    }
}

/// `getErrorStatusDescription` of `request-light`.
fn error_status_description(status: u16) -> Option<String> {
    let text = match status {
        0..400 => return None,
        400 => "Bad request. The request cannot be fulfilled due to bad syntax.",
        401 => "Unauthorized. The server is refusing to respond.",
        403 => "Forbidden. The server is refusing to respond.",
        404 => "Not Found. The requested location could not be found.",
        405 => {
            "Method not allowed. A request was made using a request method not supported by that location."
        }
        406 => {
            "Not Acceptable. The server can only generate a response that is not accepted by the client."
        }
        407 => {
            "Proxy Authentication Required. The client must first authenticate itself with the proxy."
        }
        408 => "Request Timeout. The server timed out waiting for the request.",
        409 => "Conflict. The request could not be completed because of a conflict in the request.",
        410 => "Gone. The requested page is no longer available.",
        411 => "Length Required. The \"Content-Length\" is not defined.",
        412 => {
            "Precondition Failed. The precondition given in the request evaluated to false by the server."
        }
        413 => {
            "Request Entity Too Large. The server will not accept the request, because the request entity is too large."
        }
        414 => {
            "Request-URI Too Long. The server will not accept the request, because the URL is too long."
        }
        415 => {
            "Unsupported Media Type. The server will not accept the request, because the media type is not supported."
        }
        500 => "Internal Server Error.",
        501 => {
            "Not Implemented. The server either does not recognize the request method, or it lacks the ability to fulfill the request."
        }
        502 => "Bad Gateway. The upstream server did not respond.",
        503 => "Service Unavailable. The server is currently unavailable (overloaded or down).",
        _ => return Some(format!("HTTP status code {status}")),
    };
    Some(text.to_owned())
}
