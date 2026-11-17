//! Replays a session from `lsp-recorder` to a server, or compares two replays.
//!
//! `lsp-replay [--times] <session.jsonl> <result.json> -- <server command> [arguments]`
//! `lsp-replay --diff <a.json> <b.json>`
//!
//! `--times` keeps only the size of each response, for large documents.
//!
//! The replay sends the client messages of the session in order and waits for the response
//! to each request. It answers server requests with the recorded client responses (matched by
//! method and params, in order). `$/cancelRequest` is not sent, because each request is
//! answered before the next message. As Zed does, a `textDocument/diagnostic` request sends
//! the `resultId` of the last report of its document as `previousResultId`.

use std::collections::{HashMap, VecDeque};
use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use lsp_recorder::{read_message, write_message};
use serde_json::{Map, Value, json};

/// The wait for each response. `LSP_REPLAY_TIMEOUT` (seconds) overrides it.
fn timeout() -> Duration {
    let seconds = std::env::var("LSP_REPLAY_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok());
    Duration::from_secs(seconds.unwrap_or(60))
}
/// Time for push diagnostics after the last message.
const SETTLE: Duration = Duration::from_secs(2);

fn read_session(path: &str) -> Result<Vec<Value>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {path}"))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| Ok(serde_json::from_str(l)?))
        .collect()
}

/// Current and peak working set in bytes.
#[cfg(windows)]
fn memory(child: &Child) -> Option<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let size = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // The handle stays valid after the process exits, until `child` is dropped.
    let ok = unsafe { GetProcessMemoryInfo(child.as_raw_handle() as isize, &mut counters, size) };
    (ok != 0).then_some((
        counters.WorkingSetSize as u64,
        counters.PeakWorkingSetSize as u64,
    ))
}

#[cfg(target_os = "linux")]
fn memory(child: &Child) -> Option<(u64, u64)> {
    let status = std::fs::read_to_string(format!("/proc/{}/status", child.id())).ok()?;
    let field = |name: &str| -> Option<u64> {
        let line = status.lines().find(|l| l.starts_with(name))?;
        Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024)
    };
    Some((field("VmRSS:")?, field("VmHWM:")?))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn memory(_: &Child) -> Option<(u64, u64)> {
    None
}

fn megabytes(bytes: u64) -> f64 {
    (bytes as f64 / 1048576.0 * 10.0).round() / 10.0
}

fn key(method: &str, params: &Value) -> String {
    format!("{method} {params}")
}

/// The client responses to server requests, by method and params.
fn client_answers(session: &[Value]) -> HashMap<String, VecDeque<Value>> {
    let mut requests = HashMap::new();
    let mut answers: HashMap<String, VecDeque<Value>> = HashMap::new();
    for line in session {
        let msg = &line["msg"];
        let Some(id) = msg.get("id") else { continue };
        match (
            line["dir"].as_str(),
            msg.get("method").and_then(Value::as_str),
        ) {
            (Some("out"), Some(method)) => {
                requests.insert(id.to_string(), key(method, &msg["params"]));
            }
            (Some("in"), None) => {
                if let Some(k) = requests.remove(&id.to_string()) {
                    let mut answer = msg.clone();
                    answer.as_object_mut().unwrap().remove("id");
                    answers.entry(k).or_default().push_back(answer);
                }
            }
            _ => {}
        }
    }
    answers
}

struct Server {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    messages: Receiver<Value>,
    answers: HashMap<String, VecDeque<Value>>,
    server_requests: Vec<Value>,
    diagnostics: Map<String, Value>,
    logs: Vec<Value>,
}

