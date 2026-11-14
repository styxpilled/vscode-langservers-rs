//! Starts a language server and copies its stdio traffic to a JSON Lines file.
//!
//! Usage: `lsp-recorder --out <file> -- <server command> [arguments]`
//!
//! If `<file>` is a folder, each run writes a new `session-<unix ms>.jsonl` in it.
//!
//! Each line of the file is `{"t": <ms>, "dir": "in" | "out", "msg": <message>}`. `in` is
//! client to server. The first line is `{"t": 0, "dir": "meta", "cmd": [...]}`.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use lsp_recorder::read_message;
use serde_json::{Value, json};

struct Log {
    file: Mutex<BufWriter<File>>,
    start: Instant,
}

impl Log {
    fn write(&self, dir: &str, key: &str, value: Value) {
        let t = self.start.elapsed().as_millis() as u64;
        let line = json!({ "t": t, "dir": dir, key: value });
        let mut file = self.file.lock().unwrap();
        // A full disk must not stop the session.
        let _ = writeln!(file, "{line}").and_then(|_| file.flush());
    }
}

/// Copies messages from `reader` to `writer` without change, and logs each one.
fn pump(mut reader: impl BufRead, mut writer: impl Write, log: &Log, dir: &str) -> Result<()> {
    while let Some((header, body)) = read_message(&mut reader)? {
        writer.write_all(&header)?;
        writer.write_all(&body)?;
        writer.flush()?;
        let msg = serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
        log.write(dir, "msg", msg);
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (out, command) = match args.as_slice() {
        [flag, out, sep, command @ ..] if flag == "--out" && sep == "--" && !command.is_empty() => {
            (out, command)
        }
        _ => bail!("usage: lsp-recorder --out <file> -- <server command> [arguments]"),
    };
    let mut out = std::path::PathBuf::from(out);
    if out.is_dir() {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis();
        out.push(format!("session-{millis}.jsonl"));
    }
    let log = Arc::new(Log {
        file: Mutex::new(BufWriter::new(
            File::create(&out).with_context(|| format!("cannot create {}", out.display()))?,
        )),
        start: Instant::now(),
    });
    log.write("meta", "cmd", json!(command));

    let mut child = Command::new(&command[0])
        .args(&command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("cannot start {}", command[0]))?;
    let child_stdin = child.stdin.take().unwrap();
    let child_stdout = child.stdout.take().unwrap();

    let client_log = log.clone();
    // Not joined: it blocks on stdin until the client closes it.
    std::thread::spawn(move || {
        let stdin = BufReader::new(std::io::stdin().lock());
        if let Err(e) = pump(stdin, child_stdin, &client_log, "in") {
            eprintln!("lsp-recorder: client to server: {e}");
        }
    });
    if let Err(e) = pump(
        BufReader::new(child_stdout),
        std::io::stdout().lock(),
        &log,
        "out",
    ) {
        eprintln!("lsp-recorder: server to client: {e}");
    }
    let status = child.wait()?;
    std::process::exit(status.code().unwrap_or(1));
}
