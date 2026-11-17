// Writes .scratch/test_files/html-perf/embedded.html, a page with a style
// element, a script element, style and event handler attributes, and a session
// that types in the body, in the script and in the style. Each edit is followed
// by a pull and a completion at the edit, as in Zed. For the time of each step:
// replay with VSCODE_LANGSERVERS_SLOW_MS=0 and read the log lines.
// Usage: node gen-embedded-session.mjs <zed session.jsonl> <out.jsonl> [sections]
import fs from "fs";
import path from "path";
import { pathToFileURL } from "url";

const [base, out, count = "300"] = process.argv.slice(2);
const dir = path.resolve(".scratch/test_files/html-perf");
fs.mkdirSync(dir, { recursive: true });

const section = (i) => `<section id="s${i}" class="card c${i % 7}">
  <h2 style="color: #${(i * 2654435761 % 0xffffff).toString(16).padStart(6, "0")}; margin: ${i % 5}px">Title ${i}</h2>
  <p>Text <a href="page${i}.html">link</a> <b>bold</b></p>
  <button onclick="select(${i}, 'c${i % 7}')">Select ${i}</button>
</section>`;
const style = Array.from(
  { length: 200 },
  (_, i) => `.c${i} {\n  color: #${(i * 40503 % 0xffffff).toString(16).padStart(6, "0")};\n  padding: ${i % 9}px ${i % 4}em;\n}\n.c${i}:hover > a { text-decoration: underline; }`,
).join("\n");
const script = Array.from(
  { length: 200 },
  (_, i) => `function handler${i}(event, items) {
  const result = items.filter((x) => x.id % ${i + 1} === 0).map((x) => ({ ...x, n: x.name.toUpperCase() }));
  if (event.target instanceof HTMLElement) {
    event.target.dataset.count = String(result.length);
  }
  return result;
}`,
).join("\n");
const text = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Embedded</title>
<style>
${style}
</style>
</head>
<body>
${Array.from({ length: Number(count) }, (_, i) => section(i)).join("\n")}
<script>
function select(i, name) {
  document.getElementById("s" + i).classList.toggle(name);
}
${script}
</script>
</body>
</html>
`;
const file = path.join(dir, "embedded.html");
fs.writeFileSync(file, text);

const lines = fs.readFileSync(base, "utf8").split("\n").filter(Boolean).map(JSON.parse);
const init = structuredClone(lines.find((l) => l.dir === "in" && l.msg?.method === "initialize").msg);
const folder = pathToFileURL(dir).href;
init.params.rootPath = dir;
init.params.rootUri = folder;
init.params.workspaceFolders = [{ uri: folder, name: "html-perf" }];
init.params.initializationOptions = { provideFormatter: true };

const msgs = [{ t: 0, dir: "meta", cmd: [] }];
let id = 1000;
let outId = 1;
let t = 0;
const send = (msg) => msgs.push({ t: (t += 10), dir: "in", msg: { jsonrpc: "2.0", ...msg } });
const req = (method, params) => send({ id: id++, method, params });
const answer = (method, params, result) => {
  msgs.push({ t: (t += 1), dir: "out", msg: { jsonrpc: "2.0", id: outId, method, params } });
  msgs.push({ t: (t += 1), dir: "in", msg: { jsonrpc: "2.0", id: outId++, result } });
};
send({ ...init, id: 0 });
send({ method: "initialized", params: {} });
send({ method: "workspace/didChangeConfiguration", params: { settings: {} } });

const positionAt = (text, offset) => {
  const before = text.slice(0, offset);
  const line = before.split("\n").length - 1;
  return { line, character: offset - (before.lastIndexOf("\n") + 1) };
};

const uri = pathToFileURL(file).href;
const textDocument = { uri };
let version = 1;
answer(
  "workspace/configuration",
  { items: ["css", "html", "javascript", "js/ts"].map((section) => ({ scopeUri: uri, section })) },
  [{}, {}, null, null],
);
send({ method: "textDocument/didOpen", params: { textDocument: { uri, languageId: "html", version: version++, text } } });
req("textDocument/diagnostic", { textDocument });
let current = text;
const type = (needle, chars) => {
  let offset = current.indexOf(needle) + needle.length;
  for (const ch of chars) {
    const position = positionAt(current, offset);
    send({
      method: "textDocument/didChange",
      params: { textDocument: { uri, version: version++ }, contentChanges: [{ range: { start: position, end: position }, text: ch }] },
    });
    current = current.slice(0, offset) + ch + current.slice(offset);
    offset += ch.length;
    req("textDocument/diagnostic", { textDocument });
    req("textDocument/completion", { textDocument, position: positionAt(current, offset), context: { triggerKind: 1 } });
  }
};
const middle = `<section id="s${Math.floor(Number(count) / 2)}"`;
type(`${middle} class="card c0">\n  <h2`, [" ", "t", "i", "t", "l", "e"]);
type("Text <a", [" ", "h", "r", "e", "f"]);
type("function handler7(event, items) {\n", ["  ", "c", "o", "n", "s", "o", "l", "e", "."]);
type(".c9 {\n", ["  ", "m", "a", "r", "g", "i", "n", ":"]);
req("textDocument/documentSymbol", { textDocument });
req("textDocument/foldingRange", { textDocument });
req("textDocument/documentColor", { textDocument });
req("textDocument/semanticTokens/full", { textDocument });
send({ method: "textDocument/didClose", params: { textDocument } });
send({ id: id++, method: "shutdown" });
send({ method: "exit" });
fs.writeFileSync(out, msgs.map((l) => JSON.stringify(l)).join("\n") + "\n");
console.log(`${msgs.length} messages; embedded.html ${(text.length / 1e3).toFixed(0)} KB`);
