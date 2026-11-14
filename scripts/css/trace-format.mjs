// Records the `format` calls of the upstream tests with their results. With
// sweep arguments, also formats each style sheet input with several option
// sets: the full document, each line (short texts) and pseudo-random ranges.
// Usage: node trace-format.mjs <vscode-css-languageservice dir> <fixture.json> [parser fixture sweep.json]
// Two upstream quirks are patched (see `docs/differences/css-formatter.md`).
import fs from 'fs';
import module from 'module';
import path from 'path';
import url from 'url';

const [root, out, parserFixture, sweepOut] = process.argv.slice(2);
const lib = url.pathToFileURL(path.resolve(root, 'lib/esm')).href;
const formatterUrl = `${lib}/services/cssFormatter.js`;
const beautifyUrl = `${lib}/beautify/beautify-css.js`;

const calls = [];
globalThis.__wrapFormat = (fn) => function (document, range, options) {
	const result = fn.call(this, document, range, options);
	calls.push({
		text: document.getText(),
		range: range ?? null,
		options: JSON.parse(JSON.stringify(options ?? null)),
		result: JSON.parse(JSON.stringify(result)),
	});
	return result;
};

const patches = [
	['result += this._input.next();', "result += this._input.next() ?? '';"],
	['variableOrRule in this.NESTED_AT_RULE', 'Object.hasOwn(this.NESTED_AT_RULE, variableOrRule)'],
	['variableOrRule in this.CONDITIONAL_GROUP_RULE', 'Object.hasOwn(this.CONDITIONAL_GROUP_RULE, variableOrRule)'],
];

