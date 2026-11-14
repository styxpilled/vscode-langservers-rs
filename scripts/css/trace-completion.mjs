// Records each `doComplete` call of the upstream tests with its result and the
// participant contexts. With a sweep argument, also completes at each offset of
// the sweep texts (CSS, SCSS, Less) and records a hash of each result.
// Usage: node trace-completion.mjs <vscode-css-languageservice dir> <fixture.json> [parser fixture] [sweep.json]
//
// Fixture: `docs` (documentation values), `templates` (items without the range
// of `textEdit`; `documentation` is an index into `docs`), and `calls`. A call
// result has `items` as [template, range index or -1] and `ranges`.
// Sweep: for each text and dialect, `hashes` at each offset (UTF-16, not
// inside a surrogate pair): cyrb53 of the canonical JSON (sorted keys) of
// `{ result, contexts }`, or "error" when upstream throws. Each 7th offset also
// without markdown (`plain`).
//
// Not recorded: the `references` of the SCSS `@use` and `@forward` items, and
// the documentation that each new `SCSSCompletion` appends to (see
// `docs/differences/css-completion.md`).
import fs from 'fs';
import path from 'path';
import url from 'url';

const [root, out, parserFixture, sweepOut] = process.argv.slice(2);
const lib = url.pathToFileURL(path.resolve(root, 'lib/esm')).href;

const { Parser } = await import(`${lib}/parser/cssParser.js`);
const { SCSSParser } = await import(`${lib}/parser/scssParser.js`);
const { LESSParser } = await import(`${lib}/parser/lessParser.js`);
const { CSSCompletion } = await import(`${lib}/services/cssCompletion.js`);
const { SCSSCompletion } = await import(`${lib}/services/scssCompletion.js`);
const { LESSCompletion } = await import(`${lib}/services/lessCompletion.js`);
const { CSSDataManager } = await import(`${lib}/languageFacts/dataManager.js`);
const { TextDocument } = await import(`${lib}/cssLanguageTypes.js`);
const { cssData } = await import(`${lib}/data/webCustomData.js`);

const scssStatic = [...SCSSCompletion.scssModuleLoaders, ...SCSSCompletion.scssModuleBuiltIns];
const scssDocs = scssStatic.map((i) => i.documentation);
function resetScssDocs() {
	scssStatic.forEach((i, n) => {
		i.documentation = { kind: 'markdown', value: scssDocs[n] + '\n\n' + i.references.map((r) => `[${r.name}](${r.url})`).join(' | ') };
	});
}

const dialectOf = (c) => c instanceof SCSSCompletion ? 'scss' : c instanceof LESSCompletion ? 'less' : 'css';
const parserOf = (p) => p instanceof SCSSParser ? 'scss' : p instanceof LESSParser ? 'less' : 'css';
const parsedWith = new WeakMap();
const parseStylesheet = Parser.prototype.parseStylesheet;
Parser.prototype.parseStylesheet = function (document) {
	const node = parseStylesheet.call(this, document);
	parsedWith.set(node, parserOf(this));
	return node;
};

function dataOf(dm) {
	const providers = dm.dataProviders;
	const builtIn = providers[0]?.provideProperties().length === cssData.properties.length;
	const customData = providers.slice(builtIn ? 1 : 0).map((p) => ({
		version: 1,
		properties: p.provideProperties(),
		atDirectives: p.provideAtDirectives(),
		pseudoClasses: p.providePseudoClasses(),
		pseudoElements: p.providePseudoElements(),
	}));
	return { builtIn, customData };
}

function plain(result) {
	const r = JSON.parse(JSON.stringify(result));
	for (const item of r.items ?? []) delete item.references;
	return r;
}

