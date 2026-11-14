// Writes parse inputs for the nested sweep: rule set and declaration forms in CSS
// rule bodies and nested at-rules, and namespace prefixes in each dialect, each
// prefix. Checks `may_be_ruleset` and `may_be_namespace_prefix`.
// Usage: node gen-nested-sweep.mjs <out inputs.json>
import fs from 'fs';
const items = [
	'div #b { color: red }', 'div[x] { }', 'div [x=y] {}', 'a:hover{}', 'a:not(.b) {}', 'a::before{}',
	'b > c {}', 'b, c {}', 'b + .c {}', 'b ~ d{}', 'b|c {}', '*|b {}', 'b /deep/ c{}', 'b >>> c {}',
	'& b {}', 'b & {}', 'color: red;', 'color:red}', 'margin: 0 auto;', 'background: url(x.png) no-repeat;',
	'font: 12px/1.5 "x", serif;', '--x: { a: b };', '--y: [1] ;', 'width: calc(1px + 2%);',
	'color: red !important;', 'a: b, c d;', 'a:b:c{}', 'a#b.c:d {}', 'transition: a .3s, b .4s;',
	'x: y z {}', 'x: #fff;', 'x: a(b) {}', 'x:[y] {}', 'x ~= y;', 'x: y |', 'x|y: z;', 'x: y|z {}',
	'unicode-range: u+0-7f;', 'filter: progid:DXImageTransform.Microsoft.gradient(x=1);',
	'x: y\\;z;', 'x: y /* c */ ;', 'a\n{}','x: .5 {}', 'x . y {}', 'x: #a {}', 'x: y, z {}', 'x, y: z;',
	'x::y(z);', 'x: y:hover {}', 'x > y: z;', 'x: >y{}', 'x * y{}', 'x: @y;', 'x: y; z {}', 'x: y! {}',
	'x #{y} {}', 'x: u+12 {}', 'x: "y" {}', 'x: 1px {}', 'x: y = z;', 'x: y % {}', 'x: ;', 'x }',
];
const wrappers = [
	(s) => `.a { ${s} }`, (s) => `.a { b: c; ${s} d: e; }`, (s) => `@media x { ${s} }`,
	(s) => `.a { @media x { ${s} } }`, (s) => `@scope (.a) { ${s} }`, (s) => `@supports (x:y) { ${s} }`,
	(s) => `.a { @supports (x:y) { ${s} } }`, (s) => `@layer l { ${s} }`, (s) => `.a { @layer l { ${s} } }`,
	(s) => `@container c { ${s} }`, (s) => `.a { @container c { ${s} } }`,
	(s) => `@starting-style { ${s} }`, (s) => `.a { @starting-style { ${s} } }`,
];
const namespaces = [
	'|b {}', 'a|b {}', '*|* {}', '*|b {}', 'a | b {}', 'a |b{}', '* | b {}', '|* {}', 'a|*{}', '.a |b {}',
	'a > |b {}', '[|b] {}', '[a|b] {}', '[*|b=c]{}', '[a | b]{}', 'a|b:hover {}', 'a||b {}', 'a|=b {}',
	'a\\|b {}', 'a |  b c|d, |e {}', '.x { a|b { } }', '.x { |b {} }', '.x { *|b: c; }',
];
const inputs = new Map();
for (const text of namespaces) {
	for (let i = 0; i <= text.length; i++) {
		for (const dialect of ['css', 'scss', 'less']) {
			const e = { dialect, fn: '_parseStylesheet', input: text.slice(0, i) };
			inputs.set(JSON.stringify(e), e);
		}
	}
}
for (const item of items) {
	for (const w of wrappers) {
		const text = w(item);
		for (let i = 0; i <= text.length; i++) {
			const e = { dialect: 'css', fn: '_parseStylesheet', input: text.slice(0, i) };
			inputs.set(JSON.stringify(e), e);
		}
	}
	for (let i = 0; i <= item.length; i++) {
		const e = { dialect: 'css', fn: '_parseStylesheet', input: `.a{${item.slice(0, i)}` };
		inputs.set(JSON.stringify(e), e);
	}
}
fs.writeFileSync(process.argv[2], JSON.stringify([...inputs.values()]));
console.error(`${inputs.size} inputs`);
