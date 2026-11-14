// Formats HTML texts with upstream `format` and several option sets: the full
// document, each line (short texts) and pseudo-random ranges.
// Usage: node trace-format.mjs <vscode-html-languageservice dir> <upstream fixture.json> <vscode dir> <sweep.json>
// The CSS beautifier quirks that the CSS port does not port are patched, as in
// `scripts/css/trace-format.mjs`.
import fs from 'fs';
import module from 'module';
import path from 'path';
import url from 'url';
import { formatTexts, sweepTexts } from './texts.mjs';

const [root, fixture, vscode, out] = process.argv.slice(2);
const lib = url.pathToFileURL(path.resolve(root, 'lib/esm')).href;
const cssBeautifyUrl = `${lib}/beautify/beautify-css.js`;

const patches = [
	['result += this._input.next();', "result += this._input.next() ?? '';"],
	['variableOrRule in this.NESTED_AT_RULE', 'Object.hasOwn(this.NESTED_AT_RULE, variableOrRule)'],
	['variableOrRule in this.CONDITIONAL_GROUP_RULE', 'Object.hasOwn(this.CONDITIONAL_GROUP_RULE, variableOrRule)'],
];

module.registerHooks({
	load(u, context, next) {
		if (u === cssBeautifyUrl) {
			let source = fs.readFileSync(url.fileURLToPath(u), 'utf8');
			for (const [from, to] of patches) {
				if (!source.includes(from)) throw new Error('patch not found: ' + from);
				source = source.split(from).join(to);
			}
			return { format: 'module', source, shortCircuit: true };
		}
		return next(u, context);
	},
});

const { TextDocument } = await import(`${lib}/htmlLanguageTypes.js`);
const { format } = await import(`${lib}/services/htmlFormatter.js`);

const texts = sweepTexts(fixture, vscode, formatTexts);

const optionSets = [
	{ tabSize: 2, insertSpaces: true },
	{ tabSize: 4, insertSpaces: false, endWithNewline: true, indentInnerHtml: true },
	{ tabSize: 2, insertSpaces: true, wrapLineLength: 30, wrapAttributes: 'force' },
	{ tabSize: 2, insertSpaces: true, wrapLineLength: 20, wrapAttributes: 'force-aligned', indentEmptyLines: true },
	{ tabSize: 3, insertSpaces: true, wrapAttributes: 'force-expand-multiline', wrapLineLength: 0, preserveNewLines: false },
	{ tabSize: 2, insertSpaces: true, wrapAttributes: 'aligned-multiple', wrapLineLength: 25, maxPreserveNewLines: 1, indentScripts: 'keep' },
	{ tabSize: 2, insertSpaces: true, wrapAttributes: 'preserve', indentScripts: 'separate', unformatted: 'span, B', contentUnformatted: 'div,script', extraLiners: '' },
	{ tabSize: 2, insertSpaces: true, wrapAttributes: 'preserve-aligned', templating: true, indentHandlebars: true, wrapLineLength: 40 },
	{ tabSize: 1, insertSpaces: false, templating: ['angular', 'smarty'], unformattedContentDelimiter: '{{', wrapAttributesIndentSize: 6 },
	{ tabSize: 2, insertSpaces: true, templating: ['django', 'erb', 'php', 'smarty', 'handlebars', 'angular'], extraLiners: 'p,/div' },
	{ tabSize: 2, insertSpaces: true, css: { newlineBetweenSelectors: false, braceStyle: 'expand', newlineBetweenRules: false, spaceAroundSelectorSeparator: true }, preserveNewLines: false },
	{},
	{ tabSize: 2, insertSpaces: true, templating: ['smarty'], indentHandlebars: true, endWithNewline: true },
	{ tabSize: 2, insertSpaces: true, contentUnformatted: 'script', unformattedContentDelimiter: 'a\\tb' },
];

let seed = 1;
const random = () => {
	seed = (seed * 1103515245 + 12345) % 2147483648;
	return seed / 2147483648;
};
const isLow = (s, i) => /[\uDC00-\uDFFF]/.test(s[i] ?? '') && /[\uD800-\uDBFF]/.test(s[i - 1] ?? '');
const randomOffset = (s) => {
	let o = Math.floor(random() * (s.length + 1));
	if (isLow(s, o)) o--;
	return o;
};

const results = [];
let errors = 0;
const run = (text, doc, o, range) => {
	let result;
	try {
		result = format(doc, range, optionSets[o]);
	} catch (e) {
		errors++;
		result = { error: String(e) };
	}
	results.push({ text, options: o, range: range ?? null, result });
};
for (const text of texts) {
	const doc = TextDocument.create('test://test/test.html', 'html', 1, text);
	for (let o = 0; o < optionSets.length; o++) run(text, doc, o, undefined);
	const lines = doc.lineCount;
	for (const o of [0, 1, 2, 7]) {
		if (lines <= 40) {
			for (let l = 0; l < lines; l++) {
				const end = l + 1 < lines ? doc.offsetAt({ line: l + 1, character: 0 }) : text.length;
				const lineText = text.slice(doc.offsetAt({ line: l, character: 0 }), end).replace(/\r?\n$|\r$/, '');
				run(text, doc, o, { start: { line: l, character: 0 }, end: { line: l, character: lineText.length } });
			}
		}
		for (let k = 0; k < 4; k++) {
			const a = randomOffset(text);
			const b = randomOffset(text);
			run(text, doc, o, { start: doc.positionAt(Math.min(a, b)), end: doc.positionAt(Math.max(a, b)) });
		}
	}
}
fs.writeFileSync(out, JSON.stringify({ optionSets, results }));
console.error(`${texts.length} texts, ${results.length} results, ${errors} errors`);
