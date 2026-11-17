//! Runs the server binary and talks LSP over stdio.

use std::io::{BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use lsp_server::{Message, Notification, Request, RequestId, Response};
use serde_json::{Value, json};

const URI: &str = "file:///c:/ws/a.html";

struct Client {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Message>,
    next_id: i32,
}

impl Client {
    fn start(capabilities: Value) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_vscode-html-language-server"))
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
        client.request("initialize", json!({ "capabilities": capabilities }));
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

    /// Answers the next `workspace/configuration` request.
    fn configuration(&mut self, result: Result<Value, ()>) {
        let request = loop {
            match self.recv() {
                Message::Request(r) if r.method == "workspace/configuration" => break r,
                Message::Request(r) => panic!("unexpected request {r:?}"),
                _ => {}
            }
        };
        self.send(match result {
            Ok(result) => Response::new_ok(request.id, result),
            Err(()) => Response::new_err(request.id, -32603, "failed".into()),
        });
    }

    fn exit(mut self) -> i32 {
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        self.child.wait().unwrap().code().unwrap()
    }
}

fn formatting(client: &mut Client) -> RequestId {
    client.send_request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": URI },
            "options": { "tabSize": 2, "insertSpaces": true }
        }),
    )
}

/// `docs/experimental.md`: a failed settings request gives the global settings
/// and is not cached; a non-string `html.format.unformatted` is ignored.
#[test]
fn scoped_settings() {
    let mut client = Client::start(json!({ "workspace": { "configuration": true } }));
    client.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": URI, "languageId": "html", "version": 1, "text": "<div><p>a</p></div>" } }),
    );
    let id = formatting(&mut client);
    client.configuration(Err(()));
    assert_eq!(client.response(id).as_array().unwrap().len(), 1);

    let id = formatting(&mut client);
    client.configuration(Ok(Value::Null));
    assert_eq!(client.response(id).as_array().unwrap().len(), 1);

    let id = formatting(&mut client);
    client.configuration(Ok(json!([{}, { "format": { "unformatted": 1 } }, {}, {}])));
    assert_eq!(client.response(id).as_array().unwrap().len(), 1);

    // The settings are cached for the document.
    let id = formatting(&mut client);
    assert_eq!(client.response(id).as_array().unwrap().len(), 1);
    assert_eq!(client.exit(), 0);
}

/// `docs/experimental.md`: `DocumentSymbol` when the client supports it. A style
/// attribute gives no `__` symbol.
#[test]
fn hierarchical_symbols() {
    let capabilities = json!({
        "textDocument": { "documentSymbol": { "hierarchicalDocumentSymbolSupport": true } }
    });
    let mut client = Client::start(capabilities);
    let text = "<div><p style=\"color: red\"></p></div><style>.a { .b {} }</style>";
    client.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": URI, "languageId": "html", "version": 1, "text": text } }),
    );
    let document = json!({ "textDocument": { "uri": URI } });
    let symbols = client.request("textDocument/documentSymbol", document.clone());
    let names = |symbols: &Value| -> Vec<String> {
        symbols
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(names(&symbols), [".a", "div", "style"]);
    assert_eq!(names(&symbols[0]["children"]), [".b"]);
    assert_eq!(names(&symbols[1]["children"]), ["p"]);
    assert!(symbols[0].get("location").is_none());

    // The client reads at most 128 levels of JSON.
    let text = "<div>".repeat(200);
    client.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": URI, "version": 2 }, "contentChanges": [{ "text": text }] }),
    );
    let symbols = client.request("textDocument/documentSymbol", document);
    assert_eq!(symbol_tree(&symbols), (50, 200));
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

/// `docs/experimental.md`: at `MAX_DEPTH`, a start tag closes the deepest open
/// element, so deep nesting does not overflow the stack.
#[test]
fn deep_nesting() {
    let mut client = Client::start(json!({}));
    let depth = 100_000;
    let text = "<div>\n".repeat(depth);
    client.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": URI, "languageId": "html", "version": 1, "text": text } }),
    );
    let document = json!({ "textDocument": { "uri": URI } });
    let symbols = client.request("textDocument/documentSymbol", document);
    assert_eq!(symbols.as_array().unwrap().len(), depth);
    assert_eq!(client.exit(), 0);
}

/// `docs/experimental.md`: without `rangeLimit`, at most 5000 folding ranges.
#[test]
fn default_folding_limit() {
    let mut client = Client::start(json!({}));
    let text = "<div>\n<p></p>\n</div>\n".repeat(6000);
    client.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": URI, "languageId": "html", "version": 1, "text": text } }),
    );
    let ranges = client.request(
        "textDocument/foldingRange",
        json!({ "textDocument": { "uri": URI } }),
    );
    assert_eq!(ranges.as_array().unwrap().len(), 5000);
    assert_eq!(client.exit(), 0);
}

/// `docs/experimental.md`: with `itemDefaults` and resolve, the items have no
/// range or documentation; resolve adds the documentation.
#[test]
fn compact_completion() {
    let capabilities = json!({ "textDocument": { "completion": {
        "completionItem": {
            "snippetSupport": true,
            "documentationFormat": ["markdown", "plaintext"],
            "resolveSupport": { "properties": ["additionalTextEdits", "command", "detail", "documentation"] }
        },
        "completionList": { "itemDefaults": ["commitCharacters", "editRange", "insertTextMode", "insertTextFormat", "data"] }
    } } });
    let mut client = Client::start(capabilities);
    client.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": URI, "languageId": "html", "version": 1, "text": "<di" } }),
    );
    let list = client.request(
        "textDocument/completion",
        json!({ "textDocument": { "uri": URI }, "position": { "line": 0, "character": 3 } }),
    );
    let range =
        json!({ "start": { "line": 0, "character": 1 }, "end": { "line": 0, "character": 3 } });
    assert_eq!(list["itemDefaults"]["editRange"], range);
    let div = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["label"] == "div")
        .unwrap();
    assert!(div.get("textEdit").is_none() && div.get("documentation").is_none());
    let resolved = client.request("completionItem/resolve", div.clone());
    assert!(
        resolved["documentation"]["value"]
            .as_str()
            .unwrap()
            .contains("div")
    );
    assert_eq!(client.exit(), 0);
}
