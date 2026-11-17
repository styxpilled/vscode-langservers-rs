//! Stdio transport for the language servers. Same as `lsp_server::Connection::stdio`, but the writer takes framed
//! bytes, so a response result is serialized directly and not through a `serde_json::Value`.
//! A `Value` for a large result (for example 166 MB of document symbols) takes about 8 times
//! the memory of its text.

use std::io::{self, BufRead, Write};
use std::sync::OnceLock;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use lsp_server::{ErrorCode, Message, RequestId, Response};
use lsp_types::DocumentSymbol;
use serde::Serialize;
use serde_json::Value;

pub mod completion;
pub mod server;

#[derive(Clone)]
pub struct Writer(Sender<Vec<u8>>);

impl Writer {
    pub fn message(&self, message: Message) {
        let mut bytes = Vec::new();
        // Writes to a `Vec` do not fail.
        let _ = message.write(&mut bytes);
        self.send(bytes);
    }

    pub fn response(&self, id: RequestId, result: &impl Serialize) {
        #[derive(Serialize)]
        struct Ok<'a, T> {
            jsonrpc: &'static str,
            id: &'a RequestId,
            result: &'a T,
        }
        let ok = Ok {
            jsonrpc: "2.0",
            id: &id,
            result,
        };
        if let Err(e) = self.json(&ok) {
            self.message(
                Response::new_err(id, ErrorCode::InternalError as i32, e.to_string()).into(),
            );
        }
    }

    pub fn notification(&self, method: &str, params: &impl Serialize) {
        #[derive(Serialize)]
        struct Notification<'a, T> {
            jsonrpc: &'static str,
            method: &'a str,
            params: &'a T,
        }
        // Server notification params are `lsp-types` values, which serialize.
        let _ = self.json(&Notification {
            jsonrpc: "2.0",
            method,
            params,
        });
    }

    fn json(&self, body: &impl Serialize) -> serde_json::Result<()> {
        let body = serde_json::to_vec(body)?;
        let mut bytes = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        bytes.extend_from_slice(&body);
        self.send(bytes);
        Ok(())
    }

    fn send(&self, bytes: Vec<u8>) {
        // The receiver only closes when the client is gone.
        let _ = self.0.send(bytes);
    }
}

pub struct IoThreads {
    reader: JoinHandle<io::Result<()>>,
    writer: JoinHandle<io::Result<()>>,
}

impl IoThreads {
    pub fn join(self) -> io::Result<()> {
        self.reader.join().unwrap_or(Ok(()))?;
        self.writer.join().unwrap_or(Ok(()))
    }
}

/// Stack of the server thread. The parsers and tree walks recurse for each
/// nesting level, up to the parser limits. This is address space: the system
/// commits pages only on use.
const SERVER_STACK_SIZE: usize = 256 << 20;

/// Runs `f` on a thread with a large stack. A stack overflow stops the process
/// and `catch_unwind` cannot catch it.
pub fn run_server<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .name("server".into())
        .stack_size(SERVER_STACK_SIZE)
        .spawn(f)
        .expect("server thread")
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e))
}

/// Work that takes longer gets a log message. `VSCODE_LANGSERVERS_SLOW_MS` overrides it.
pub fn slow_limit() -> Duration {
    static LIMIT: OnceLock<Duration> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        let millis = std::env::var("VSCODE_LANGSERVERS_SLOW_MS")
            .ok()
            .and_then(|s| s.parse().ok());
        Duration::from_millis(millis.unwrap_or(50))
    })
}

impl Writer {
    /// Logs `what` if `elapsed` is at or above `slow_limit`.
    pub fn log_slow(&self, what: &str, elapsed: Duration) {
        if elapsed >= slow_limit() {
            self.notification(
                "window/logMessage",
                &serde_json::json!({
                    "type": 4,
                    "message": format!("slow: {what}: {} ms", elapsed.as_millis()),
                }),
            );
        }
    }
}

/// `1.25 MB`, `12 KB`, `300 B`.
pub fn size(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1048576 => format!("{} KB", bytes / 1024),
        _ => format!("{:.2} MB", bytes as f64 / 1048576.0),
    }
}

fn method(message: &Message) -> &str {
    match message {
        Message::Request(r) => &r.method,
        Message::Notification(n) => &n.method,
        Message::Response(_) => "response",
    }
}

fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// `Message::read`, which also gives the body size and the decode time.
fn read(r: &mut impl BufRead) -> io::Result<Option<(Message, usize, Duration)>> {
    let mut size = None;
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let Some(header) = line.strip_suffix("\r\n") else {
            return Err(invalid_data(format!("malformed header: {line:?}")));
        };
        if header.is_empty() {
            break;
        }
        let (name, value) = header
            .split_once(": ")
            .ok_or_else(|| invalid_data(format!("malformed header: {header:?}")))?;
        if name.eq_ignore_ascii_case("Content-Length") {
            size = Some(value.parse::<usize>().map_err(invalid_data)?);
        }
    }
    let size = size.ok_or_else(|| invalid_data("no Content-Length"))?;
    let mut body = vec![0; size];
    r.read_exact(&mut body)?;
    let start = Instant::now();
    let message = serde_json::from_slice(&body)
        .map_err(|e| invalid_data(format!("malformed LSP payload: {e}")))?;
    Ok(Some((message, size, start.elapsed())))
}

