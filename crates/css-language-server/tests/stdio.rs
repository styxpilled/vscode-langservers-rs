//! Runs the server binary and talks LSP over stdio.

use std::io::{BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use lsp_server::{Message, Notification, Request, RequestId, Response};
use serde_json::{Value, json};

const URI: &str = "file:///c:/ws/a.css";

struct Client {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Message>,
    next_id: i32,
}

impl Client {
    fn start(capabilities: Value, options: Value) -> (Client, Value) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_vscode-css-language-server"))
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
            json!({
                "capabilities": capabilities,
                "initializationOptions": options,
                "workspaceFolders": [{ "name": "ws", "uri": "file:///c:/ws" }]
            }),
        );
        client.notify("initialized", json!({}));
        (client, result)
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
        self.response(id).unwrap()
    }

    fn response(&mut self, id: RequestId) -> Result<Value, i32> {
        loop {
            match self.recv() {
                Message::Response(r) if r.id == id => {
                    return r.response_result.map_err(|e| e.code);
                }
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
            match self.recv() {
                Message::Request(r) if r.method == method => return r,
                Message::Request(r) => panic!("unexpected request {r:?}"),
                _ => {}
            }
        }
    }

    /// Answers the next `workspace/configuration` request.
    fn configuration(&mut self, result: Value) -> Value {
        let request = self.server_request("workspace/configuration");
        self.send(Response::new_ok(request.id, result));
        request.params
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

fn text_document(uri: &str) -> Value {
    json!({ "textDocument": { "uri": uri } })
}

fn at(uri: &str, line: u32, character: u32) -> Value {
    json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } })
}

fn severities(diagnostics: &[Value]) -> Vec<(String, i64)> {
    diagnostics
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap().to_owned(),
                d["severity"].as_i64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn capabilities() {
    let caps = json!({ "textDocument": { "completion": { "completionItem": { "snippetSupport": true } } } });
    let (client, result) = Client::start(caps, json!({ "provideFormatter": true }));
    let caps = &result["capabilities"];
    assert_eq!(
        caps["completionProvider"]["triggerCharacters"],
        json!(["/", "-", ":"])
    );
    assert_eq!(
        caps["codeActionProvider"],
        json!({ "codeActionKinds": ["quickfix"] })
    );
    assert_eq!(caps["documentFormattingProvider"], true);
    assert_eq!(client.exit(), 0);

    let (client, result) = Client::start(json!({}), json!({}));
    let caps = &result["capabilities"];
    assert!(caps.get("completionProvider").is_none());
    assert_eq!(caps["documentFormattingProvider"], false);
    assert_eq!(client.exit(), 0);
}

#[test]
fn push_diagnostics() {
    let (mut client, _) = Client::start(json!({}), json!({}));
    client.open(URI, "css", "a { colr: red }");
    let diagnostics = client.diagnostics(URI);
    assert_eq!(severities(&diagnostics), [("unknownProperties".into(), 2)]);

    client.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": URI, "version": 2 },
            "contentChanges": [{ "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 8 } }, "text": "color" }]
        }),
    );
    assert_eq!(client.diagnostics(URI), Vec::<Value>::new());

    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "css": { "lint": { "zeroUnits": "error" } } } }),
    );
    client.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": URI, "version": 3 }, "contentChanges": [{ "text": "a { width: 0px }" }] }),
    );
    let diagnostics = client.diagnostics(URI);
    assert_eq!(severities(&diagnostics), [("zeroUnits".into(), 1)]);

    // `docs/experimental.md`: `null` settings are no settings.
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": null }),
    );
    assert_eq!(client.diagnostics(URI), Vec::<Value>::new());

    client.notify("textDocument/didClose", text_document(URI));
    assert_eq!(client.diagnostics(URI), Vec::<Value>::new());
    assert_eq!(client.exit(), 0);
}

/// A change while a validation waits for the settings: only the new text is published.
#[test]
fn stale_validation() {
    let (mut client, _) =
        Client::start(json!({ "workspace": { "configuration": true } }), json!({}));
    client.open(URI, "css", "a { colr: red }");
    let request = client.server_request("workspace/configuration");
    client.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": URI, "version": 2 }, "contentChanges": [{ "text": "a { color: red }" }] }),
    );
    client.send(Response::new_ok(request.id, json!([{}])));
    assert_eq!(client.diagnostics(URI), Vec::<Value>::new());
    assert_eq!(client.exit(), 0);
}

