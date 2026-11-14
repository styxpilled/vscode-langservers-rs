// Records each `internalParse` call of the upstream tests, then writes the full
// node tree of the upstream parser for each one.
// Usage: node trace-parser.mjs <vscode-css-languageservice dir> <out.json> [extra inputs.json]
// Extra inputs: [{ dialect, fn, input }].
import fs from 'fs';
import path from 'path';
import url from 'url';

const [root, out, extraFile] = process.argv.slice(2);
const lib = url.pathToFileURL(path.resolve(root, 'lib/esm')).href;
const nodes = await import(`${lib}/parser/cssNodes.js`);
const { Parser } = await import(`${lib}/parser/cssParser.js`);
const { SCSSParser } = await import(`${lib}/parser/scssParser.js`);
const { LESSParser } = await import(`${lib}/parser/lessParser.js`);
const { TokenType } = await import(`${lib}/parser/cssScanner.js`);

const dialectOf = (p) => p instanceof SCSSParser ? 'scss' : p instanceof LESSParser ? 'less' : 'css';
const create = (d) => d === 'scss' ? new SCSSParser() : d === 'less' ? new LESSParser() : new Parser();

const inputs = new Map();
const add = (e) => inputs.set(JSON.stringify(e), e);
const internalParse = Parser.prototype.internalParse;
Parser.prototype.internalParse = function (input, f, textProvider) {
	add({ dialect: dialectOf(this), fn: f.name.replace(/^bound /, ''), input });
	return internalParse.call(this, input, f, textProvider);
};

const SKIP = new Set(['parent', 'offset', 'length', 'nodeType', 'textProvider', 'options', 'issues', 'children']);

function dump(node) {
	const children = node.getChildren();
	const out = { c: node.constructor.name, t: nodes.NodeType[node.type], o: node.offset, l: node.length };
	const issues = node.issues;
	if (issues && issues.length) {
		out.i = issues.map((m) => [m.getRule().id, m.getOffset(), m.getLength(), m.getLevel()]);
	}
	const fields = {};
	const scalars = {};
	for (const [k, v] of Object.entries(node)) {
		if (SKIP.has(k) || v === undefined) continue;
		if (v instanceof nodes.Node) {
			fields[k] = children.indexOf(v);
			if (fields[k] === -1) fields[k] = dump(v);
		} else {
			scalars[k] = v;
		}
	}
	if (Object.keys(fields).length) out.f = fields;
	if (Object.keys(scalars).length) out.s = scalars;
	if (children.length) out.ch = children.map(dump);
	return out;
}

function run(e) {
	const p = create(e.dialect);
	const node = internalParse.call(p, e.input, p[e.fn]);
	return { ...e, tree: node ? dump(node) : null, next: [TokenType[p.token.type] ?? p.token.type, p.token.offset] };
}

process.on('exit', () => {
	if (extraFile) {
		for (const e of JSON.parse(fs.readFileSync(extraFile, 'utf8'))) add(e);
	}
	const cases = [...inputs.values()].map(run);
	fs.writeFileSync(out, JSON.stringify(cases));
	console.error(`${cases.length} parses`);
});

const testDir = path.resolve(root, 'lib/esm/test');
for (const dir of ['css', 'scss', 'less']) {
	for (const f of fs.readdirSync(path.join(testDir, dir)).filter((f) => f.endsWith('.test.js'))) {
		await import(url.pathToFileURL(path.join(testDir, dir, f)).href);
	}
}
