// Writes an HTML server session that runs each feature on the files in
// .scratch/test_files/html, with the Zed `initialize` of a recorded session.
// Usage: node gen-html-session.mjs <zed session.jsonl> <out.jsonl> [--push]
// `--push`: no pull diagnostics capability (push diagnostics).
import fs from "fs";
import path from "path";
import { pathToFileURL } from "url";

const [base, out, mode] = process.argv.slice(2);
const push = mode === "--push";
const dir = path.resolve(".scratch/test_files/html");
const lines = fs.readFileSync(base, "utf8").split("\n").filter(Boolean).map(JSON.parse);
const init = structuredClone(lines.find((l) => l.dir === "in" && l.msg?.method === "initialize").msg);
const folder = pathToFileURL(dir).href;
init.params.rootPath = dir;
init.params.rootUri = folder;
init.params.workspaceFolders = [{ uri: folder, name: "html" }];
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

const settings = {
  html: { format: { wrapLineLength: 80, unformatted: "wbr", indentInnerHtml: true }, suggest: { html5: true }, hover: { documentation: true, references: false } },
  css: { lint: { unknownProperties: "error" } },
};
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
  ["page.html", 2],
  ["broken.html", 1],
  ["index.html", 1],
  ["about/about.html", 1],
  ["19813.html", 3],
  ["21634.html", 1],
];
let version = 1;
for (const [file, step] of docs) {
  const p = path.join(dir, file);
  const uri = pathToFileURL(p).href;
  const text = fs.readFileSync(p, "utf8");
  const textDocument = { uri };
  const sections = ["css", "html", "javascript", "js/ts"].map((section) => ({ scopeUri: uri, section }));
  answer("workspace/configuration", { items: sections }, [settings.css, settings.html, null, null]);
  send({ method: "textDocument/didOpen", params: { textDocument: { uri, languageId: "html", version: version++, text } } });
  if (!push) req("textDocument/diagnostic", { textDocument });
  req("textDocument/documentSymbol", { textDocument });
  req("textDocument/foldingRange", { textDocument });
  req("textDocument/documentColor", { textDocument });
  req("textDocument/documentLink", { textDocument });
  req("html/semanticTokens", { textDocument });
  req("textDocument/formatting", { textDocument, options: { tabSize: 4, insertSpaces: true } });
  req("textDocument/formatting", { textDocument, options: { tabSize: 2, insertSpaces: false, insertFinalNewline: true } });
  const offsets = [];
  for (let o = 0; o <= text.length; o += step) {
    if (!/[\udc00-\udfff]/.test(text[o] ?? "")) offsets.push(o);
  }
  const positions = offsets.map((o) => positionAt(text, o));
  req("textDocument/selectionRange", { textDocument, positions });
  for (const position of positions) {
    req("textDocument/hover", { textDocument, position });
    req("textDocument/completion", { textDocument, position, context: { triggerKind: 1 } });
    req("textDocument/documentHighlight", { textDocument, position });
    req("textDocument/definition", { textDocument, position });
    req("textDocument/references", { textDocument, position, context: { includeDeclaration: true } });
    req("textDocument/linkedEditingRange", { textDocument, position });
    req("textDocument/signatureHelp", { textDocument, position });
  }
  for (const position of positions.filter((_, i) => i % 5 === 0)) {
    req("textDocument/rename", { textDocument, position, newName: "renamed" });
    req("html/autoInsert", { textDocument, position, kind: "autoClose" });
    req("html/autoInsert", { textDocument, position, kind: "autoQuote" });
  }
  for (const m of text.matchAll(/#[0-9a-f]{3,6}\b|rgb\([^)]*\)|hsl\([^)]*\)/g)) {
    req("textDocument/colorPresentation", {
      textDocument,
      color: { red: 0.41568628, green: 0.8, blue: 0.2, alpha: 1 },
      range: rangeOf(text, m.index, m.index + m[0].length),
    });
  }
  const third = Math.floor(text.length / 3);
  for (const [a, b] of [[third, 2 * third], [0, text.indexOf("\n") + 1], [third, third]]) {
    req("textDocument/rangeFormatting", { textDocument, range: rangeOf(text, a, b), options: { tabSize: 2, insertSpaces: true } });
  }
  // Typing: each change of one character may give a snippet edit.
  let current = text;
  for (const ch of ["<div", ">", " class", "=", "<br", "/", "<b>x</", "/", "<i>y<", "/"]) {
    const at = current.length;
    const position = positionAt(current, at);
    send({
      method: "textDocument/didChange",
      params: { textDocument: { uri, version: version++ }, contentChanges: [{ range: { start: position, end: position }, text: ch }] },
    });
    current += ch;
    req("textDocument/completion", { textDocument, position: positionAt(current, current.length), context: { triggerKind: 1 } });
  }
  if (!push) req("textDocument/diagnostic", { textDocument });
  req("textDocument/documentSymbol", { textDocument });
  if (file !== "page.html") send({ method: "textDocument/didClose", params: { textDocument } });
}
send({ method: "workspace/didChangeConfiguration", params: { settings: { html: { tagAutoclosing: false, validate: { styles: false } } } } });
const p = path.join(dir, "page.html");
const uri = pathToFileURL(p).href;
answer("workspace/configuration", { items: ["css", "html", "javascript", "js/ts"].map((section) => ({ scopeUri: uri, section })) }, [{}, { validate: { styles: false } }, null, null]);
if (!push) req("textDocument/diagnostic", { textDocument: { uri } });
send({
  method: "textDocument/didChange",
  params: { textDocument: { uri, version: version++ }, contentChanges: [{ range: { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } }, text: ">" }] },
});
req("textDocument/hover", { textDocument: { uri }, position: { line: 12, character: 8 } });
// CSS folding ranges that end on the line of `</style>`, a range format that
// starts in a style region, and hover settings: the CSS mode reads none, the
// HTML mode reads the scoped settings.
const extraText = `<html>
<style>
/*
x
*/</style>
<style>
a {
  color: red;
}</style>
<style>
b {
  color: blue;
  }
</style>
<p>t</p>
</html>
`;
const extra = { uri: pathToFileURL(path.join(dir, "extra.html")).href };
answer("workspace/configuration", { items: ["css", "html", "javascript", "js/ts"].map((section) => ({ scopeUri: extra.uri, section })) }, [{}, { hover: { documentation: false, references: false } }, null, null]);
send({ method: "textDocument/didOpen", params: { textDocument: { ...extra, languageId: "html", version: version++, text: extraText } } });
req("textDocument/hover", { textDocument: extra, position: positionAt(extraText, extraText.indexOf("color")) });
send({ method: "workspace/didChangeConfiguration", params: { settings: { html: { tagAutoclosing: false, validate: { styles: false } } } } });
req("textDocument/hover", { textDocument: extra, position: positionAt(extraText, extraText.indexOf("<p>") + 1) });
req("textDocument/foldingRange", { textDocument: extra });
const styleStart = extraText.indexOf("a {");
req("textDocument/rangeFormatting", { textDocument: extra, range: rangeOf(extraText, styleStart, extraText.indexOf("</html>")), options: { tabSize: 2, insertSpaces: true } });
send({ id: id++, method: "shutdown" });
send({ method: "exit" });
fs.writeFileSync(out, msgs.map((l) => JSON.stringify(l)).join("\n") + "\n");
console.log(`${msgs.length} messages`);