#[test]
fn scoped_settings() {
    let caps =
        json!({ "workspace": { "configuration": true }, "textDocument": { "diagnostic": {} } });
    let (mut client, _) = Client::start(caps, json!({}));
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "css": { "lint": { "unknownProperties": "error" } } } }),
    );
    client.server_request("workspace/diagnostic/refresh");
    client.open(URI, "css", "a { colr: 0px }");

    let id = client.send_request("textDocument/diagnostic", text_document(URI));
    let params = client.configuration(json!([{ "lint": { "unknownProperties": "ignore" } }]));
    assert_eq!(
        params,
        json!({ "items": [{ "scopeUri": URI, "section": "css" }] })
    );
    let report = client.response(id).unwrap();
    assert_eq!(report["kind"], "full");
    assert_eq!(severities(report["items"].as_array().unwrap()), []);

    // The settings are cached for the document.
    let hover = client.request("textDocument/hover", at(URI, 0, 5));
    assert!(hover.is_null());

    // `undefined` settings: the `configure` settings apply.
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "css": { "lint": { "unknownProperties": "error" } } } }),
    );
    client.server_request("workspace/diagnostic/refresh");
    let id = client.send_request("textDocument/diagnostic", text_document(URI));
    client.configuration(json!([]));
    let report = client.response(id).unwrap();
    assert_eq!(
        severities(report["items"].as_array().unwrap()),
        [("unknownProperties".into(), 1)]
    );

    // `null` settings: the defaults apply.
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": {} }),
    );
    client.server_request("workspace/diagnostic/refresh");
    let id = client.send_request("textDocument/diagnostic", text_document(URI));
    client.configuration(json!([null]));
    let report = client.response(id).unwrap();
    assert_eq!(
        severities(report["items"].as_array().unwrap()),
        [("unknownProperties".into(), 2)]
    );

    // `docs/experimental.md`: an unchanged report needs no settings.
    let id = report["resultId"].clone();
    let report = client.request(
        "textDocument/diagnostic",
        json!({ "textDocument": { "uri": URI }, "previousResultId": id }),
    );
    assert_eq!(report, json!({ "kind": "unchanged", "resultId": id }));

    // Settings that were requested before a configuration change are older: the
    // next pull validates again.
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": {} }),
    );
    client.server_request("workspace/diagnostic/refresh");
    let pull = client.send_request("textDocument/diagnostic", text_document(URI));
    let request = client.server_request("workspace/configuration");
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "css": { "lint": { "unknownProperties": "error" } } } }),
    );
    client.server_request("workspace/diagnostic/refresh");
    client.send(Response::new_ok(request.id, json!([null])));
    let report = client.response(pull).unwrap();
    assert_eq!(
        severities(report["items"].as_array().unwrap()),
        [("unknownProperties".into(), 2)]
    );
    let id = client.send_request(
        "textDocument/diagnostic",
        json!({ "textDocument": { "uri": URI }, "previousResultId": report["resultId"] }),
    );
    client.configuration(json!([]));
    assert_eq!(
        severities(client.response(id).unwrap()["items"].as_array().unwrap()),
        [("unknownProperties".into(), 1)]
    );

    // A request that waits for the settings can be cancelled.
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": {} }),
    );
    client.server_request("workspace/diagnostic/refresh");
    let id = client.send_request("textDocument/hover", at(URI, 0, 5));
    let request = client.server_request("workspace/configuration");
    client.notify("$/cancelRequest", json!({ "id": id }));
    assert_eq!(client.response(id), Err(-32800));
    client.send(Response::new_ok(request.id, json!([{}])));

    // `docs/experimental.md`: a failed settings request gives the `configure`
    // settings and is not cached.
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": {} }),
    );
    client.server_request("workspace/diagnostic/refresh");
    let id = client.send_request("textDocument/completion", at(URI, 0, 5));
    let request = client.server_request("workspace/configuration");
    client.send(Response::new_err(request.id, -32603, "failed".into()));
    assert!(client.response(id).unwrap()["items"].is_array());
    let id = client.send_request("textDocument/completion", at(URI, 0, 5));
    let request = client.server_request("workspace/configuration");
    client.send(Response::new_ok(request.id, Value::Null));
    assert!(client.response(id).unwrap()["items"].is_array());
    assert_eq!(client.exit(), 0);
}

