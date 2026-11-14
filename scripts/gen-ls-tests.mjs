// Generates Rust test functions from vscode-json-languageservice test files.
// Usage: node gen-ls.mjs <upstream src/test dir> <out dir>
import { readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const [, , testDir, outDir] = process.argv;

const prelude = `
const __out = [];
let __cur = null;
function suite(name, fn) { fn(); }
function test(name, fn) { __cur = { name, calls: [] }; __out.push(__cur); try { fn(); } catch (e) { __cur.error = String(e); } }
const __rec = (kind) => (...args) => { __cur.calls.push({ kind, args: args.map((a) => a === undefined ? { __undef: true } : a) }); };
const r = (startLine, endLine, kind) => ({ startLine, endLine, kind });
const getLanguageService = () => ({});
const ClientCapabilities = { LATEST: {} };
const TextDocument = { applyEdits: () => { throw new Error("unused"); } };
const assertRanges = __rec("ranges"), format = __rec("format"), testSort = __rec("sort");
process.on("exit", () => process.stdout.write(JSON.stringify(__out)));
`;

const FILES = {
  "folding.test.ts": "folding",
  "formatter.test.ts": "formatter",
  "selectionRanges.test.ts": "selection",
  "sort.test.ts": "sort",
};

function extract(file) {
  let src = readFileSync(join(testDir, file), "utf8");
  src = src.slice(src.indexOf("suite("));
  src = src.replace(/function (format|testSort|assertRanges)\(/g, "function __shadowed_$1(");
  const harness = join(outDir, file.replace(/\.ts$/, ".harness.mts"));
  writeFileSync(harness, prelude + src);
  return JSON.parse(execFileSync(process.execPath, [harness], { encoding: "utf8" }));
}

const rs = (s) => {
  let out = '"';
  for (const ch of s) {
    const c = ch.codePointAt(0);
    if (ch === "\\") out += "\\\\";
    else if (ch === '"') out += '\\"';
    else if (ch === "\n") out += "\\n";
    else if (ch === "\r") out += "\\r";
    else if (ch === "\t") out += "\\t";
    else if (c < 0x20 || c > 0x7e) out += "\\u{" + c.toString(16) + "}";
    else out += ch;
  }
  return out + '"';
};
const undef = (v) => v && v.__undef;
const kind = (k) => ({ comment: "Some(FoldingRangeKind::Comment)", region: "Some(FoldingRangeKind::Region)" })[k] ?? "None";

function emit(type, call) {
  const a = call.args;
  switch (type) {
    case "folding": {
      const ranges = a[1].map((x) => `(${x.startLine}, ${x.endLine}, ${kind(x.kind)})`).join(", ");
      const limit = a[2] === undefined || undef(a[2]) ? "None" : `Some(${a[2]})`;
      return `assert_ranges(&[${a[0].map(rs).join(", ")}], &[${ranges}], ${limit});`;
    }
    case "selection":
      return `assert_ranges(${rs(a[0])}, &[${a[1].map(([o, t]) => `(${o}, ${rs(t)})`).join(", ")}]);`;
    case "formatter":
      return `check(${rs(a[0])}, ${rs(a[1])}, ${a[2] === undefined || undef(a[2]) ? true : !!a[2]});`;
    case "sort": {
      const o = a[2];
      if (o.tabSize !== 2 || !o.insertSpaces || o.insertFinalNewline) throw new Error("unexpected sort options");
      return `test_sort(${rs(a[0])}, ${rs(a[1])});`;
    }
  }
}

for (const [file, type] of Object.entries(FILES)) {
  const names = new Set();
  let out = "";
  for (const t of extract(file)) {
    let name = t.name.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
    if (/^\d/.test(name)) name = "t_" + name;
    while (names.has(name)) name += "_2";
    names.add(name);
    if (t.error || t.calls.length === 0) {
      out += `\n// Not ported: ${JSON.stringify(t.name)}${t.error ? ` (${t.error})` : ""}\n`;
      continue;
    }
    out += `\n#[test]\nfn ${name}() {\n${t.calls.map((c) => "    " + emit(type, c)).join("\n")}\n}\n`;
  }
  writeFileSync(join(outDir, type + ".generated.rs"), out);
  console.error(file, names.size);
}
