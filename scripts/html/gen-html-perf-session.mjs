// Writes large HTML documents to .scratch/test_files/html-perf/ and a session
// that runs each feature on them, with the Zed `initialize` of a recorded
// session. Each document also gets typing edits, each followed by completion
// (a parse for each change).
// Usage: node gen-html-perf-session.mjs <zed session.jsonl> <out.jsonl>
import fs from "fs";
import path from "path";
import { pathToFileURL } from "url";

const [base, out] = process.argv.slice(2);
const dir = path.resolve(".scratch/test_files/html-perf");
fs.mkdirSync(dir, { recursive: true });

const section = (i) => `<section id="s${i}" class="card c${i % 7}">
  <!-- section ${i} -->
  <h2 style="color: #${(i * 2654435761 % 0xffffff).toString(16).padStart(6, "0")}; margin: ${i % 5}px">Title ${i} &amp; more</h2>
  <p>Text <a href="page${i}.html#x">link</a> <b>bold <i>it</i></b> &copy; <img src="img/${i}.png" alt="a${i}"><br>
    second line with <span class="x">span</span> and <code>code()</code>.</p>
  <ul>
${[0, 1, 2, 3, 4].map((j) => `    <li data-k="${j}"><a href="#s${i}-${j}">item ${j}</a></li>`).join("\n")}
  </ul>
  <table>
    <tr><th>a</th><th style="width: 10%">b</th></tr>
    <tr><td>${i}</td><td>${i * 3}</td></tr>
  </table>
  <form action="/f${i}"><input type="text" name="n${i}" value="v"><select><option>1</option><option selected>2</option></select>
    <textarea rows="2">x</textarea></form>
</section>`;
const page = (body, head = "") =>
  `<!DOCTYPE html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<title>Perf</title>\n${head}</head>\n<body>\n${body}\n</body>\n</html>\n`;
const sections = (n) => Array.from({ length: n }, (_, i) => section(i)).join("\n");
const bootstrap = fs.readFileSync(path.resolve(".scratch/test_files/css/bootstrap.min.css"), "utf8");

const files = {
  "large.html": page(sections(2000)),
  "minified.html": page(sections(1000)).replace(/\n\s*/g, ""),
  "styles.html": page(sections(50), `<style>\n${bootstrap.replace(/}/g, "}\n").repeat(4)}</style>\n`),
};
for (const [name, text] of Object.entries(files)) fs.writeFileSync(path.join(dir, name), text);

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

// The small document first: both servers load their code before the timed documents.
const order = [path.resolve(".scratch/test_files/html/page.html"), ...Object.keys(files).map((f) => path.join(dir, f))];
let version = 1;
for (const p of order) {
  const uri = pathToFileURL(p).href;
  const text = fs.readFileSync(p, "utf8");
  const textDocument = { uri };
  answer("workspace/configuration", { items: ["css", "html", "javascript", "js/ts"].map((section) => ({ scopeUri: uri, section })) }, [{}, {}, null, null]);
  send({ method: "textDocument/didOpen", params: { textDocument: { uri, languageId: "html", version: version++, text } } });
  req("textDocument/diagnostic", { textDocument });
  req("textDocument/documentSymbol", { textDocument });
  req("textDocument/foldingRange", { textDocument });
  req("textDocument/documentLink", { textDocument });
  req("textDocument/documentColor", { textDocument });
  req("textDocument/formatting", { textDocument, options: { tabSize: 2, insertSpaces: true } });
  const at = (needle, from, delta = 1) => positionAt(text, text.indexOf(needle, from) + delta);
  const middle = Math.floor(text.length / 2);
  const tenth = Math.floor(text.length / 10);
  req("textDocument/rangeFormatting", {
    textDocument,
    range: { start: positionAt(text, middle), end: positionAt(text, middle + tenth) },
    options: { tabSize: 2, insertSpaces: true },
  });
  const points = [
    at("<p", middle, 2), // tag name
    at("href=\"", middle, 7), // attribute value
    at("class=", middle, 3), // attribute name
    at("Text", middle, 2), // text
    at("&copy", middle, 3), // entity
    at("color:", middle, 3), // style attribute
    at("{", text.indexOf("<style>"), 1), // style element
  ];
  req("textDocument/selectionRange", { textDocument, positions: points });
  for (const position of points) {
    req("textDocument/completion", { textDocument, position, context: { triggerKind: 1 } });
    req("textDocument/hover", { textDocument, position });
    req("textDocument/documentHighlight", { textDocument, position });
    req("textDocument/linkedEditingRange", { textDocument, position });
    req("textDocument/rename", { textDocument, position, newName: "renamed" });
  }
  let current = text;
  for (const ch of ["<", "d", "i", "v", ">"]) {
    const offset = current.indexOf("<p", middle);
    const position = positionAt(current, offset);
    send({
      method: "textDocument/didChange",
      params: { textDocument: { uri, version: version++ }, contentChanges: [{ range: { start: position, end: position }, text: ch }] },
    });
    current = current.slice(0, offset) + ch + current.slice(offset);
    req("textDocument/completion", { textDocument, position: positionAt(current, offset + 1), context: { triggerKind: 1 } });
  }
  req("textDocument/diagnostic", { textDocument });
  send({ method: "textDocument/didClose", params: { textDocument } });
}
send({ id: id++, method: "shutdown" });
send({ method: "exit" });
fs.writeFileSync(out, msgs.map((l) => JSON.stringify(l)).join("\n") + "\n");
console.log(`${msgs.length} messages; ${Object.entries(files).map(([n, s]) => `${n} ${(s.length / 1e6).toFixed(2)} MB`).join(", ")}`);
