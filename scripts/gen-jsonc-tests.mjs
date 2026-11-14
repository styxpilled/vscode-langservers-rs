// Usage: node gen.mjs <upstream test dir> <out dir>
import { readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const [, , testDir, outDir] = process.argv;

const SYNTAX = ["", "OpenBraceToken", "CloseBraceToken", "OpenBracketToken", "CloseBracketToken", "CommaToken", "ColonToken", "NullKeyword", "TrueKeyword", "FalseKeyword", "StringLiteral", "NumericLiteral", "LineCommentTrivia", "BlockCommentTrivia", "LineBreakTrivia", "Trivia", "Unknown", "EOF"];
const SCAN = ["None", "UnexpectedEndOfComment", "UnexpectedEndOfString", "UnexpectedEndOfNumber", "InvalidUnicode", "InvalidEscapeCharacter", "InvalidCharacter"];
const PARSE = ["", "InvalidSymbol", "InvalidNumberFormat", "PropertyNameExpected", "ValueExpected", "ColonExpected", "CommaExpected", "CloseBraceExpected", "CloseBracketExpected", "EndOfFileExpected", "InvalidCommentToken", "UnexpectedEndOfComment", "UnexpectedEndOfString", "UnexpectedEndOfNumber", "InvalidUnicode", "InvalidEscapeCharacter", "InvalidCharacter"];

// Runtime prelude evaluated before the upstream suite body.
const prelude = `
const __out = [];
let __cur = null;
const __enum = (names) => Object.fromEntries(names.map((n, i) => [n, i]).filter(([n]) => n));
const SyntaxKind = __enum(${JSON.stringify(SYNTAX)});
const ScanError = __enum(${JSON.stringify(SCAN)});
const ParseErrorCode = __enum(${JSON.stringify(PARSE)});
function suite(name, fn) { fn(); }
function test(name, fn) { __cur = { name, calls: [] }; __out.push(__cur); try { fn(); } catch (e) { __cur.error = String(e); } }
const __rec = (kind) => (...args) => { __cur.calls.push({ kind, args: args.map((a) => a === undefined ? { __undef: true } : a) }); };
const assertKinds = __rec("kinds"), assertScanError = __rec("scanError"), assertValidParse = __rec("validParse"),
  assertInvalidParse = __rec("invalidParse"), assertTree = __rec("tree"), assertVisit = __rec("visit"),
  assertLocation = __rec("location"), assertMatchesLocation = __rec("matches"), assertNodeAtLocation = __rec("nodeAtLocation"),
  assertEdit = __rec("edit"), format = __rec("format");
const parseTree = (text) => ({ __tree: text });
const modify = (content, path, value, options) => ({ path, value: value === undefined ? { __undef: true } : value,
  options: { formatting: !!(options && options.formattingOptions), arrayInsertion: !!(options && options.isArrayInsertion),
    insertionIndex: options && options.getInsertionIndex ? options.getInsertionIndex([]) : null } });
process.on("exit", () => process.stdout.write(JSON.stringify(__out)));
`;

function extract(file) {
  let src = readFileSync(join(testDir, file), "utf8");
  src = src.slice(src.indexOf("suite("));
  src = src.replace(/function (assertEdit|format)\(/g, "function __shadowed_$1(");
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
const json = (v) => (undef(v) ? "None" : `Some(r#"${JSON.stringify(v)}"#)`);
const seg = (s) => (typeof s === "number" ? (s < 0 ? "i(usize::MAX)" : `i(${s})`) : `p(${rs(s)})`);
const segs = (a) => `&[${a.map(seg).join(", ")}]`;
const kinds = (a) => `&[${a.map((k) => `SyntaxKind::${SYNTAX[k]}`).join(", ")}]`;
const parseOptions = (o) => {
  if (undef(o) || !o) return "ParseOptions::default()";
  const f = [];
  if (o.disallowComments) f.push("disallow_comments: true");
  if (o.allowTrailingComma) f.push("allow_trailing_comma: true");
  if (o.allowEmptyContent) f.push("allow_empty_content: true");
  return f.length ? `ParseOptions { ${f.join(", ")}, ..Default::default() }` : "ParseOptions::default()";
};
const bool = (v, d) => (v === undefined || undef(v) ? d : !!v);

function emit(call) {
  const a = call.args;
  switch (call.kind) {
    case "kinds": return `assert_kinds(${rs(a[0])}, ${kinds(a.slice(1))});`;
    case "scanError": return `assert_scan_error(${rs(a[0])}, ScanError::${SCAN[a[1]]}, ${kinds(a.slice(2))});`;
    case "validParse": return `assert_valid_parse(${rs(a[0])}, ${json(a[1])}, ${parseOptions(a[2])});`;
    case "invalidParse": return `assert_invalid_parse(${rs(a[0])}, ${json(a[1])}, ${parseOptions(a[2])});`;
    case "tree": return `assert_tree(${rs(a[0])}, r#"${JSON.stringify(a[1])}"#, r#"${JSON.stringify(a[2] && !undef(a[2]) ? a[2] : [])}"#);`;
    case "visit": {
      const stops = a[4] && !undef(a[4]) ? `&[${a[4].join(", ")}]` : "&[]";
      return `assert_visit(${rs(a[0])}, r#"${JSON.stringify(a[1])}"#, r#"${JSON.stringify(a[2] && !undef(a[2]) ? a[2] : [])}"#, ${bool(a[3], false)}, ${stops});`;
    }
    case "location": return `assert_location(${rs(a[0])}, ${segs(a[1])}, ${undef(a[2]) ? "None" : `Some(NodeType::${a[2][0].toUpperCase() + a[2].slice(1)})`}, ${a[3]});`;
    case "matches": return `assert_matches_location(${rs(a[0])}, ${segs(a[1])}, ${bool(a[2], true)});`;
    case "nodeAtLocation": return `assert_node_at_location(${rs(a[0].__tree)}, ${segs(a[1])}, ${json(a[2])});`;
    case "format": return `check(${rs(a[0])}, ${rs(a[1])}, ${bool(a[2], true)}, ${bool(a[3], false)}, ${bool(a[4], false)});`;
    case "edit": {
      const m = a[1];
      const o = [];
      if (m.options.formatting) o.push("formatting_options: Some(formatting_options())");
      if (m.options.arrayInsertion) o.push("is_array_insertion: true");
      if (m.options.insertionIndex !== null) o.push(`get_insertion_index: Some(&|_: &[&str]| ${m.options.insertionIndex})`);
      const opts = o.length ? `ModificationOptions { ${o.join(", ")}, ..Default::default() }` : "ModificationOptions::default()";
      return `assert_edit(${rs(a[0])}, ${segs(m.path)}, ${json(m.value)}, &${opts}, ${rs(a[2])});`;
    }
  }
  throw new Error("unknown call " + call.kind);
}

for (const file of ["json.test.ts", "format.test.ts", "edit.test.ts"]) {
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
    out += `\n#[test]\nfn ${name}() {\n${t.calls.map((c) => "    " + emit(c)).join("\n")}\n}\n`;
  }
  writeFileSync(join(outDir, file.replace(/\.test\.ts$/, ".generated.rs")), out);
  console.error(file, names.size);
}
