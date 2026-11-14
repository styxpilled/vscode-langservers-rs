// Records each source that the upstream tests give to a scanner, then writes the
// full token stream of the upstream scanner for each one.
// Usage: node trace-scanner.mjs <vscode-css-languageservice dir> <out.json>
import fs from 'fs';
import path from 'path';
import url from 'url';

const [root, out] = process.argv.slice(2);
const lib = url.pathToFileURL(path.resolve(root, 'lib/esm')).href;
const { Scanner, TokenType } = await import(`${lib}/parser/cssScanner.js`);
const scss = await import(`${lib}/parser/scssScanner.js`);
const less = await import(`${lib}/parser/lessScanner.js`);

const names = Object.fromEntries(Object.entries(TokenType).filter(([k]) => isNaN(+k)).map(([k, v]) => [v, k]));
for (const n of ['VariableName', 'InterpolationFunction', 'Default', 'EqualsOperator', 'NotEqualsOperator',
	'GreaterEqualsOperator', 'SmallerEqualsOperator', 'Ellipsis', 'Module']) {
	names[scss[n]] = n;
}
const dialectOf = (s) => s instanceof scss.SCSSScanner ? 'scss' : s instanceof less.LESSScanner ? 'less' : 'css';
const typeName = (s, t) => (dialectOf(s) === 'less' && t === less.Ellipsis) ? 'Ellipsis' : names[t];

const inputs = new Map();
const setSource = Scanner.prototype.setSource;
Scanner.prototype.setSource = function (input) {
	const entry = { dialect: dialectOf(this), ignoreWhitespace: this.ignoreWhitespace, ignoreComment: this.ignoreComment, source: input };
	inputs.set(JSON.stringify(entry), entry);
	return setSource.call(this, input);
};

const extras = ['a\\', '"x\\', '#a\\', '@\\', '\\0', '\\0 x', '1.2.3', '1Khz', '1KHZ', '😀a', '@😀',
	'"a😀\\\r\nb"', 'x\\😀y', '\\\n', 'u+1F?-2', 'a // b\nc', '`js` `bad', '$x == 1 != 2 <= 3 >= 4 < > ...',
	'#{a}', '$', '$-', '-\\', '--', '-', '+.5e', 'a/**/b', '/* open', '<!-- -->', '\0a\0', '"a\0b"', 'é\\E9 x\\10000y'];
for (const dialect of ['css', 'scss', 'less']) {
	for (const source of extras) {
		const entry = { dialect, ignoreWhitespace: true, ignoreComment: true, source };
		inputs.set(JSON.stringify(entry), entry);
	}
}

const create = (d) => d === 'scss' ? new scss.SCSSScanner() : d === 'less' ? new less.LESSScanner() : new Scanner();
const tok = (s, t) => t && [typeName(s, t.type), t.offset, t.len, t.text];

process.on('exit', () => {
	const cases = [...inputs.values()].map((e) => {
		const s = create(e.dialect);
		s.ignoreWhitespace = e.ignoreWhitespace;
		s.ignoreComment = e.ignoreComment;
		setSource.call(s, e.source);
		const tokens = [];
		for (let t = s.scan(); ; t = s.scan()) {
			tokens.push(tok(s, t));
			if (t.type === TokenType.EOF) break;
		}
		setSource.call(s, e.source);
		const unquoted = tok(s, s.scanUnquotedString());
		return { ...e, tokens, unquoted };
	});
	fs.writeFileSync(out, JSON.stringify(cases));
	console.error(`${cases.length} sources`);
});

const testDir = path.resolve(root, 'lib/esm/test');
for (const dir of ['css', 'scss', 'less']) {
	for (const f of fs.readdirSync(path.join(testDir, dir)).filter((f) => f.endsWith('.test.js'))) {
		await import(url.pathToFileURL(path.join(testDir, dir, f)).href);
	}
}
