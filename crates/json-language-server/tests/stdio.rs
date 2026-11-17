//! Runs the server binary and talks LSP over stdio.

use std::io::{BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
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
        let mut child = Command::new(env!("CARGO_BIN_EXE_vscode-json-language-server"))
            .arg("--stdio")
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

#[test]
fn exit_without_shutdown() {
    let mut client = Client::start(json!({}), json!({}));
    client.notify("exit", Value::Null);
    assert_eq!(client.child.wait().unwrap().code(), Some(1));
}

/// Serves `/ok.json` and gives 404 with an empty body for other paths.
fn http_server() -> u16 {
    use std::io::{BufRead, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
            }
            let body = r#"{ "properties": { "c": { "type": "boolean" } } }"#;
            let response = if request_line.contains("/ok.json") {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned()
            };
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    port
}

#[test]
fn http_schemas() {
    let port = http_server();
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