#[test]
fn hover_completion_and_navigation() {
    let caps = json!({
        "textDocument": {
            "hover": { "contentFormat": ["markdown"] },
            "completion": { "completionItem": { "documentationFormat": ["plaintext"] } }
        }
    });
    let (mut client, _) = Client::start(caps, json!({}));
    let text = "@import 'b.css';\n.a { color: #ff0000; }\n.a:hover {}";
    client.open(URI, "css", text);

    let hover = client.request("textDocument/hover", at(URI, 1, 7));
    assert_eq!(hover["contents"]["kind"], "markdown");
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("color")
    );

    let completion = client.request("textDocument/completion", at(URI, 1, 8));
    let items = completion["items"].as_array().unwrap();
    let color = items.iter().find(|i| i["label"] == "color").unwrap();
    assert_eq!(color["documentation"]["kind"], "plaintext");

    let links = client.request("textDocument/documentLink", text_document(URI));
    assert_eq!(links[0]["target"], "file:///c:/ws/b.css");

    let colors = client.request("textDocument/documentColor", text_document(URI));
    assert_eq!(
        colors[0]["color"],
        json!({ "red": 1, "green": 0, "blue": 0, "alpha": 1 })
    );

    let highlights = client.request("textDocument/documentHighlight", at(URI, 1, 1));
    assert_eq!(highlights.as_array().unwrap().len(), 2);

    let symbols = client.request("textDocument/documentSymbol", text_document(URI));
    assert_eq!(symbols[0]["name"], ".a");

    let folding = client.request("textDocument/foldingRange", text_document(URI));
    assert!(folding.is_array());

    let missing = "file:///c:/ws/missing.css";
    assert!(
        client
            .request("textDocument/hover", at(missing, 0, 0))
            .is_null()
    );
    assert_eq!(
        client.request("textDocument/documentSymbol", text_document(missing)),
        json!([])
    );
    assert_eq!(client.exit(), 0);
}

#[test]
fn formatting() {
    let (mut client, _) = Client::start(json!({}), json!({ "provideFormatter": true }));
    client.open(URI, "css", "a{color:red}");
    let format = |client: &mut Client, options: Value| {
        let params = json!({ "textDocument": { "uri": URI }, "options": options });
        client.request("textDocument/formatting", params)
    };
    let edits = format(&mut client, json!({ "tabSize": 2, "insertSpaces": true }));
    assert_eq!(edits[0]["newText"], "a {\n  color: red;\n}");
    let options = json!({ "tabSize": 2.7, "insertSpaces": 1, "braceStyle": "collapse,expand" });
    let edits = format(&mut client, options);
    assert_eq!(edits[0]["newText"], "a\n{\n  color: red;\n}");
    // `docs/experimental.md`: an unknown brace style is ignored.
    let edits = format(&mut client, json!({ "braceStyle": "sideways" }));
    assert_eq!(
        edits[0]["newText"],
        "a {
	color: red;
}"
    );
    assert_eq!(client.exit(), 0);
}

