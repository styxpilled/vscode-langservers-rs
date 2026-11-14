// Writes a session that runs each feature on large documents.
// Usage: node gen-perf-session.mjs <zed session.jsonl> <out.jsonl>
// Needs .scratch/schema-mem/ from gen-schema-session.mjs.
import fs from "fs";
import path from "path";
import { pathToFileURL } from "url";

const [base, out] = process.argv.slice(2);
const dir = path.resolve(".scratch");
const lines = fs.readFileSync(base, "utf8").split("\n").filter(Boolean).map(JSON.parse);
const pick = (m) => lines.find((l) => l.dir === "in" && l.msg?.method === m).msg;

const pretty = path.join(dir, "test_files/large-pretty.json");
if (!fs.existsSync(pretty)) {
  const v = JSON.parse(fs.readFileSync(path.join(dir, "test_files/large-file.json"), "utf8"));
  fs.writeFileSync(pretty, JSON.stringify(v, null, 2));
}
const docs = [
  ["test_files/large-pretty.json", { line: 40, character: 10 }],
  ["test_files/large-file.json", { line: 0, character: 50 }],
  ["schema-mem/uses-big-schema.json", { line: 2, character: 5 }],
];

const msgs = [{ t: 0, dir: "meta", cmd: [] }];
let id = 1000;
let t = 0;
const send = (msg) => msgs.push({ t: (t += 10), dir: "in", msg: { jsonrpc: "2.0", ...msg } });
const req = (method, params) => send({ id: id++, method, params });
send(pick("initialize"));
send(pick("initialized"));
send(pick("workspace/didChangeConfiguration"));
for (const [file, position] of docs) {
  const p = path.join(dir, file);
  const uri = pathToFileURL(p).href;
  const textDocument = { uri };
  send({ method: "textDocument/didOpen", params: { textDocument: { uri, languageId: "json", version: 1, text: fs.readFileSync(p, "utf8") } } });
  req("textDocument/diagnostic", { textDocument });
  req("textDocument/documentSymbol", { textDocument });
  req("textDocument/foldingRange", { textDocument });
  req("textDocument/documentColor", { textDocument });
  req("textDocument/documentLink", { textDocument });
  req("textDocument/selectionRange", { textDocument, positions: [position] });
  req("textDocument/hover", { textDocument, position });
  req("textDocument/completion", { textDocument, position });
  req("textDocument/formatting", { textDocument, options: { tabSize: 4, insertSpaces: true } });
  send({ method: "textDocument/didChange", params: { textDocument: { uri, version: 2 }, contentChanges: [{ range: { start: { line: 0, character: 1 }, end: { line: 0, character: 1 } }, text: " " }] } });
  req("textDocument/diagnostic", { textDocument });
  send({ method: "textDocument/didClose", params: { textDocument } });
}
send({ id: id++, method: "shutdown" });
send({ method: "exit" });
fs.writeFileSync(out, msgs.map((l) => JSON.stringify(l)).join("\n") + "\n");
