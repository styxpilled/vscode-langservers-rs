// Records the navigation (without file system), code action, folding and
// selection range calls of the upstream tests with their results. With sweep
// arguments, also runs these features on each style sheet input (CSS, SCSS,
// Less): highlights, definition, rename range and selection ranges at each
// offset, symbols, colors and their presentations, folding, code actions for
// the validation diagnostics and for misspelled properties, and color
// presentations for a grid of colors.
// Usage: node trace-navigation.mjs <vscode-css-languageservice dir> <fixture.json> [parser fixture sweep.json]
// Sweep offsets are UTF-16 offsets, without the offsets inside a surrogate pair.
// The sweep uses a `difference` with one array for each LCS row (see
// `docs/differences/css-navigation.md`).
import fs from 'fs';
import module from 'module';
import path from 'path';
import url from 'url';

const [root, out, parserFixture, sweepOut] = process.argv.slice(2);
const lib = url.pathToFileURL(path.resolve(root, 'lib/esm')).href;
const foldingUrl = `${lib}/services/cssFolding.js`;
const selectionUrl = `${lib}/services/cssSelectionRange.js`;
const stringsUrl = `${lib}/utils/strings.js`;

let depth = 0;
const calls = [];
const origin = new WeakMap();

const docOf = (d) => ({ uri: d.uri, languageId: d.languageId, version: d.version, text: d.getText() });
const sheetOf = (s) => {
	const o = origin.get(s);
	if (!o) throw new Error('style sheet without a recorded parse');
	return o;
};

function wrap(name, fn, record) {
	return function (...args) {
		const top = depth === 0;
		depth++;
		try {
			const result = fn.apply(this, args);
			if (top) calls.push({ fn: name, ...record.call(this, ...args), result: JSON.parse(JSON.stringify(result ?? null)) });
			return result;
		} finally {
			depth--;
		}
	};
}

globalThis.__wrapFolding = (fn) => wrap('getFoldingRanges', fn, (document, context) => ({ document: docOf(document), context: context ?? null }));
globalThis.__fixedDifference = false;
globalThis.__difference = (orig) => (first, second, maxLenDelta = 4) => {
	if (!globalThis.__fixedDifference) return orig(first, second, maxLenDelta);
	const lengthDifference = Math.abs(first.length - second.length);
	if (lengthDifference > maxLenDelta) return 0;
	let prev = new Array(second.length + 1).fill(0);
	for (let i = 1; i <= first.length; i++) {
		const row = [0];
		for (let j = 1; j <= second.length; j++) {
			row[j] = first[i - 1] === second[j - 1] ? prev[j - 1] + 1 : Math.max(prev[j], row[j - 1]);
		}
		prev = row;
	}
	return prev[second.length] - Math.sqrt(lengthDifference);
};
globalThis.__wrapSelection = (fn) => wrap('getSelectionRanges', fn, (document, positions, stylesheet) => ({ document: docOf(document), positions, ...sheetOf(stylesheet) }));

module.registerHooks({
	load(u, context, next) {
		const hooked = { [foldingUrl]: ['getFoldingRanges', '__wrapFolding'], [selectionUrl]: ['getSelectionRanges', '__wrapSelection'] }[u];
		if (u === stringsUrl) {
			const source = [
				`export * from ${JSON.stringify(u + '?orig')};`,
				`import * as m from ${JSON.stringify(u + '?orig')};`,
				`export const difference = globalThis.__difference(m.difference);`,
			].join('\n');
			return { format: 'module', source, shortCircuit: true };
		}
		if (hooked) {
			const [name, wrapper] = hooked;
			const source = [
				`export * from ${JSON.stringify(u + '?orig')};`,
				`import * as m from ${JSON.stringify(u + '?orig')};`,
				`export const ${name} = globalThis.${wrapper}(m.${name});`,
			].join('\n');
			return { format: 'module', source, shortCircuit: true };
		}
		return next(u, context);
	},
});

const { Parser } = await import(`${lib}/parser/cssParser.js`);
const { SCSSParser } = await import(`${lib}/parser/scssParser.js`);
const { LESSParser } = await import(`${lib}/parser/lessParser.js`);
const { CSSNavigation } = await import(`${lib}/services/cssNavigation.js`);
const { CSSCodeActions } = await import(`${lib}/services/cssCodeActions.js`);
const { CSSValidation } = await import(`${lib}/services/cssValidation.js`);
const { CSSDataManager } = await import(`${lib}/languageFacts/dataManager.js`);
const { TextDocument } = await import(`${lib}/cssLanguageTypes.js`);
const { cssData } = await import(`${lib}/data/webCustomData.js`);
const folding = await import(foldingUrl);
const selection = await import(selectionUrl);