#[test]
fn custom_data() {
    let dir = std::env::temp_dir().join(format!("css-custom-data-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("data.json");
    let data =
        json!({ "version": 1.1, "properties": [{ "name": "foo-bar", "description": "Foo bar." }] });
    std::fs::write(&file, data.to_string()).unwrap();
    let path = file.to_str().unwrap().replace('\\', "/");
    let data_uri = format!("file:///{}", path.trim_start_matches('/'));

    let (mut client, _) = Client::start(json!({}), json!({}));
    client.open(URI, "css", "a { foo-bar: 1 }");
    assert_eq!(client.diagnostics(URI).len(), 1);
    client.notify(
        "css/customDataChanged",
        json!([[data_uri, "file:///c:/missing.json"]]),
    );
    let hover = client.request("textDocument/hover", at(URI, 0, 5));
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Foo bar.")
    );
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(client.exit(), 0);
}

/// `docs/experimental.md`: nesting deeper than `MAX_DEPTH` gives an error, not a
/// stack overflow, and nested `:not(` and `#{` take polynomial time.
#[test]
fn deep_nesting() {
    let caps = json!({ "textDocument": { "diagnostic": {} } });
    let (mut client, _) = Client::start(caps, json!({}));
    let diagnostics = |client: &mut Client, uri: &str, language_id: &str, text: &str| {
        client.open(uri, language_id, text);
        let report = client.request("textDocument/diagnostic", text_document(uri));
        severities(report["items"].as_array().unwrap())
    };

    // `a{` is the same in release, but slow in debug (`debug_assert!` in `discard_from`).
    let media = "@media ".to_owned() + &"(".repeat(1200);
    let codes = diagnostics(&mut client, "file:///media.css", "css", &media);
    assert!(
        codes.contains(&("css-nestingtoodeep".to_owned(), 1)),
        "{codes:?}"
    );
    let chain = format!("a{{b:{}1}}", "1+".repeat(1200));
    diagnostics(&mut client, "file:///chain.css", "css", &chain);
    let not = ":not(".repeat(40);
    diagnostics(&mut client, "file:///not.css", "css", &not);
    let interpolation = format!("a{{b:{}", "#{".repeat(40));
    diagnostics(&mut client, "file:///i.scss", "scss", &interpolation);

    let rules = "a{".repeat(200);
    client.open("file:///rules.css", "css", &rules);
    let symbols = client.request(
        "textDocument/documentSymbol",
        text_document("file:///rules.css"),
    );
    assert_eq!(symbol_tree(&symbols), (50, 200));

    let selector = "a ".repeat(3000) + "{}";
    client.open("file:///selector.css", "css", &selector);
    let hover = client.request("textDocument/hover", at("file:///selector.css", 0, 1));
    let html = hover["contents"][0].as_str().unwrap();
    assert_eq!(html.lines().count(), 1001);
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

/// `docs/experimental.md`: without `rangeLimit`, at most 5000 folding ranges.
#[test]
fn default_folding_limit() {
    let (mut client, _) = Client::start(json!({}), json!({}));
    client.open("file:///a.css", "css", &"a {\nb: c;\n}\n".repeat(6000));
    let ranges = client.request("textDocument/foldingRange", text_document("file:///a.css"));
    assert_eq!(ranges.as_array().unwrap().len(), 5000);
    assert_eq!(client.exit(), 0);
}

/// Completion capabilities of Zed.
fn zed_completion() -> Value {
    json!({ "textDocument": { "completion": {
        "completionItem": {
            "snippetSupport": true,
            "documentationFormat": ["markdown", "plaintext"],
            "resolveSupport": { "properties": ["additionalTextEdits", "command", "detail", "documentation"] }
        },
        "completionList": { "itemDefaults": ["commitCharacters", "editRange", "insertTextMode", "insertTextFormat", "data"] }
    } } })
}

/// `docs/experimental.md`: with `itemDefaults` and resolve, the items have no
/// range, documentation or command; resolve adds them.
#[test]
fn compact_completion() {
    let (mut client, init) = Client::start(zed_completion(), json!({}));
    assert_eq!(
        init["capabilities"]["completionProvider"]["resolveProvider"],
        true
    );
    client.open("file:///a.css", "css", "a { colo }");
    let list = client.request("textDocument/completion", at("file:///a.css", 0, 8));
    let range =
        json!({ "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 8 } });
    assert_eq!(
        list["itemDefaults"],
        json!({ "editRange": range, "insertTextFormat": 2 })
    );
    let items = list["items"].as_array().unwrap();
    let color = items.iter().find(|i| i["label"] == "color").unwrap();
    assert_eq!(color["textEditText"], "color: $0;");
    for key in [
        "textEdit",
        "documentation",
        "command",
        "insertTextFormat",
        "tags",
    ] {
        assert!(color.get(key).is_none(), "{key}");
    }
    let resolved = client.request("completionItem/resolve", color.clone());
    assert!(
        resolved["documentation"]["value"]
            .as_str()
            .unwrap()
            .contains("color")
    );
    assert_eq!(
        resolved["command"]["command"],
        "editor.action.triggerSuggest"
    );
    assert_eq!(client.exit(), 0);
}
