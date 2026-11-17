//! Runs the server binary and talks LSP over stdio.

use std::io::{BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lsp_server::{Message, Notification, Request, RequestId, Response};
use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Message>,
    next_id: i32,
}

impl Client {
    fn start(capabilities: Value, options: Value) -> Client {
        Client::start_with_cache(capabilities, options, "")
    }

    fn start_with_cache(capabilities: Value, options: Value, cache: &str) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_vscode-json-language-server"))
            .arg("--stdio")
            .env("JSON_SCHEMA_CACHE", cache)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, messages) = channel();
        std::thread::spawn(move || {
            while let Ok(Some(message)) = Message::read(&mut stdout) {
                if tx.send(message).is_err() {
                    break;
                }
            }
        });
        let mut client = Client {
            child,
            stdin,
            messages,
            next_id: 0,
        };
        let result = client.request(
            "initialize",
            json!({ "capabilities": capabilities, "initializationOptions": options }),
        );
        assert!(result["capabilities"].is_object());
        client.notify("initialized", json!({}));
        client
    }

    fn send(&mut self, message: impl Into<Message>) {
        message.into().write(&mut self.stdin).unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(Notification::new(method.to_owned(), params));
    }

    fn send_request(&mut self, method: &str, params: Value) -> RequestId {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.send(Request::new(id.clone(), method.to_owned(), params));
        id
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send_request(method, params);
        self.response(id)
    }

    fn response(&mut self, id: RequestId) -> Value {
        loop {
            match self.recv() {
                Message::Response(r) if r.id == id => return r.response_result.unwrap(),
                Message::Request(r) => panic!("unexpected request {r:?}"),
                _ => {}
            }
        }
    }

    fn recv(&self) -> Message {
        self.messages
            .recv_timeout(Duration::from_secs(10))
            .expect("no message from the server")
    }

    /// Waits for the server request `method`, skipping other messages.
    fn server_request(&self, method: &str) -> Request {
        loop {
            if let Message::Request(r) = self.recv()
                && r.method == method
            {
                return r;
            }
        }
    }

    fn diagnostics(&self, uri: &str) -> Vec<Value> {
        loop {
            if let Message::Notification(n) = self.recv()
                && n.method == "textDocument/publishDiagnostics"
                && n.params["uri"] == uri
            {
                return n.params["diagnostics"].as_array().unwrap().clone();
            }
        }
    }

    fn open(&mut self, uri: &str, language_id: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": language_id, "version": 1, "text": text } }),
        );
    }

    fn exit(mut self) -> i32 {
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        self.child.wait().unwrap().code().unwrap()
    }
}

fn messages(diagnostics: &[Value]) -> Vec<&str> {
    diagnostics
        .iter()
        .map(|d| d["message"].as_str().unwrap())
        .collect()
}

fn push_client() -> Client {
    let capabilities = json!({
        "textDocument": {
            "completion": { "completionItem": { "snippetSupport": true } },
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true }
        }
    });
    let mut client = Client::start(capabilities, json!({ "provideFormatter": true }));
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "json": {
            "validate": { "enable": true },
            "schemas": [
                {
                    "fileMatch": ["inline.json"],
                    "schema": { "properties": { "a": { "type": "number", "description": "The a." } } }
                },
                { "fileMatch": ["remote.json"], "url": "zed://schemas/remote" }
            ]
        } } }),
    );
    client
}