const dialectOf = (p) => p instanceof SCSSParser ? 'scss' : p instanceof LESSParser ? 'less' : 'css';
const create = (d) => d === 'scss' ? new SCSSParser() : d === 'less' ? new LESSParser() : new Parser();

const parseStylesheet = Parser.prototype.parseStylesheet;
Parser.prototype.parseStylesheet = function (document) {
	const node = parseStylesheet.call(this, document);
	origin.set(node, { dialect: dialectOf(this), input: document.getText() });
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

const nav = CSSNavigation.prototype;
for (const name of ['findDefinition', 'findReferences', 'findDocumentHighlights', 'prepareRename']) {
	nav[name] = wrap(name, nav[name], (document, position, stylesheet) => ({ document: docOf(document), position, ...sheetOf(stylesheet) }));
}
nav.doRename = wrap('doRename', nav.doRename, (document, position, newName, stylesheet) => ({ document: docOf(document), position, newName, ...sheetOf(stylesheet) }));
for (const name of ['findSymbolInformations', 'findDocumentSymbols', 'findDocumentColors']) {
	nav[name] = wrap(name, nav[name], (document, stylesheet) => ({ document: docOf(document), ...sheetOf(stylesheet) }));
}
nav.getColorPresentations = wrap('getColorPresentations', nav.getColorPresentations, (document, stylesheet, color, range) => ({ color, range }));
const ca = CSSCodeActions.prototype;
for (const name of ['doCodeActions', 'doCodeActions2']) {
	ca[name] = wrap(name, ca[name], function (document, range, context, stylesheet) {
		return { document: docOf(document), diagnostics: context.diagnostics ?? [], ...sheetOf(stylesheet), ...dataOf(this.cssDataManager) };
	});
}

const testDir = path.resolve(root, 'lib/esm/test');
for (const dir of ['css', 'scss', 'less']) {
	for (const f of fs.readdirSync(path.join(testDir, dir)).filter((f) => f.endsWith('.test.js'))) {
		await import(url.pathToFileURL(path.join(testDir, dir, f)).href);
	}
}

process.on('exit', () => {
	fs.writeFileSync(out, JSON.stringify(calls));
	console.error(`${calls.length} calls`);
	if (!sweepOut) return;
	depth++;
	globalThis.__fixedDifference = true;

	const texts = new Set();
	for (const c of JSON.parse(fs.readFileSync(parserFixture, 'utf8'))) {
		if (c.fn === '_parseStylesheet') texts.add(c.input);
	}
	for (const c of calls) if (c.input !== undefined) texts.add(c.input);
	for (const c of calls) if (c.document) texts.add(c.document.text);
	texts.add('é { cölor: red; --x: blue } 😀:hover, a😀 > b::before { color: var(--x) }\n@media screen { @scope (.a) to (.b) { .c#d { } } }');
	texts.add('/* #region a */\n.a {\n  color: red; }\n/*#endregion*/\n/* #region\n*/\n/* #regionx */\n/*\n */\n@media (x) {\n  .b {\n    c: d;\n  }\n}\n/* #endregion */');
	texts.add('// #region\n$a: 1;\n@mixin m($p) {\n  #{$p}: $a;\n}\n// #endregion x\n.c { @include m(x); }\n@function f($x) { @return $x; }\n.d { e: f(1) }');
	texts.add('@use "lib" with ($a: 1, $b: red);\n@forward "x" with ($c: 3 !default);\n.d { e: $a }');
	texts.add('/* #region a\rb */\n.a {\n}\n/* #endregion x y */\n/* #region b c */\n.b {\n}\n/* #endregion */\n/* #region c */\n/* #endregion*/');
	texts.add('/* #region a\r*/\n.a {\n}\n/* #endregion x */\n/* #region b */\n.b {\n}\n/* #endregion */\n/* #region c */\n/* #endregion*/');
	texts.add('@a: red;\n.m(@x) { color: @x }\n.b { .m(@a); background: lighten(@a, 10%) }\n@keyframes k { from { top: 0 } }\n.c { animation: k 1s }');
	texts.add('.a { colr: red; widht: 1px; heigth: 2px; backgroud: blue; -webkit-trnsition: none; dispaly: flex; } .b { --x: 1; colorr: #fff; }');
	texts.add('.a { color: #f00; background: rgb(10 20 30 / 50%); border-color: hsl(120deg 50% 50%); outline-color: hwb(10 20% 30%); c: lab(50% 20 -30); d: lch(50% 30 120 / .5); e: oklab(0.5 0.1 -0.1); f: oklch(60% 0.1 200); g: red; h: transparent; i: #12345678 }');

	// Misspelled properties for the code actions.
	const props = cssData.properties.map((p) => p.name);
	const misspelled = [];
	for (let i = 0; i < props.length; i += 5) {
		const p = props[i];
		const k = (i / 5) % p.length;
		misspelled.push(p.slice(0, k) + p.slice(k + 1));
		if (k + 1 < p.length) misspelled.push(p.slice(0, k) + p[k + 1] + p[k] + p.slice(k + 2));
		misspelled.push(p.slice(0, k) + 'x' + p.slice(k));
	}
	for (let i = 0; i < misspelled.length; i += 40) {
		texts.add('.a {\n' + misspelled.slice(i, i + 40).map((m) => `  ${m}: 1;`).join('\n') + '\n}');
	}

	const dm = new CSSDataManager({ useDefaultDataProvider: true });
	const navs = { css: new CSSNavigation(undefined, true), scss: new CSSNavigation(undefined, false), less: new CSSNavigation(undefined, true) };
	const codeActions = new CSSCodeActions(dm);
	const validation = new CSSValidation(dm);
	const results = [];
	for (const text of texts) {
		for (const dialect of ['css', 'scss', 'less']) {
			const doc = TextDocument.create('test://test/test.' + dialect, dialect, 1, text);
			const sheet = parseStylesheet.call(create(dialect), doc);
			const n = navs[dialect];
			const off = (p) => doc.offsetAt(p);
			const rng = (r) => [off(r.start), off(r.end)];
			const r = { dialect, input: text, highlights: [], definition: [], rename: [], selection: [] };
			for (let offset = 0; offset <= text.length; offset++) {
				if (/[\uDC00-\uDFFF]/.test(text[offset] ?? '') && /[\uD800-\uDBFF]/.test(text[offset - 1] ?? '')) continue;
				const pos = doc.positionAt(offset);
				r.highlights.push(n.findDocumentHighlights(doc, pos, sheet).map((h) => [...rng(h.range), h.kind]));
				const d = n.findDefinition(doc, pos, sheet);
				r.definition.push(d ? rng(d.range) : null);
				const p = n.prepareRename(doc, pos, sheet);
				r.rename.push(p ? rng(p) : null);
				const chain = [];
				for (let s = selection.getSelectionRanges(doc, [pos], sheet)[0]; s; s = s.parent) chain.push(...rng(s.range));
				r.selection.push(chain);
			}
			r.symbols = n.findDocumentSymbols(doc, sheet);
			r.symbolInfos = n.findSymbolInformations(doc, sheet);
			r.colors = n.findDocumentColors(doc, sheet);
			r.presentations = r.colors.map((c) => n.getColorPresentations(doc, sheet, c.color, c.range).map((p) => p.label));
			r.folding = folding.getFoldingRanges(doc, {});
			const diagnostics = validation.doValidation(doc, sheet, { validate: true });
			r.codeActions = codeActions.doCodeActions2(doc, diagnostics[0]?.range, { diagnostics }, sheet);
			r.diagnostics = diagnostics;
			results.push(r);
		}
	}

	const levels = [0, 0.1, 0.25, 0.5, 0.75, 1, 0.41568627450980394, 0.123456789, 0.999];
	const range = { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } };
	const presentations = [];
	const n = navs.css;
	for (const red of levels) for (const green of levels) for (const blue of levels) for (const alpha of [1, 0.5, 0, 0.3333]) {
		const color = { red, green, blue, alpha };
		presentations.push({ color, labels: n.getColorPresentations(undefined, undefined, color, range).map((p) => p.label) });
	}
	fs.writeFileSync(sweepOut, JSON.stringify({ results, presentations }));
	console.error(`${results.length} sweep texts, ${presentations.length} colors`);
});