pub fn stdio() -> (Receiver<Message>, Writer, IoThreads) {
    let (writer_tx, writer_rx) = unbounded::<Vec<u8>>();
    let log = Writer(writer_tx.clone());
    let (reader_tx, reader_rx) = unbounded();
    let reader = std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        while let Some((message, bytes, decode)) = read(&mut stdin)? {
            log.log_slow(
                &format!("read {} ({})", method(&message), size(bytes)),
                decode,
            );
            let exit = matches!(&message, Message::Notification(n) if n.method == "exit");
            if reader_tx.send(message).is_err() || exit {
                break;
            }
        }
        Ok(())
    });
    let writer = std::thread::spawn(move || {
        let mut stdout = io::stdout().lock();
        for bytes in writer_rx {
            stdout.write_all(&bytes)?;
            stdout.flush()?;
        }
        Ok(())
    });
    (reader_rx, Writer(writer_tx), IoThreads { reader, writer })
}

/// `Connection::initialize_start`: waits for `initialize`. Other requests get
/// `ServerNotInitialized`. Returns `None` on `exit` or when the client is gone.
pub fn initialize_start(
    receiver: &Receiver<Message>,
    writer: &Writer,
) -> Option<(RequestId, Value)> {
    for message in receiver {
        match message {
            Message::Request(r) if r.method == "initialize" => return Some((r.id, r.params)),
            Message::Request(r) => writer.message(
                Response::new_err(
                    r.id,
                    ErrorCode::ServerNotInitialized as i32,
                    format!("expected initialize request, got {}", r.method),
                )
                .into(),
            ),
            Message::Notification(n) if n.method == "exit" => return None,
            _ => {}
        }
    }
    None
}

/// Limit of symbols and folding ranges when the client gives none, as the
/// VS Code defaults (`json.maxItemsComputed`, `editor.foldingMaximumRegions`).
pub const DEFAULT_RESULT_LIMIT: usize = 5000;

/// Limit of colors when the client gives none (`editor.colorDecoratorsLimit`).
pub const DEFAULT_COLOR_LIMIT: usize = 500;

/// Deepest level of a nested symbol. A client with the default `serde_json`
/// recursion limit (128) cannot read a response with symbols 62 levels deep.
pub const MAX_SYMBOL_DEPTH: usize = 50;

/// Moves the symbols below `MAX_SYMBOL_DEPTH` into the children of their
/// ancestor at that depth, in document order.
pub fn cap_symbol_depth(symbols: &mut [DocumentSymbol]) {
    fn cap(symbols: &mut [DocumentSymbol], depth: usize) {
        for symbol in symbols {
            let Some(children) = &mut symbol.children else {
                continue;
            };
            if depth + 1 < MAX_SYMBOL_DEPTH {
                cap(children, depth + 1);
            } else if children.iter().any(|c| c.children.is_some()) {
                let mut flat = Vec::new();
                let mut stack: Vec<_> = std::mem::take(children).into_iter().rev().collect();
                while let Some(mut s) = stack.pop() {
                    stack.extend(s.children.take().into_iter().flatten().rev());
                    flat.push(s);
                }
                *children = flat;
            }
        }
    }
    cap(symbols, 1);
}

#[cfg(test)]
mod tests {
    use lsp_types::{Range, SymbolKind};

    use super::*;

    fn symbol(name: String, children: Option<Vec<DocumentSymbol>>) -> DocumentSymbol {
        #[allow(deprecated)]
        DocumentSymbol {
            name,
            detail: None,
            kind: SymbolKind::FIELD,
            tags: None,
            deprecated: None,
            range: Range::default(),
            selection_range: Range::default(),
            children,
        }
    }

    fn depth(symbols: &[DocumentSymbol]) -> usize {
        symbols
            .iter()
            .map(|s| 1 + s.children.as_deref().map_or(0, depth))
            .max()
            .unwrap_or(0)
    }

    fn names(symbols: &[DocumentSymbol], out: &mut Vec<String>) {
        for s in symbols {
            out.push(s.name.clone());
            names(s.children.as_deref().unwrap_or_default(), out);
        }
    }

    #[test]
    fn read_messages() {
        let body = r#"{"jsonrpc":"2.0","method":"exit"}"#;
        let text = format!(
            "Content-Length: {}\r\nContent-Type: x\r\n\r\n{body}",
            body.len()
        );
        let mut input = text.as_bytes();
        let (message, bytes, _) = read(&mut input).unwrap().unwrap();
        assert_eq!(method(&message), "exit");
        assert_eq!(bytes, body.len());
        assert!(read(&mut input).unwrap().is_none());
        assert!(read(&mut "Content-Type: x\r\n\r\n".as_bytes()).is_err());
        assert!(read(&mut "Content-Length: 2\n\r\n{}".as_bytes()).is_err());
    }

    #[test]
    fn sizes() {
        assert_eq!(size(300), "300 B");
        assert_eq!(size(12 * 1024 + 5), "12 KB");
        assert_eq!(size(1310720), "1.25 MB");
    }

    #[test]
    fn symbol_depth() {
        // A chain of 100 symbols, each with a leaf sibling after its child.
        let mut chain = None;
        for i in (0..100).rev() {
            let leaf = symbol(format!("leaf{i}"), None);
            let children = match chain {
                Some(c) => vec![c, leaf],
                None => vec![leaf],
            };
            chain = Some(symbol(i.to_string(), Some(children)));
        }
        let mut symbols = vec![chain.unwrap()];
        let mut before = Vec::new();
        names(&symbols, &mut before);
        cap_symbol_depth(&mut symbols);
        let mut after = Vec::new();
        names(&symbols, &mut after);
        assert_eq!(depth(&symbols), MAX_SYMBOL_DEPTH);
        assert_eq!(before, after);

        let mut shallow = symbols[0].children.clone().unwrap();
        let copy = shallow.clone();
        cap_symbol_depth(&mut shallow);
        assert_eq!(shallow, copy);
    }
}