#[test]
fn push_diagnostics_and_features() {
    let mut client = push_client();

    client.open("file:///inline.json", "json", "{ \"a\": \"x\" // c\n}");
    let diagnostics = client.diagnostics("file:///inline.json");
    assert_eq!(
        messages(&diagnostics),
        [
            "Incorrect type. Expected \"number\".",
            "Comments are not permitted in JSON."
        ]
    );

    // Incremental change: replace `"x"` with `1` and remove the comment.
    client.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": "file:///inline.json", "version": 2 },
            "contentChanges": [
                { "range": { "start": { "line": 0, "character": 7 }, "end": { "line": 0, "character": 10 } }, "text": "1" },
                { "range": { "start": { "line": 0, "character": 8 }, "end": { "line": 0, "character": 13 } }, "text": "" }
            ]
        }),
    );
    assert_eq!(
        client.diagnostics("file:///inline.json"),
        Vec::<Value>::new()
    );

    let hover = client.request(
        "textDocument/hover",
        json!({ "textDocument": { "uri": "file:///inline.json" }, "position": { "line": 0, "character": 3 } }),
    );
    assert_eq!(hover["contents"][0], r"The a\.");

    let edits = client.request(
        "textDocument/formatting",
        json!({ "textDocument": { "uri": "file:///inline.json" }, "options": { "tabSize": 2, "insertSpaces": true } }),
    );
    assert!(!edits.as_array().unwrap().is_empty());

    let symbols = client.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": "file:///inline.json" } }),
    );
    assert_eq!(symbols[0]["name"], "a");
    assert!(symbols[0]["selectionRange"].is_object());

    client.notify(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": "file:///inline.json" } }),
    );
    assert_eq!(
        client.diagnostics("file:///inline.json"),
        Vec::<Value>::new()
    );
    assert_eq!(client.exit(), 0);
}

#[test]
fn client_schema_content() {
    let mut client = push_client();

    // The hover waits for the schema from the client.
    client.open("file:///remote.json", "json", "{ \"b\": 1 }");
    let hover = client.send_request(
        "textDocument/hover",
        json!({ "textDocument": { "uri": "file:///remote.json" }, "position": { "line": 0, "character": 3 } }),
    );
    let request = client.server_request("vscode/content");
    assert_eq!(request.params, json!(["zed://schemas/remote"]));
    let schema = json!({ "properties": { "b": { "type": "string", "description": "The b." } } });
    client.send(Response::new_ok(request.id, schema.to_string()));
    assert_eq!(client.response(hover)["contents"][0], r"The b\.");
    assert_eq!(
        messages(&client.diagnostics("file:///remote.json")),
        ["Incorrect type. Expected \"string\"."]
    );

    // A changed schema is requested again.
    client.notify("json/schemaContent", json!("zed://schemas/remote"));
    let request = client.server_request("vscode/content");
    client.send(Response::new_err(request.id, -32603, "gone".to_owned()));
    assert_eq!(
        messages(&client.diagnostics("file:///remote.json")),
        ["Unable to load schema from 'zed://schemas/remote': gone."]
    );
    assert_eq!(client.exit(), 0);
}

#[test]
fn pull_diagnostics() {
    let capabilities = json!({ "textDocument": { "diagnostic": {} } });
    let mut client = Client::start(capabilities, json!({}));
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "json": { "validate": { "enable": true } } } }),
    );
    client.server_request("workspace/diagnostic/refresh");
    client.open("file:///a.jsonc", "jsonc", "[1,]");
    let report = client.request(
        "textDocument/diagnostic",
        json!({ "textDocument": { "uri": "file:///a.jsonc" } }),
    );
    assert_eq!(report["kind"], "full");
    assert_eq!(report["items"][0]["message"], "Trailing comma");
    assert_eq!(report["items"][0]["severity"], 2);
    assert_eq!(client.exit(), 0);
}