impl Server {
    fn start(command: &[String], answers: HashMap<String, VecDeque<Value>>) -> Result<Server> {
        let mut child = Command::new(&command[0])
            .args(&command[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("cannot start {}", command[0]))?;
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, messages) = channel();
        std::thread::spawn(move || {
            while let Ok(Some((_, body))) = read_message(&mut stdout) {
                let Ok(msg) = serde_json::from_slice(&body) else {
                    continue;
                };
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        Ok(Server {
            child,
            stdin,
            messages,
            answers,
            server_requests: Vec::new(),
            diagnostics: Map::new(),
            logs: Vec::new(),
        })
    }

    fn send(&mut self, msg: &Value) -> Result<()> {
        write_message(&mut self.stdin, msg.to_string().as_bytes())
    }

    /// Handles one server message. Returns it if it is a response.
    fn handle(&mut self, msg: Value) -> Result<Option<Value>> {
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            return Ok(Some(msg));
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match msg.get("id") {
            Some(id) => {
                let k = key(method, &params);
                let mut answer = match self.answers.get_mut(&k) {
                    Some(queue) if queue.len() > 1 => queue.pop_front().unwrap(),
                    Some(queue) => queue[0].clone(),
                    None => json!({ "jsonrpc": "2.0", "result": null }),
                };
                answer["id"] = id.clone();
                self.server_requests
                    .push(json!({ "method": method, "params": params }));
                self.send(&answer)?;
            }
            None if method == "textDocument/publishDiagnostics" => {
                let uri = params["uri"].as_str().unwrap_or_default().to_owned();
                self.diagnostics.insert(uri, params["diagnostics"].clone());
            }
            None if method == "window/logMessage" => self.logs.push(params),
            None => {}
        }
        Ok(None)
    }

    fn response(&mut self, id: &Value) -> Result<Value> {
        let deadline = Instant::now() + timeout();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let msg = self
                .messages
                .recv_timeout(remaining)
                .with_context(|| format!("no response to request {id}"))?;
            if let Some(response) = self.handle(msg)?
                && response.get("id") == Some(id)
            {
                return Ok(response);
            }
        }
    }

    fn settle(&mut self) -> Result<()> {
        let deadline = Instant::now() + SETTLE;
        while let Ok(msg) = self
            .messages
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            self.handle(msg)?;
        }
        Ok(())
    }
}

fn replay(session_path: &str, out: &str, command: &[String], times_only: bool) -> Result<()> {
    let session = read_session(session_path)?;
    let mut server = Server::start(command, client_answers(&session))?;
    let mut requests = Vec::new();
    let start = Instant::now();
    let mut startup_millis = None;
    let mut end_memory = None;
    let mut result_ids: HashMap<String, Value> = HashMap::new();
    for (index, line) in session.iter().enumerate() {
        let msg = &line["msg"];
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            continue;
        };
        if line["dir"] != "in" || method == "$/cancelRequest" {
            continue;
        }
        if method == "shutdown" || method == "exit" {
            server.settle()?;
            end_memory = end_memory.or_else(|| memory(&server.child));
        }
        let uri = msg
            .pointer("/params/textDocument/uri")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut msg = msg.clone();
        match method {
            // `vscode-languageserver` exits when the client process is gone.
            "initialize" => msg["params"]["processId"] = json!(std::process::id()),
            "textDocument/diagnostic" => {
                if let Some(result_id) = result_ids.get(&uri) {
                    msg["params"]["previousResultId"] = result_id.clone();
                }
            }
            "textDocument/didClose" => {
                result_ids.remove(&uri);
            }
            _ => {}
        }
        server.send(&msg)?;
        let Some(id) = msg.get("id") else { continue };
        let sent = Instant::now();
        let response = server.response(id)?;
        if method == "textDocument/diagnostic" {
            match response.pointer("/result/resultId") {
                Some(result_id) => result_ids.insert(uri, result_id.clone()),
                None => result_ids.remove(&uri),
            };
        }
        let millis = sent.elapsed().as_millis() as u64;
        if method == "initialize" {
            startup_millis = Some(start.elapsed().as_millis() as u64);
        }
        let mut entry = json!({
            "line": index + 1,
            "method": method,
            "millis": millis,
            "params": msg.get("params"),
        });
        if times_only {
            entry["response_bytes"] = json!(response.to_string().len());
        } else {
            let mut response = response.as_object().cloned().unwrap_or_default();
            response.remove("jsonrpc");
            response.remove("id");
            entry["response"] = Value::Object(response);
        }
        requests.push(entry);
    }
    if end_memory.is_none() {
        server.settle()?;
        end_memory = memory(&server.child);
    }
    let exit = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = server.child.try_wait()? {
            break status.code();
        }
        if Instant::now() > exit {
            server.child.kill()?;
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let result = json!({
        "session": session_path,
        "command": command,
        "millis": start.elapsed().as_millis() as u64,
        "exit_code": status,
        "startup_millis": startup_millis,
        "end_memory_mb": end_memory.map(|(current, _)| megabytes(current)),
        "peak_memory_mb": memory(&server.child).map(|(_, peak)| megabytes(peak)),
        "requests": requests,
        "server_requests": server.server_requests,
        "diagnostics": server.diagnostics,
        "logs": server.logs,
    });
    std::fs::write(out, serde_json::to_string_pretty(&result)?)?;
    eprintln!(
        "{} requests in {} ms, exit code {status:?}, startup {} ms, memory at end {} MB, peak {} MB",
        result["requests"].as_array().unwrap().len(),
        result["millis"],
        result["startup_millis"],
        result["end_memory_mb"],
        result["peak_memory_mb"],
    );
    Ok(())
}

/// The path of the first difference, or `None` if the values are equal. Object key order
/// does not matter.
fn first_difference(a: &Value, b: &Value, path: String) -> Option<(String, String, String)> {
    let show = |v: Option<&Value>| match v {
        Some(v) => {
            let s = v.to_string();
            if s.len() > 300 {
                format!("{}...", &s[..s.floor_char_boundary(300)])
            } else {
                s
            }
        }
        None => "(missing)".to_owned(),
    };
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                let p = format!("{path}.{k}");
                match (x.get(k), y.get(k)) {
                    (Some(u), Some(v)) => {
                        if let Some(d) = first_difference(u, v, p) {
                            return Some(d);
                        }
                    }
                    (u, v) => return Some((p, show(u), show(v))),
                }
            }
            None
        }
        (Value::Array(x), Value::Array(y)) => {
            for (i, (u, v)) in x.iter().zip(y).enumerate() {
                if let Some(d) = first_difference(u, v, format!("{path}[{i}]")) {
                    return Some(d);
                }
            }
            (x.len() != y.len()).then(|| {
                (
                    format!("{path}.length"),
                    x.len().to_string(),
                    y.len().to_string(),
                )
            })
        }
        _ => (a != b).then(|| (path, show(Some(a)), show(Some(b)))),
    }
}

