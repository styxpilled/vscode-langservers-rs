// Records each `LintVisitor.entries` call of the upstream tests with the parse
// that made its node. With a sweep input file, also lints each style sheet
// input (all levels, default settings and a `validProperties` setting).
// Also records each `CSSValidation.doValidation` call of the tests, and calls
// with generated texts and settings, in `<fixture dir>/validation.json`.
// Usage: node trace-lint.mjs <vscode-css-languageservice dir> <fixture.json> [sweep inputs.json sweep.json]
// Offsets and lengths are UTF-16.
import fs from 'fs';
import path from 'path';
import url from 'url';

const [root, out, sweepIn, sweepOut] = process.argv.slice(2);
const lib = url.pathToFileURL(path.resolve(root, 'lib/esm')).href;
const nodes = await import(`${lib}/parser/cssNodes.js`);
const { Parser } = await import(`${lib}/parser/cssParser.js`);
const { SCSSParser } = await import(`${lib}/parser/scssParser.js`);
const { LESSParser } = await import(`${lib}/parser/lessParser.js`);
const { LintVisitor } = await import(`${lib}/services/lint.js`);
const { LintConfigurationSettings } = await import(`${lib}/services/lintRules.js`);
const { CSSDataManager } = await import(`${lib}/languageFacts/dataManager.js`);
const { TextDocument } = await import(`${lib}/cssLanguageTypes.js`);
const { CSSValidation } = await import(`${lib}/services/cssValidation.js`);
const { cssData } = await import(`${lib}/data/webCustomData.js`);

const dialectOf = (p) => p instanceof SCSSParser ? 'scss' : p instanceof LESSParser ? 'less' : 'css';
const create = (d) => d === 'scss' ? new SCSSParser() : d === 'less' ? new LESSParser() : new Parser();

const origin = new WeakMap();
const internalParse = Parser.prototype.internalParse;
Parser.prototype.internalParse = function (input, f, textProvider) {
	const node = internalParse.call(this, input, f, textProvider);
	if (node) origin.set(node, { dialect: dialectOf(this), fn: f.name.replace(/^bound /, ''), input });
	return node;
};
const parseStylesheet = Parser.prototype.parseStylesheet;
Parser.prototype.parseStylesheet = function (document) {
	const node = parseStylesheet.call(this, document);
	origin.set(node, { dialect: dialectOf(this), fn: '_parseStylesheet', input: document.getText() });
	return node;
};

const encEntries = (entries) => entries.map((e) => [e.getRule().id, e.getLevel(), e.getMessage(), e.getOffset(), e.getLength()]);

// The data manager as `builtIn` and `customData` (CSSDataV1 objects).
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

const calls = [];
const entries = LintVisitor.entries;
LintVisitor.entries = function (node, document, settings, dataManager, filter) {
	const result = entries.call(this, node, document, settings, dataManager, filter);
	const o = origin.get(node);
	if (!o) throw new Error('lint of a node without a recorded parse');
	if (document.getText() !== o.input) throw new Error('document text differs from the parse input');
	calls.push({ ...o, settings: settings.conf, filter: filter ?? null, ...dataOf(dataManager), entries: encEntries(result) });
	return result;
};

const validations = [];
const doValidation = CSSValidation.prototype.doValidation;
CSSValidation.prototype.doValidation = function (document, stylesheet, settings = this.settings) {
	const result = doValidation.call(this, document, stylesheet, settings);
	const o = origin.get(stylesheet);
	validations.push({ dialect: o.dialect, languageId: document.languageId, input: document.getText(), settings: settings ?? null, ...dataOf(this.cssDataManager), result });
	return result;
};

const testDir = path.resolve(root, 'lib/esm/test');
for (const dir of ['css', 'scss', 'less']) {
	for (const f of fs.readdirSync(path.join(testDir, dir)).filter((f) => f.endsWith('.test.js'))) {
		await import(url.pathToFileURL(path.join(testDir, dir, f)).href);
	}
}

const validationTexts = [
	'a { color: red }\n\n@foo x;\r\nb {\r\n  _x: 1;\n\tcolor: red;; }\r\n',
	'é { cölor: 0px }\n/* ü 😀 */ #x 😀 { width: 1px; padding: 1px }\n@-moz-keyframes k { }',
	'a { color: \n',
	'@import "x"\na { b: c; d: e } }\n} {',
	'.a { float: left !important; display: inline-block }\n* {}\n',
	'',
];
const validationSettings = [
	undefined,
	{ validate: false },
	{ validate: true, lint: { emptyRules: 'error', important: 'warning', float: 'ignore', unknownAtRules: 'x', idSelector: 1, universalSelector: 'error' } },
	{ lint: { boxModel: 'warning', zeroUnits: 'error', ieHack: 'warning', unknownProperties: 'ignore', compatibleVendorPrefixes: 'warning', importStatement: 'warning' } },
	{ lint: { validProperties: ['cölor', ' _X '], unknownProperties: 'error', vendorPrefix: 'ignore' } },
];