/// `docs/experimental.md`: a pull with the `resultId` of the current report gets
/// `unchanged`. A change of the text or of the diagnostics gives a new `resultId`.
#[test]
fn diagnostic_result_ids() {
    let capabilities = json!({ "textDocument": { "diagnostic": {} } });
    let mut client = Client::start(capabilities, json!({}));
    let settings = |enable: bool, format: bool| json!({ "settings": { "json": { "validate": { "enable": enable }, "format": { "enable": format } } } });
    client.notify("workspace/didChangeConfiguration", settings(true, false));
    client.server_request("workspace/diagnostic/refresh");
    client.open("file:///a.json", "json", "{");
    let pull = |client: &mut Client, previous: &Value| {
        client.request(
            "textDocument/diagnostic",
            json!({ "textDocument": { "uri": "file:///a.json" }, "previousResultId": previous }),
        )
    };
    let first = pull(&mut client, &Value::Null);
    assert_eq!(first["kind"], "full");
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    let id = &first["resultId"];
    assert_eq!(
        pull(&mut client, id),
        json!({ "kind": "unchanged", "resultId": id })
    );
    // The report without `previousResultId`.
    assert_eq!(pull(&mut client, &Value::Null), first);

    let change = |client: &mut Client, version: i32, text: &str| {
        client.notify(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": "file:///a.json", "version": version }, "contentChanges": [{ "text": text }] }),
        );
    };
    change(&mut client, 2, "{}");
    let empty = pull(&mut client, id);
    assert_eq!(empty["items"], json!([]));
    assert_ne!(&empty["resultId"], id);
    let id = &empty["resultId"];

    // Other settings, the same diagnostics.
    client.notify("workspace/didChangeConfiguration", settings(true, true));
    client.server_request("workspace/diagnostic/refresh");
    assert_eq!(pull(&mut client, id)["kind"], "unchanged");

    change(&mut client, 3, "{");
    let error = pull(&mut client, id);
    assert_eq!(error["items"].as_array().unwrap().len(), 1);
    let id = &error["resultId"];
    client.notify("workspace/didChangeConfiguration", settings(false, true));
    client.server_request("workspace/diagnostic/refresh");
    let disabled = pull(&mut client, id);
    assert_eq!(disabled["items"], json!([]));
    assert_ne!(&disabled["resultId"], id);

    // A closed document has no report.
    client.notify(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": "file:///a.json" } }),
    );
    assert_eq!(
        pull(&mut client, &disabled["resultId"]),
        json!({ "kind": "full", "items": [] })
    );
    assert_eq!(client.exit(), 0);
}

/// `docs/experimental.md`: the schema load starts at `didOpen`, and a pull does not wait
/// for it. A refresh follows when the schema is ready.
#[test]
fn pull_diagnostics_without_schema() {
    let capabilities = json!({ "textDocument": { "diagnostic": {} } });
    let mut client = Client::start(capabilities, json!({}));
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "json": {
            "validate": { "enable": true },
            "schemas": [{ "fileMatch": ["remote.json"], "url": "zed://schemas/remote" }]
        } } }),
    );
    client.server_request("workspace/diagnostic/refresh");
    // The schema can allow trailing commas, so the first answer has only syntax errors.
    client.open("file:///remote.json", "json", "{ \"b\": 1, } x");
    let content = client.server_request("vscode/content");
    let document = json!({ "textDocument": { "uri": "file:///remote.json" } });
    let report = client.request("textDocument/diagnostic", document.clone());
    assert_eq!(
        messages(report["items"].as_array().unwrap()),
        ["End of file expected."]
    );
    // Not a complete report.
    assert!(report.get("resultId").is_none());

    let schema = json!({ "properties": { "b": { "type": "string" } } });
    client.send(Response::new_ok(content.id, schema.to_string()));
    client.server_request("workspace/diagnostic/refresh");
    let report = client.request("textDocument/diagnostic", document);
    assert_eq!(
        messages(report["items"].as_array().unwrap()),
        [
            "Incorrect type. Expected \"string\".",
            "Trailing comma",
            "End of file expected."
        ]
    );
    assert_eq!(client.exit(), 0);
}

#[test]
fn exit_without_shutdown() {
    let mut client = Client::start(json!({}), json!({}));
    client.notify("exit", Value::Null);
    assert_eq!(client.child.wait().unwrap().code(), Some(1));
}

const BOOLEAN_SCHEMA: &str = r#"{ "properties": { "c": { "type": "boolean" } } }"#;

/// (`ETag`, body)
type Schema = Arc<Mutex<(&'static str, &'static str)>>;

