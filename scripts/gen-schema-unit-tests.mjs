// Generates Rust test functions from the upstream glob and vocabularies tests.
// Usage: node gen-schema-unit-tests.mjs <upstream src/test dir> <out dir>
// Writes <out dir>/glob.rs and <out dir>/vocabularies.rs (test functions only).
import { readFileSync, writeFileSync, mkdtempSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

const [, , testDir, outDir] = process.argv;
const tmp = mkdtempSync(join(tmpdir(), 'gen-'));

const prelude = `
const __out = [];
let __cur = null;
function suite(name, fn) { fn(); }
function test(name, fn) { __cur = { name, calls: [] }; __out.push(__cur); fn(); }
const SchemaDraft = { v3: 3, v4: 4, v6: 6, v7: 7, v2019_09: 19, v2020_12: 20 };
const mapArg = m => m === undefined ? null : [...m.entries()];
const isKeywordEnabled = (k, m) => ({ fn: 'kw', k, m: mapArg(m) });
const isFormatAssertionEnabled = (m, d) => ({ fn: 'fmt', m: mapArg(m), d: d ?? null });
const assert = { strictEqual: (actual, expected) => __cur.calls.push({ actual, expected }), fail: () => {} };
function assertMatch(pattern, input, expected) { __cur.calls.push({ pattern, input, expected }); }
process.on('exit', () => process.stdout.write(JSON.stringify(__out)));
`;

function extract(file) {
	let src = readFileSync(join(testDir, file), 'utf8');
	src = src.slice(src.indexOf('suite('));
	src = src.replace(/function assertMatch\(/g, 'function __shadowed_assertMatch(');
	src = src.replace(/new Map<[^>]*>/g, 'new Map');
	const harness = join(tmp, file.replace(/\.ts$/, '.mts'));
	writeFileSync(harness, prelude + src);
	return JSON.parse(execFileSync(process.execPath, ['--experimental-strip-types', harness], { encoding: 'utf8' }));
}

const rs = s => JSON.stringify(s);
const fnName = name => {
	const n = name.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, '');
	return /^[0-9]/.test(n) ? '_' + n : n;
};
const names = new Set();
const uniqueName = name => {
	let n = fnName(name), i = 2;
	while (names.has(n)) {
		n = fnName(name) + '_' + i++;
	}
	names.add(n);
	return n;
};
const drafts = { 3: 'V3', 4: 'V4', 6: 'V6', 7: 'V7', 19: 'V2019_09', 20: 'V2020_12' };
const vocabs = m => m === null ? 'None' : `Some(&vocabs(&[${m.map(([k, v]) => `(${rs(k)}, ${v})`).join(', ')}]))`;

let glob = '';
for (const t of extract('glob.test.ts')) {
	glob += `\n#[test]\nfn ${uniqueName(t.name)}() {\n`;
	for (const c of t.calls) {
		glob += `    assert_match(${rs(c.pattern)}, ${rs(c.input)}, ${c.expected});\n`;
	}
	glob += '}\n';
}
writeFileSync(join(outDir, 'glob.rs'), glob);

let vocab = '';
for (const t of extract('vocabularies.test.ts')) {
	vocab += `\n#[test]\nfn ${uniqueName(t.name)}() {\n`;
	for (const { actual: a, expected } of t.calls) {
		const call = a.fn === 'kw'
			? `is_keyword_enabled(${rs(a.k)}, ${vocabs(a.m)})`
			: `is_format_assertion_enabled(${vocabs(a.m)}, ${a.d === null ? 'None' : `Some(SchemaDraft::${drafts[a.d]})`})`;
		vocab += `    assert_eq!(${call}, ${expected});\n`;
	}
	vocab += '}\n';
}
writeFileSync(join(outDir, 'vocabularies.rs'), vocab);
