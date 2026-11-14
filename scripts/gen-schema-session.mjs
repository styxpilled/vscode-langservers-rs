// Writes a large schema, a document that uses it, and a session that opens the document
// several times, with a configuration change between the opens.
// Usage: node gen-schema-session.mjs <zed session.jsonl> <out dir> <property count>
import fs from "fs";
import path from "path";
import { pathToFileURL } from "url";

const [base, outDir, countArg] = process.argv.slice(2);
const count = Number(countArg ?? 20000);
fs.mkdirSync(outDir, { recursive: true });

const properties = {};
for (let i = 0; i < count; i++) {
  properties[`prop${i}`] = {
    type: i % 3 === 0 ? "string" : i % 3 === 1 ? "number" : "boolean",
    description: `Description of property ${i}.`,
    ...(i % 10 === 0 ? { enum: ["a", "b", "c"] } : {}),
  };
}
const schema = { $schema: "http://json-schema.org/draft-07/schema#", type: "object", properties, additionalProperties: false };
const schemaPath = path.resolve(outDir, "big-schema.json");
fs.writeFileSync(schemaPath, JSON.stringify(schema, null, 2));
const schemaUri = pathToFileURL(schemaPath).href;

const doc = { $schema: schemaUri };
for (let i = 0; i < 200; i++) doc[`prop${i * 97}`] = i % 3 === 0 ? "a" : i % 3 === 1 ? i : true;
doc.unknown = 1;
const text = JSON.stringify(doc, null, 2);
const docUri = pathToFileURL(path.resolve(outDir, "uses-big-schema.json")).href;

const lines = fs.readFileSync(base, "utf8").split("\n").filter(Boolean).map(JSON.parse);
const pick = (m) => lines.find((l) => l.dir === "in" && l.msg?.method === m).msg;
const out = [{ t: 0, dir: "meta", cmd: [] }];
let id = 1000;
let t = 0;
const send = (msg) => out.push({ t: (t += 10), dir: "in", msg: { jsonrpc: "2.0", ...msg } });
send(pick("initialize"));
send(pick("initialized"));
const config = pick("workspace/didChangeConfiguration");
for (let round = 0; round < 3; round++) {
  send(config);
  send({ method: "textDocument/didOpen", params: { textDocument: { uri: docUri, languageId: "json", version: 1, text } } });
  send({ id: id++, method: "textDocument/diagnostic", params: { textDocument: { uri: docUri } } });
  send({ id: id++, method: "textDocument/hover", params: { textDocument: { uri: docUri }, position: { line: 2, character: 5 } } });
  send({ id: id++, method: "textDocument/completion", params: { textDocument: { uri: docUri }, position: { line: 1, character: 0 } } });
  send({ method: "textDocument/didClose", params: { textDocument: { uri: docUri } } });
}
send({ id: id++, method: "shutdown" });
send({ method: "exit" });
fs.writeFileSync(path.resolve(outDir, "schema-session.jsonl"), out.map((l) => JSON.stringify(l)).join("\n") + "\n");
console.log(`schema ${fs.statSync(schemaPath).size} bytes, session written`);