/// Serves `schema` at `/ok.json`, with 304 for a matching `If-None-Match`, and 404 with an
/// empty body for other paths. Returns the port and the `If-None-Match` of each request.
fn http_server(schema: Schema) -> (u16, Arc<Mutex<Vec<Option<String>>>>) {
    use std::io::BufRead;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut if_none_match = None;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("if-none-match")
                {
                    if_none_match = Some(value.trim().to_owned());
                }
            }
            let (etag, body) = *schema.lock().unwrap();
            let response = if !request_line.contains("/ok.json") {
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned()
            } else if if_none_match.as_deref() == Some(etag) {
                "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".to_owned()
            } else {
                format!(
                    "HTTP/1.1 200 OK\r\nETag: {etag}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            log.lock().unwrap().push(if_none_match);
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    (port, requests)
}

#[test]
fn http_schemas() {
    let (port, _) = http_server(Arc::new(Mutex::new(("\"v1\"", BOOLEAN_SCHEMA))));
    let mut client = Client::start(json!({}), json!({}));
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "json": {
            "validate": { "enable": true },
            "schemas": [
                { "fileMatch": ["ok.json"], "url": format!("http://127.0.0.1:{port}/ok.json") },
                { "fileMatch": ["missing.json"], "url": format!("http://127.0.0.1:{port}/missing.json") }
            ]
        } } }),
    );
    client.open("file:///ok.json", "json", "{ \"c\": 1 }");
    assert_eq!(
        messages(&client.diagnostics("file:///ok.json")),
        ["Incorrect type. Expected \"boolean\"."]
    );
    client.open("file:///missing.json", "json", "{}");
    assert_eq!(
        messages(&client.diagnostics("file:///missing.json")),
        [format!(
            "Unable to load schema from 'http://127.0.0.1:{port}/missing.json': Not Found. The requested location could not be found."
        )]
    );
    assert_eq!(client.exit(), 0);
}

/// `docs/experimental.md`: the server uses the cached copy of an HTTP schema, then checks it
/// with `If-None-Match`.
#[test]
fn http_schema_cache() {
    let string = r#"{ "properties": { "c": { "type": "string" } } }"#;
    let schema = Arc::new(Mutex::new(("\"v1\"", BOOLEAN_SCHEMA)));
    let (port, requests) = http_server(schema.clone());
    let cache = std::env::temp_dir().join(format!("json-schema-cache-{port}"));
    let _ = std::fs::remove_dir_all(&cache);
    let boolean_error = "Incorrect type. Expected \"boolean\".";
    let string_error = "Incorrect type. Expected \"string\".";
    // The diagnostics of the cached copy can come before those of the changed schema.
    let session = |expected: &str| {
        let mut client = Client::start_with_cache(json!({}), json!({}), cache.to_str().unwrap());
        client.notify(
            "workspace/didChangeConfiguration",
            json!({ "settings": { "json": {
                "validate": { "enable": true },
                "schemas": [{ "fileMatch": ["c.json"], "url": format!("http://127.0.0.1:{port}/ok.json") }]
            } } }),
        );
        client.open("file:///c.json", "json", "{ \"c\": 1 }");
        loop {
            let diagnostics = client.diagnostics("file:///c.json");
            if messages(&diagnostics) == [expected] {
                break;
            }
            assert_eq!(messages(&diagnostics), [boolean_error]);
        }
        assert_eq!(client.exit(), 0);
    };

    session(boolean_error);
    session(boolean_error);
    *schema.lock().unwrap() = ("\"v2\"", string);
    session(string_error);
    session(string_error);
    assert_eq!(
        *requests.lock().unwrap(),
        [
            None,
            Some("\"v1\"".to_owned()),
            Some("\"v1\"".to_owned()),
            Some("\"v2\"".to_owned())
        ]
    );
    std::fs::remove_dir_all(&cache).unwrap();
}