// Runs `doComplete` with a participant that records the contexts.
const doComplete = CSSCompletion.prototype.doComplete;
function complete(service, args) {
	const contexts = { property: [], propertyValue: [], uriLiteralValue: [], importPath: [], mixinReference: [] };
	const recorder = {
		onCssProperty: (c) => contexts.property.push(c),
		onCssPropertyValue: (c) => contexts.propertyValue.push(c),
		onCssURILiteralValue: (c) => contexts.uriLiteralValue.push(c),
		onCssImportPath: (c) => contexts.importPath.push(c),
		onCssMixinReference: (c) => contexts.mixinReference.push(c),
	};
	if (service instanceof SCSSCompletion) resetScssDocs();
	const saved = service.completionParticipants;
	service.completionParticipants = [recorder, ...saved];
	try {
		return { result: doComplete.apply(service, args), contexts: JSON.parse(JSON.stringify(contexts)) };
	} finally {
		service.completionParticipants = saved;
	}
}

const docs = new Map();
const templates = new Map();
const intern = (map, v) => {
	const k = JSON.stringify(v);
	if (!map.has(k)) map.set(k, map.size);
	return map.get(k);
};
function compact(result) {
	const ranges = new Map();
	const items = result.items.map((item) => {
		const t = { ...item };
		let r = -1;
		if (t.textEdit) {
			r = intern(ranges, t.textEdit.range);
			t.textEdit = { newText: t.textEdit.newText };
		}
		if (t.documentation !== undefined) t.documentation = intern(docs, t.documentation);
		return [intern(templates, t), r];
	});
	return { ...result, items, ranges: [...ranges.keys()].map((k) => JSON.parse(k)) };
}

const calls = [];
CSSCompletion.prototype.doComplete = function (document, position, styleSheet, settings) {
	try {
		const { result, contexts } = complete(this, [document, position, styleSheet, settings]);
		calls.push({
			dialect: dialectOf(this), parser: parsedWith.get(styleSheet), input: document.getText(), languageId: document.languageId,
			position, settings: settings ?? null, markdown: this.doesSupportMarkdown(), ...dataOf(this.cssDataManager),
			contexts, result: compact(plain(result)),
		});
		return result;
	} catch (e) {
		calls.push({ dialect: dialectOf(this), input: document.getText(), position, error: String(e) });
		throw e;
	}
};

const testDir = path.resolve(root, 'lib/esm/test');
for (const dir of ['css', 'scss', 'less']) {
	for (const f of fs.readdirSync(path.join(testDir, dir)).filter((f) => f.endsWith('.test.js'))) {
		await import(url.pathToFileURL(path.join(testDir, dir, f)).href);
	}
}

function canonical(v) {
	if (Array.isArray(v)) return '[' + v.map(canonical).join(',') + ']';
	if (v && typeof v === 'object') {
		return '{' + Object.keys(v).filter((k) => v[k] !== undefined).sort().map((k) => JSON.stringify(k) + ':' + canonical(v[k])).join(',') + '}';
	}
	return JSON.stringify(v);
}

// cyrb53 over UTF-16 code units.
function cyrb53(str) {
	let h1 = 0xdeadbeef, h2 = 0x41c6ce57;
	for (let i = 0; i < str.length; i++) {
		const ch = str.charCodeAt(i);
		h1 = Math.imul(h1 ^ ch, 2654435761);
		h2 = Math.imul(h2 ^ ch, 1597334677);
	}
	h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507);
	h1 ^= Math.imul(h2 ^ (h2 >>> 13), 3266489909);
	h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507);
	h2 ^= Math.imul(h1 ^ (h1 >>> 13), 3266489909);
	return 4294967296 * (2097151 & h2) + (h1 >>> 0);
}