module.registerHooks({
	load(u, context, next) {
		if (u === formatterUrl) {
			const source = [
				`export * from ${JSON.stringify(u + '?orig')};`,
				`import * as m from ${JSON.stringify(u + '?orig')};`,
				`export const format = globalThis.__wrapFormat(m.format);`,
			].join('\n');
			return { format: 'module', source, shortCircuit: true };
		}
		if (u === beautifyUrl && !process.env.UNPATCHED) {
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

const { TextDocument } = await import(`${lib}/cssLanguageTypes.js`);
const formatter = await import(formatterUrl);

const testDir = path.resolve(root, 'lib/esm/test');
for (const dir of ['css', 'scss', 'less']) {
	for (const f of fs.readdirSync(path.join(testDir, dir)).filter((f) => f.endsWith('.test.js'))) {
		await import(url.pathToFileURL(path.join(testDir, dir, f)).href);
	}
}

process.on('exit', () => {
	const recorded = calls.splice(0);
	fs.writeFileSync(out, JSON.stringify(recorded));
	console.error(`${recorded.length} calls`);
	if (!sweepOut) return;

	const texts = new Set();
	for (const c of JSON.parse(fs.readFileSync(parserFixture, 'utf8'))) {
		if (c.fn === '_parseStylesheet') texts.add(c.input);
	}
	for (const c of recorded) texts.add(c.text);
	const extra = [
		'.a {\n  grid-template-areas: "a b"\n    "c d";\n  grid-template:\n    "x" 1fr\n    "y" 2fr / auto;\n  color: red\n}',
		'.a { color: red }\n/* beautify ignore:start */\n.b    {   x:y }\n/* beautify ignore:end */\n.c{d:e}',
		'/*\tbeautify\tignore:start\t*/ a  {  }  /* beautify ignore:end */ b{c:d}',
		'/* beautify ignore:start */ a { b: c }',
		'/* x /* beautify a:b ignore:start c:d */ .a  { b : c } /* beautify ignore:end */ .d{e:f}',
		'/* beautify ignore:start ignore:end */ .a  { b : c }',
		'/* beautify preserve:start */ .a  { b : c }',
		'  .a{b:c}\n  .d{e:f}',
		'\t\t.a{b:c;\n\n\n\nd:e}',
		'.a{b:c}\r\n.d{e:f}\r.g{h:i}\r\n\r\n\r\n.j{}',
		'$map: (a: 1, b: (c: 2, d: 3), e: 4);\n$x : 1;\n.a { $y: 2; b: $y }',
		'.a { background: url( "x.png" ) , url(x.png) , url() , URL(  y  ) , url(\\)) }',
		'@media screen{.a{b:c}} @supports (display:grid) { .a { b: c } } @document url(x) { .a {} }',
		'@page :first { margin: 1in } @font-face{font-family:x} @keyframes k{from{a:b}to{a:c}}',
		'@constructor { a: b } @toString { a: b } @__proto__ { c: d } @valueOf x;',
		'@{var}: 1; .a { @{prop}: 2; #{$p}: 3; c: #{$q}px }',
		'.a\\:b { c\\!: d \\! e; f: "g\\"h"; i: \'j\\\nk\' }',
		'.a { content: "abc\\',
		'.a { content: "abc',
		'.a { content: "abc\\😀',
		'/* unterminated',
		'.a { b: c } // trailing',
		'.a { font-family: aaaa, bbbb, cccc, dddd, eeee, ffff, gggg; margin: 1px 2px 3px 4px 5px 6px 7px 8px 9px 10px }',
		'.😀😀 > .é😀 { content: "é😀" 😀😀😀 😀😀😀 😀😀😀😀😀😀 😀😀😀😀 😀😀😀😀😀 ; x: 😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀 😀😀 }',
		'a:hover, a::before , &:hover, & :not(.b), .c :is(.d) {} .e { &:hover { f: g } &:focus{} }',
		'a>b+c~d, a > b + c ~ d { e: f>g; h: i + j }',
		'[a = b], [ c ~= "d" ] , [e|=f]{ g: h !important; i: j! important; k: l!ie }',
		'.a { b: c } .d{﻿e: f g }',
		'@include foo({a: b}); .m(@a, {x: y}); .n({ z: w });',
		'@use "a" with ($b: 1, $c: 2); @forward "x" with($y: 2);',
		'.a { b: c; } ; ; .d{}{}{ } }} {{ e',
		'@mixin m($a: 1, $b: 2) { @if $a == $b { c: d } @else { e: f } }',
		'@media (min-width:100px) and (max-width:200px){.a{color:red}.b{color:blue}}',
		'.a{b:c}\n\n\n\n\n.d{e:f}\n\n\n',
		'.a {\n\n\n  b: c;\n\n\n  d: e;\n\n\n}\n\n\n',
		'',
		'   ',
		'\n\n',
		'a',
		'{',
		'}',
		'(',
		')',
		'$',
		'@',
		'\\',
		'.a{b:c}\n',
		'a{}b{}c{}',
		':root{--x:{a:b};--y:  1px   2px}',
		'@import url("foo.css") screen;@import "bar.css";@charset "utf-8";@namespace svg url(x);',
		'.a { filter: progid:DXImageTransform.Microsoft.gradient(startColorstr="#000", endColorstr="#fff"); }',
		'.a { b: calc( 1px+2px ) ; c: calc(1px - -2px) ; d: var( --x , 1px ) }',
		'@media screen { .a { b: c } @media print { .d { e: f } } } .g { h: i }',
		'.a { .b { c: d } e: f; .g & { h: i } }',
		'.a{b:c}/* x */.d{e:f}/* y */\n/* z */',
		'.a { b: c; /* x */ d: e; // y\n f: g }',
		'.a { b:{c:d} e:;f:g; h:(i) { } j:) k:}',
		'.a { b:{ c: d } }\n.e { f:;g:h }',
		'.a { h:(i) { } }', '.a { h:(i);j:k }',
		'/* ignore:start /* beautify */ .a  { b : c }',
		'/* beautify */ .a  { b : c }',
		'.a { b: c\\\\!important; d: e\\!f; g: h \\\\! i }',
		'$ ', '@ ', 'a $ ', '.a {\n  $ }', '.a {\n  b: $ ',
	];
	for (const t of extra) texts.add(t);

	const optionSets = [
		{ tabSize: 2, insertSpaces: true },
		{ tabSize: 4, insertSpaces: false, insertFinalNewline: true },
		{ tabSize: 1, insertSpaces: false },
		{ tabSize: 3, insertSpaces: true, newlineBetweenSelectors: false, newlineBetweenRules: false, spaceAroundSelectorSeparator: true, braceStyle: 'expand' },
		{ tabSize: 2, insertSpaces: true, preserveNewLines: false, indentEmptyLines: true },
		{ tabSize: 2, insertSpaces: true, maxPreserveNewLines: 1, wrapLineLength: 20, insertFinalNewline: true },
		{ tabSize: 4, insertSpaces: true, wrapLineLength: 40, braceStyle: 'expand', indentEmptyLines: true, maxPreserveNewLines: 0 },
		{},
		{ tabSize: 0, insertSpaces: true, wrapLineLength: 1, newlineBetweenSelectors: false },
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
	const run = (text, doc, o, range) => {
		const [edit] = formatter.format(doc, range, optionSets[o]);
		const r = edit.range;
		results.push({ text, options: o, range, result: [r.start.line, r.start.character, r.end.line, r.end.character, edit.newText] });
	};
	for (const text of texts) {
		const doc = TextDocument.create('test://test/test.css', 'css', 1, text);
		for (let o = 0; o < optionSets.length; o++) run(text, doc, o, undefined);
		const lines = doc.lineCount;
		for (const o of [0, 1, 3, 6]) {
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
	fs.writeFileSync(sweepOut, JSON.stringify({ optionSets, results }));
	console.error(`${texts.size} sweep texts, ${results.length} results`);
});