process.on('exit', () => {
	const vdm = new CSSDataManager({ useDefaultDataProvider: true });
	for (const text of validationTexts) {
		for (const dialect of ['css', 'scss', 'less']) {
			for (const settings of validationSettings) {
				const doc = TextDocument.create('test://test/test.css', dialect, 0, text);
				new CSSValidation(vdm).doValidation(doc, create(dialect).parseStylesheet(doc), settings);
			}
		}
	}
	fs.writeFileSync(path.join(path.dirname(out), 'validation.json'), JSON.stringify(validations));
	console.error(`${validations.length} validations`);
	fs.writeFileSync(out, JSON.stringify(calls));
	console.error(`${calls.length} lint calls`);
	if (!sweepIn) return;
	const dm = new CSSDataManager({ useDefaultDataProvider: true });
	const all = nodes.Level.Error | nodes.Level.Warning | nodes.Level.Ignore;
	const settingsList = [{}, { validProperties: ['float', 'color', 'foo', 'width'] }];
	const inputs = JSON.parse(fs.readFileSync(sweepIn, 'utf8'));
	const extra = [
		'a { color: #12; b: #1234567; c: #ü12; d: #12345; e: #1234; f: #12345678 }',
		'a { color: rgb(1, 2); b: rgba(1, 2, 3); c: hsl(1 2 3 / 4) }',
		'a::-webkit-x { -webkit-transition: none; } a::-moz-x { b { -webkit-transition: none; -moz-transition: none } }',
		'a { padding-inline: 1px; width: 1px; padding-top: 0; padding-left: 1em; height: 2px }',
		'a { border-left: 1px solid; border-right-width: 0; border-top-style: none; border-bottom-color: red; width: 1px; height: 1px }',
		'a { border-style: none solid; border-width: 1px 0 0; width: 1px; height: 1px; border-x-y: 1px }',
		'a { border: 1px initial; width: 1px } b { border: initial 1px; height: 1px; padding: -1px; padding: .0e1px; padding: 1e-400px }',
		'a { padding: +0.0px Infinity; width: 1px; height: 1px; padding-top: unset; padding-right: initial }',
		'a { display: inline-block; float: left; float: none; FLOAT: right; display: block; vertical-align: top; Vertical-Align: x }',
		'a { display: x inline-block; float: x } b { display: block !important; vertical-align: 1 }',
		'a { color: red; Color: blue; color: -x; color: red !important; background: 1; background: 2 }',
		'a { _color: red; *color: blue; _foo: 1; *zoom: 1; -webkit-box-shadow: 1; -foo-bar: 2; --x: 3 }',
		'a { width: 0px; width: 0PX; width: 0.0em; width: 00vh; width: 0.px; width: .0q; width: 0s; width: 0deg; margin: calc(0px + 1px) min(0px, 1px) }',
		'a { b: 0px } @media (min-width: 0px) { a { b: 0px } } @supports (width: 0px) { }',
		'@font-face { src: x; font-family: y } @font-face { SRC: x; Font-Family: y } @font-face { #{$x}: 1 } @font-face { src: ; }',
		'@-webkit-keyframes a { } @-moz-keyframes a { } @-o-keyframes a { } @keyframes b { } @-webkit-keyframes b { } @-ms-keyframes c { }',
		'@-webkit-keyframes x { } @-webkit-keyframes y { } @keyframes x { }',
		'@foo bar; @charset "x"; @Media screen { } @-moz-document url(x) { } @apply --x; @tailwind base;',
		'@import "a"; @import url(b) screen; * { } *.a { } a * { } #a, #b { } a#b:not(#c) { }',
		':export { a: 1; b: 2 } :import("x") { c: 1 } :import(x) { d: 1 } :export, a { e: 1 }',
		'é { cölor: red; -wébkit-x: 1; wïdth: 0px; border: 1px; height: 1px } ü😀 { }',
		'a { #{$p}: 1; foo-#{$p}: 2; @include x; $v: 1; width: 1px; padding: 1px }',
		'.a { .b; .c(); color: red } .d when (iscolor(@c)) { } @r: { a: b }',
		'a { b { } c: d; e { f: g } } a { & { } &:hover { width: 0px } }',
		'a { color: red !ie; color: blue ! important; color: green !IMPORTANT }',
	];
	for (const text of extra) {
		for (let i = 0; i <= text.length; i++) {
			if (/[\uD800-\uDBFF]$/.test(text.slice(0, i))) continue;
			for (const dialect of ['css', 'scss', 'less']) inputs.push({ dialect, input: text.slice(0, i) });
		}
	}
	for (const c of calls) {
		for (let i = 0; i <= c.input.length; i++) inputs.push({ dialect: c.dialect, input: c.input.slice(0, i) });
	}
	const unique = new Map(inputs.map(({ dialect, input }) => [dialect + '\0' + input, { dialect, input }]));
	const results = [];
	for (const { dialect, input } of unique.values()) {
		const doc = TextDocument.create('test://test/test.css', dialect, 0, input);
		const node = parseStylesheet.call(create(dialect), doc);
		const r = { dialect, input };
		settingsList.forEach((s, i) => {
			try {
				r['e' + i] = encEntries(entries.call(LintVisitor, node, doc, new LintConfigurationSettings(s), dm, all));
			} catch (e) {
				r['e' + i] = String(e);
			}
		});
		results.push(r);
	}
	fs.writeFileSync(sweepOut, JSON.stringify(results));
	console.error(`${results.length} sweep inputs`);
});