/// Replaces each `unchanged` diagnostic report with the last items of its document, and
/// removes the `resultId`. Returns the number of `unchanged` reports.
fn resolve_unchanged(requests: &mut [Value]) -> usize {
    let mut items: HashMap<String, Value> = HashMap::new();
    let mut unchanged = 0;
    for request in requests {
        if request["method"] != "textDocument/diagnostic" {
            continue;
        }
        let uri = request["params"]
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(Value::Object(result)) = request.pointer_mut("/response/result") else {
            continue;
        };
        result.remove("resultId");
        if result.get("kind").and_then(Value::as_str) == Some("unchanged") {
            unchanged += 1;
            result.insert("kind".into(), json!("full"));
            let last = items.get(&uri).cloned().unwrap_or(Value::Null);
            result.insert("items".into(), last);
        } else if let Some(i) = result.get("items") {
            items.insert(uri, i.clone());
        }
    }
    unchanged
}

fn diff(a_path: &str, b_path: &str) -> Result<()> {
    let read =
        |p: &str| -> Result<Value> { Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?) };
    let (mut a, mut b) = (read(a_path)?, read(b_path)?);
    let mut unchanged = [0, 0];
    for (side, count) in [&mut a, &mut b].into_iter().zip(&mut unchanged) {
        if let Some(requests) = side["requests"].as_array_mut() {
            *count = resolve_unchanged(requests);
        }
    }
    let mut differences = 0;
    let report = |what: String, d: Option<(String, String, String)>| {
        let Some((path, x, y)) = d else { return 0 };
        println!("{what}\n  at {path}\n  a: {x}\n  b: {y}");
        1
    };
    let empty = Vec::new();
    let ra = a["requests"].as_array().unwrap_or(&empty);
    let rb = b["requests"].as_array().unwrap_or(&empty);
    for (x, y) in ra.iter().zip(rb) {
        let uri = x["params"]
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let position = x["params"]
            .get("position")
            .map(|p| format!(" {}:{}", p["line"], p["character"]))
            .unwrap_or_default();
        differences += report(
            format!("line {} {} {uri}{position}", x["line"], x["method"]),
            first_difference(&x["response"], &y["response"], String::new()),
        );
    }
    if ra.len() != rb.len() {
        println!("request count: a {} b {}", ra.len(), rb.len());
        differences += 1;
    }
    // The order of server requests depends on thread timing.
    let sorted = |v: &Value| {
        let mut list = v.as_array().cloned().unwrap_or_default();
        list.sort_by_cached_key(Value::to_string);
        list
    };
    let (sa, sb) = (sorted(&a["server_requests"]), sorted(&b["server_requests"]));
    let only_in = |x: &[Value], y: &[Value]| {
        let mut rest: Vec<String> = y.iter().map(Value::to_string).collect();
        x.iter()
            .filter(|v| {
                let s = v.to_string();
                match rest.iter().position(|r| *r == s) {
                    Some(i) => {
                        rest.swap_remove(i);
                        false
                    }
                    None => true,
                }
            })
            .count()
    };
    let (only_a, only_b) = (only_in(&sa, &sb), only_in(&sb, &sa));
    differences += report(
        format!("server_requests (only in a: {only_a}, only in b: {only_b})"),
        first_difference(&Value::Array(sa), &Value::Array(sb), String::new()),
    );
    for key in ["diagnostics", "exit_code"] {
        differences += report(
            key.to_owned(),
            first_difference(&a[key], &b[key], String::new()),
        );
    }
    println!(
        "{differences} differences in {} requests. Time: a {} ms, b {} ms",
        ra.len(),
        a["millis"],
        b["millis"]
    );
    for key in ["startup_millis", "end_memory_mb", "peak_memory_mb"] {
        println!("{key}: a {}, b {}", a[key], b[key]);
    }
    println!(
        "unchanged diagnostic reports: a {}, b {}",
        unchanged[0], unchanged[1]
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag, a, b] if flag == "--diff" => diff(a, b),
        [flag, session, out, sep, command @ ..]
            if flag == "--times" && sep == "--" && !command.is_empty() =>
        {
            replay(session, out, command, true)
        }
        [session, out, sep, command @ ..] if sep == "--" && !command.is_empty() => {
            replay(session, out, command, false)
        }
        _ => bail!(
            "usage: lsp-replay [--times] <session.jsonl> <result.json> -- <server command> [arguments]\n       lsp-replay --diff <a.json> <b.json>"
        ),
    }
}