/// `docs/experimental.md`: a configuration equal to the last one changes nothing
/// and sends no refresh.
#[test]
fn equal_configuration() {
    let capabilities = json!({ "textDocument": { "diagnostic": {} } });
    let mut client = Client::start(capabilities, json!({}));
    let settings =
        |enable: bool| json!({ "settings": { "json": { "validate": { "enable": enable } } } });
    client.notify("workspace/didChangeConfiguration", settings(true));
    client.server_request("workspace/diagnostic/refresh");
    client.notify("workspace/didChangeConfiguration", settings(true));
    client.open("file:///a.json", "json", "{");
    let document = json!({ "textDocument": { "uri": "file:///a.json" } });
    let report = client.request("textDocument/diagnostic", document.clone());
    assert_eq!(report["items"].as_array().unwrap().len(), 1);
    client.notify("workspace/didChangeConfiguration", settings(false));
    client.server_request("workspace/diagnostic/refresh");
    let report = client.request("textDocument/diagnostic", document);
    assert_eq!(report["items"], json!([]));
    assert_eq!(client.exit(), 0);
}

/// `docs/experimental.md`: an object deeper than `jsonc::MAX_DEPTH` is empty, with
/// an error. Without the limit, the parse overflows the stack. The client reads
/// at most 128 levels of JSON: nested symbols stop at `MAX_SYMBOL_DEPTH`.
#[test]
fn deep_nesting() {
    let capabilities = json!({ "textDocument": {
        "diagnostic": {},
        "documentSymbol": { "hierarchicalDocumentSymbolSupport": true }
    } });
    let mut client = Client::start(capabilities, json!({}));
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "json": { "validate": { "enable": true } } } }),
    );
    client.server_request("workspace/diagnostic/refresh");
    let depth = 100_000;
    let text = "{\"a\":
"
    .repeat(depth)
        + "1"
        + &"}
"
        .repeat(depth);
    client.open("file:///a.json", "json", &text);
    let document = json!({ "textDocument": { "uri": "file:///a.json" } });
    let report = client.request("textDocument/diagnostic", document.clone());
    let items = report["items"].as_array().unwrap();
    assert_eq!(messages(items), ["Nesting too deep"]);
    assert_eq!(items[0]["range"]["start"]["line"], 1000);
    let symbols = client.request("textDocument/documentSymbol", document.clone());
    assert_eq!(symbol_tree(&symbols), (50, 1000));
    client.request("textDocument/documentColor", document);
    assert_eq!(client.exit(), 0);
}

/// Depth and count of nested symbols.
fn symbol_tree(symbols: &Value) -> (usize, usize) {
    symbols.as_array().map_or((0, 0), |symbols| {
        symbols.iter().fold((0, 0), |(depth, count), s| {
            let (d, c) = symbol_tree(&s["children"]);
            (depth.max(d + 1), count + c + 1)
        })
    })
}

/// `docs/experimental.md`: without client limits, at most 5000 symbols and
/// folding ranges and 500 colors.
#[test]
fn default_limits() {
    let mut client = Client::start(json!({}), json!({}));
    let mut text = String::from("{\n");
    for i in 0..6000 {
        text += &format!("\"k{i}\": {{\n\"c\": \"#ff0000\"\n}},\n");
    }
    text += "\"end\": 0\n}";
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "json": { "schemas": [{
            "fileMatch": ["a.json"],
            "schema": { "additionalProperties": { "properties": { "c": { "format": "color" } } } }
        }] } } }),
    );
    client.open("file:///a.json", "json", &text);
    let document = json!({ "textDocument": { "uri": "file:///a.json" } });
    let count = |client: &mut Client, method: &str| {
        let result = client.request(method, document.clone());
        result.as_array().unwrap().len()
    };
    assert_eq!(count(&mut client, "textDocument/documentSymbol"), 5000);
    assert_eq!(count(&mut client, "textDocument/foldingRange"), 5000);
    assert_eq!(count(&mut client, "textDocument/documentColor"), 500);
    assert_eq!(client.exit(), 0);
}
