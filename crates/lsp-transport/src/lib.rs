//! Stdio transport for the language servers. Same as `lsp_server::Connection::stdio`, but the writer takes framed
//! bytes, so a response result is serialized directly and not through a `serde_json::Value`.
//! A `Value` for a large result (for example 166 MB of document symbols) takes about 8 times
//! the memory of its text.

use std::io::{self, Write};
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, unbounded};
use lsp_server::{ErrorCode, Message, RequestId, Response};
use serde::Serialize;
use serde_json::Value;

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

pub fn stdio() -> (Receiver<Message>, Writer, IoThreads) {
    let (reader_tx, reader_rx) = unbounded();
    let reader = std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        while let Some(message) = Message::read(&mut stdin)? {
            let exit = matches!(&message, Message::Notification(n) if n.method == "exit");
            if reader_tx.send(message).is_err() || exit {
                break;
            }
        }
        Ok(())
    });
    let (writer_tx, writer_rx) = unbounded::<Vec<u8>>();
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