const EXTRA = [
	'.a { color: red; background: #fff; } .b { color: blue; margin: 0 auto; } @media screen and (min-width: 100px) { .c { display: block } }',
	':root { --main: #f00; --size: 10px; } .a { color: var(--main); width: calc(var(--size) * 2); border: 1px solid var(--x); }',
	'@supports (display: grid) and (not (display: inline-grid)) { .a { display: grid } } @font-face { font-family: x; src: url(a.woff) }',
	'@import url("foo.css"); @import "bar.css"; .a { background: url(//x/y.png) /* c */ } // line\n.b { }',
	'a { transition: all 1s ease-in; animation: spin 2s; transform: rotate(10deg); clip-path: circle(); background-image: linear-gradient(red, blue) }',
	'$color: red; $size: 10px; @mixin m($a, $b: 2) { width: $a; } @function f($x) { @return $x * 2; } .a { @include m(1px); color: f($size); @extend .b; }',
	'@use "sass:math"; @forward "src/list" hide list-reset; .a { #{$prop}: 1px; width: math.div(1, 2); &:hover { color: darken($color, 10%) } }',
	'@each $name, $glyph in $icons { .icon-#{$name}:before { content: $glyph; } } @for $i from 1 through 3 { .item-#{$i} { width: 2em * $i; } }',
	'@w: 10px; @c: #fff; .m(@a; @b: 2) { width: @a } .a { .m(1px); color: lighten(@c, 10%); @r: { color: red } } .guard when (@mode = huge) { width: 100% }',
	'.a:extend(.b all) { &-x { } @media (min-width: 768px) and (max-width: 979px), (max-width: 480px) { } } @plugin "x"; @import (reference) "foo";',
	'é { cölor: rëd } 😀:hover { margin: 1px } /* 😀 */ .ü { }',
	'a { width: 1px; width: 2em; width: 10; width: 05; width: -3px } b { width: 2 } c { width: 1 }',
	'.a{}.b{}.a.b{}#c{}div.d > span.e ~ .f + .g{}',
	'a { color: ; margin: ; } b { c',
	'@keyframes k { from { top: 0 } 50% { top: 1px } } a { animation-name: k }',
	'@container (min-width: 400px) { .a { } } @layer base, x; @property --p { syntax: "<length>"; } @scope (.a) to (.b) { }',
	'@media (prefers-color-scheme: ) { } @media (hover: h) and (orientation) { }',
];

process.on('exit', () => {
	const fixture = { docs: [...docs.keys()].map((k) => JSON.parse(k)), templates: [...templates.keys()].map((k) => JSON.parse(k)), calls };
	fs.writeFileSync(out, JSON.stringify(fixture));
	console.error(`${calls.length} calls, ${templates.size} templates, ${docs.size} docs`);
	if (!sweepOut) return;

	const texts = new Set(calls.map((c) => c.input));
	for (const c of JSON.parse(fs.readFileSync(parserFixture, 'utf8'))) {
		if (c.fn === '_parseStylesheet' && c.input.length <= 120) texts.add(c.input);
	}
	EXTRA.forEach((t) => texts.add(t));

	const dm = new CSSDataManager({ useDefaultDataProvider: true });
	const services = {
		css: [new CSSCompletion(null, {}, dm), new CSSCompletion(null, { clientCapabilities: {} }, dm)],
		scss: [new SCSSCompletion({}, dm), new SCSSCompletion({ clientCapabilities: {} }, dm)],
		less: [new LESSCompletion({}, dm), new LESSCompletion({ clientCapabilities: {} }, dm)],
	};
	const create = (d) => d === 'scss' ? new SCSSParser() : d === 'less' ? new LESSParser() : new Parser();
	const hash = (service, doc, pos) => {
		try {
			const sheet = parseStylesheet.call(create(dialectOf(service)), doc);
			const { result, contexts } = complete(service, [doc, pos, sheet, undefined]);
			return cyrb53(canonical({ result: plain(result), contexts }));
		} catch {
			return 'error';
		}
	};
	const results = [];
	let n = 0;
	for (const text of texts) {
		for (const dialect of ['css', 'scss', 'less']) {
			const doc = TextDocument.create('test://test/test.' + dialect, dialect, 0, text);
			const r = { dialect, input: text, hashes: [], plain: [] };
			for (let offset = 0; offset <= text.length; offset++) {
				if (/[\uDC00-\uDFFF]/.test(text[offset] ?? '') && /[\uD800-\uDBFF]/.test(text[offset - 1] ?? '')) continue;
				const pos = doc.positionAt(offset);
				r.hashes.push(hash(services[dialect][0], doc, pos));
				if (offset % 7 === 0) r.plain.push(hash(services[dialect][1], doc, pos));
				n++;
			}
			results.push(r);
		}
	}
	fs.writeFileSync(sweepOut, JSON.stringify(results));
	console.error(`${results.length} sweep texts, ${n} offsets`);
});
