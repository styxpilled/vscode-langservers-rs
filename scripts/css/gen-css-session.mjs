// Writes a CSS server session that runs each feature on the files in
// .scratch/test_files/css, with the Zed `initialize` of a recorded session.
// Usage: node gen-css-session.mjs <zed session.jsonl> <out.jsonl> [--push]
// `--push`: no pull diagnostics capability (push diagnostics).
import fs from "fs";
import path from "path";
import { pathToFileURL } from "url";

const [base, out, mode] = process.argv.slice(2);
const push = mode === "--push";
const dir = path.resolve(".scratch/test_files/css");
const lines = fs.readFileSync(base, "utf8").split("\n").filter(Boolean).map(JSON.parse);
const init = structuredClone(lines.find((l) => l.dir === "in" && l.msg?.method === "initialize").msg);
const folder = pathToFileURL(dir).href;
init.params.rootPath = dir;
init.params.rootUri = folder;
init.params.workspaceFolders = [{ uri: folder, name: "css" }];
init.params.initializationOptions = { provideFormatter: true };
if (push) delete init.params.capabilities.textDocument.diagnostic;

const msgs = [{ t: 0, dir: "meta", cmd: [] }];
let id = 1000;
let outId = 1;
let t = 0;
const send = (msg) => msgs.push({ t: (t += 10), dir: "in", msg: { jsonrpc: "2.0", ...msg } });
const req = (method, params) => send({ id: id++, method, params });
// A server request and the Zed answer, for `lsp-replay`.
const answer = (method, params, result) => {
  msgs.push({ t: (t += 1), dir: "out", msg: { jsonrpc: "2.0", id: outId, method, params } });
  msgs.push({ t: (t += 1), dir: "in", msg: { jsonrpc: "2.0", id: outId++, result } });
};

const settings = { css: { lint: {} }, less: { lint: {} }, scss: { lint: {} } };
send({ ...init, id: 0 });
send({ method: "initialized", params: {} });
send({ method: "workspace/didChangeConfiguration", params: { settings } });

const positionAt = (text, offset) => {
  const before = text.slice(0, offset);
  const line = before.split("\n").length - 1;
  return { line, character: offset - (before.lastIndexOf("\n") + 1) };
};
const rangeOf = (text, start, end) => ({ start: positionAt(text, start), end: positionAt(text, end) });

const docs = [
  ["main.css", "css", 3],
  ["styles.scss", "scss", 3],
  ["theme.less", "less", 3],
  ["example.scss", "scss", 7],
  ["bootstrap.min.css", "css", 0],
];
for (const [file, languageId, step] of docs) {
  const p = path.join(dir, file);
  const uri = pathToFileURL(p).href;
  const text = fs.readFileSync(p, "utf8");
  const textDocument = { uri };
  for (const section of ["css", "scss", "less"]) {
    answer("workspace/configuration", { items: [{ scopeUri: uri, section }] }, [{ lint: {} }]);
  }
  send({ method: "textDocument/didOpen", params: { textDocument: { uri, languageId, version: 1, text } } });
  if (!push) req("textDocument/diagnostic", { textDocument });
  req("textDocument/documentSymbol", { textDocument });
  req("textDocument/foldingRange", { textDocument });
  req("textDocument/documentColor", { textDocument });
  req("textDocument/documentLink", { textDocument });
  req("textDocument/formatting", { textDocument, options: { tabSize: 4, insertSpaces: true } });
  const offsets = [];
  if (step) for (let o = 0; o <= text.length; o += step) offsets.push(o);
  else for (let o = 0; o < text.length; o += Math.floor(text.length / 40)) offsets.push(o);
  const positions = offsets.map((o) => positionAt(text, o));
  req("textDocument/selectionRange", { textDocument, positions });
  for (const position of positions) {
    req("textDocument/hover", { textDocument, position });
    req("textDocument/completion", { textDocument, position, context: { triggerKind: 1 } });
    req("textDocument/definition", { textDocument, position });
    req("textDocument/documentHighlight", { textDocument, position });
    req("textDocument/references", { textDocument, position, context: { includeDeclaration: true } });
  }
  for (const position of positions.filter((_, i) => i % 9 === 0)) {
    req("textDocument/rename", { textDocument, position, newName: "renamed" });
  }
  const colors = [
    { red: 0.41568628, green: 0.8, blue: 0.2, alpha: 1 },
    { red: 1, green: 0, blue: 0, alpha: 0.5 },
    { red: 0.123456789, green: 0.987654321, blue: 0.5, alpha: 0.33333334 },
  ];
  for (const color of colors) {
    req("textDocument/colorPresentation", { textDocument, color, range: rangeOf(text, 0, 4) });
  }
  const diagnostics = [];
  for (const m of text.matchAll(/\b(colr|widht|backgroud)\b/g)) {
    diagnostics.push({
      range: rangeOf(text, m.index, m.index + m[0].length),
      severity: 2,
      code: "unknownProperties",
      source: languageId,
      message: `Unknown property: '${m[0]}'`,
    });
  }
  for (const d of diagnostics) {
    req("textDocument/codeAction", { textDocument, range: d.range, context: { diagnostics: [d] } });
  }
  const mid = Math.floor(text.length / 2);
  req("textDocument/rangeFormatting", {
    textDocument,
    range: rangeOf(text, Math.floor(text.length / 3), mid),
    options: { tabSize: 2, insertSpaces: false },
  });
  send({
    method: "textDocument/didChange",
    params: {
      textDocument: { uri, version: 2 },
      contentChanges: [{ range: rangeOf(text, mid, mid), text: "\n.added { colr: blue; }\n" }],
    },
  });
  if (!push) req("textDocument/diagnostic", { textDocument });
  req("textDocument/documentSymbol", { textDocument });
  if (file !== "bootstrap.min.css") send({ method: "textDocument/didClose", params: { textDocument } });
}
send({ id: id++, method: "shutdown" });
send({ method: "exit" });
fs.writeFileSync(out, msgs.map((l) => JSON.stringify(l)).join("\n") + "\n");
console.log(`${msgs.length} messages`);
