// Records the language facts calls of the upstream tests (`languageFacts.test`
// for CSS and SCSS), plus generated calls, with the upstream results.
// Usage: node trace-facts.mjs <vscode-css-languageservice dir> <fixture.json> [sweep.json]
// The fixture has a sample of the entries; the sweep has all of them.
// Numbers are strings (`String(n)`, `-0` kept), so that the Rust side can
// compare them exactly.
import fs from 'fs';
import module from 'module';
import path from 'path';
import url from 'url';

const [root, out, sweepOut] = process.argv.slice(2);
const libDir = path.resolve(root, 'lib/esm');
const lib = url.pathToFileURL(libDir).href;
const factsUrl = `${lib}/languageFacts/facts.js`;

const exportNames = [];
for (const f of ['entry.js', 'colors.js', 'builtinData.js']) {
	const src = fs.readFileSync(path.join(libDir, 'languageFacts', f), 'utf8');
	for (const m of src.matchAll(/^export (?:function|const|let|class) (\w+)/gm)) exportNames.push(m[1]);
}

const calls = [];
let recording = true;
globalThis.__factsWrap = (name, fn) => function (...args) {
	const result = fn.apply(this, args);
	if (recording && typeof args[0]?.getChildren !== 'function') {
		calls.push({ fn: name, args: enc(args), result: enc(result) });
	}
	return result;
};

module.registerHooks({
	load(u, context, next) {
		if (u === factsUrl) {
			const lines = [`import * as m from ${JSON.stringify(factsUrl + '?orig')};`, 'const wrap = globalThis.__factsWrap;'];
			for (const k of exportNames) {
				lines.push(`export const ${k} = typeof m.${k} === 'function' ? wrap('${k}', m.${k}) : m.${k};`);
			}
			return { format: 'module', source: lines.join('\n'), shortCircuit: true };
		}
		return next(u, context);
	},
});

function enc(v) {
	if (typeof v === 'number') return Object.is(v, -0) ? '-0' : String(v);
	if (v === undefined || v === null) return null;
	if (Array.isArray(v)) return v.map(enc);
	if (typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, enc(x)]));
	return v;
}

const facts = await import(factsUrl);
const nodes = await import(`${lib}/parser/cssNodes.js`);
const { Parser } = await import(`${lib}/parser/cssParser.js`);
const { SCSSParser } = await import(`${lib}/parser/scssParser.js`);
const { LESSParser } = await import(`${lib}/parser/lessParser.js`);
const { CSSDataManager } = await import(`${lib}/languageFacts/dataManager.js`);
const { CSSDataProvider } = await import(`${lib}/languageFacts/dataProvider.js`);
const { cssData } = await import(`${lib}/data/webCustomData.js`);

const texts = new Map();
const parseStylesheet = Parser.prototype.parseStylesheet;
Parser.prototype.parseStylesheet = function (document) {
	const dialect = this instanceof SCSSParser ? 'scss' : this instanceof LESSParser ? 'less' : 'css';
	const text = document.getText();
	texts.set(dialect + '\0' + text, { dialect, text });
	return parseStylesheet.call(this, document);
};

const testDir = path.resolve(root, 'lib/esm/test');
for (const dir of ['css', 'scss']) {
	await import(url.pathToFileURL(path.join(testDir, dir, 'languageFacts.test.js')).href);
}

function colorTexts() {
	const out = [];
	const names = [...Object.keys(facts.colors), 'RED', 'SlateBlue', 'none', 'NONE', 'toString',
		'currentColor', 'transparent', 'foo'];
	for (const n of names) out.push(`a { color: ${n} }`, `a { b: ${n} ${n} }`, `a { b: fn(${n}) }`);
	for (const h of ['#000', '#fff', '#15a', '#96af', '#132435', '#cafebabe', '#12Y', '#ü12', '#1234567', '#12345', '#GgHhIi']) {
		out.push(`a { color: ${h} }`);
	}
	const args = [
		'34, 89, 234', '34 89 234', '34 89 234 / 0.5', '34 89 234 / 50%', '100%, 34%, 10%, 50%', '+78, 40.6, 99%, 1%',
		'-1, 0, 0', '256, 0, 0', '0.5, .5, 5.', '1e2, 0, 0', '1, 2', '1, 2, 3, 4, 5', '$x, 1, 1, 0.7', 'var(--x), 1, 1',
		'120deg, 100%, 50%', '180,100%,25%, 0.33', '0.5turn 100% 50%', '1.5turn, 100%, 50%', '200grad 100% 50%',
		'3.14159rad 100% 50%', '2.35112rad, 76%, 63%', '120DEG 100% 50%', 'none 0% 0%', '-90deg 50% 50%', '400 50% 50%',
		'36 33% 35%', '120 0% 0% / .05', '0 60% 60%', '90 100 100', '90% 50 -50', '46.41 -39.24 33.51', '46.41, 51.60, 139.50',
		'62.8% 56.25% 31.5%', '62.796% 0.22486 0.12585', '.53376 .13032 -.21371', '62.7955% 0.257683 29.2339',
		'0.70167 0.32249 328.36deg', '.8241 26.5225% 0.84891', '0% 0 none', '50% 50% 50% / 20%', '1 1 1 / 1.5',
		'0 0 0 / -0', '100% 100% 100%', '0.3 0.1 -0.1', '70% 0.1 200', '75 30 0.5turn',
	];
	for (const f of ['rgb', 'rgba', 'hsl', 'hsla', 'hwb', 'lab', 'lch', 'oklab', 'oklch', 'RGB', 'Hsl', 'color', 'foo']) {
		for (const a of args) out.push(`a { color: ${f}(${a}) }`);
	}
	return out;
}

function nodeResults(dialect, text) {
	const parser = dialect === 'scss' ? new SCSSParser() : dialect === 'less' ? new LESSParser() : new Parser();
	const doc = { uri: 'test://test/test.css', languageId: dialect, version: 0, getText: () => text };
	const root = parseStylesheet.call(parser, doc);
	const results = [];
	const walk = (node, p) => {
		const isColor = facts.isColorValue(node);
		const color = facts.getColorValue(node);
		if (isColor || color) results.push({ path: p, isColor, color: enc(color) });
		node.getChildren().forEach((c, i) => walk(c, [...p, i]));
	};
	walk(root, []);
	return { dialect, text, nodes: results };
}

function grid(fn, ...lists) {
	const rec = (i, args) => {
		if (i === lists.length) return void fn(...args);
		for (const v of lists[i]) rec(i + 1, [...args, v]);
	};
	rec(0, []);
}

function generatedCalls(colorsFromNodes, full) {
	for (let c = 0; c < 140; c++) facts.hexDigit(c);
	facts.hexDigit(0x2160);
	for (const h of ['#000', '#fff', '#15a', '#96a3', '#cafebabe', '123', '#12Y', '#ü12', '#', '', '#1234567890', '#ABCDEF', '#abcdef']) {
		facts.colorFromHex(h);
	}
	for (const s of [...Object.keys(facts.colors), 'RED', 'Red', 'currentcolor', 'CurrentColor', 'transparent', 'none', 'constructor',
		'#000', '#0000', '#00000', '#000000', '#0000000', '#00000000', '#GGG', 'red ', ' red', 'Ked', 'hſl']) {
		facts.isColorString(s);
	}
	for (const s of ['plain', 'a\\b`c*d_e{f}g[h]i(j)k#l+m-n.o!p', '<div>&amp;', 'ü < ü', '']) facts.textToMarkedString(s);
	const codes = ['E12', 'FF28', 'FFA28', 'S9', 'SM9', 'C29', 'CA29', 'IE11', 'O16', 'X1', 'XE5', ''];
	for (let mask = 0; mask < 1 << 7; mask++) {
		facts.getMissingBaselineBrowsers(codes.slice(0, 7).filter((_, i) => mask & (1 << i)));
	}
	facts.getMissingBaselineBrowsers(codes);
	facts.getMissingBaselineBrowsers(['IE11', 'O16', 'X1']);
	facts.getMissingBaselineBrowsers(['XE5']);
	facts.getMissingBaselineBrowsers(undefined);

	const levels = full ? [0, 1, 50, 127.5, 128, 200, 254, 255] : [0, 50, 127.5, 255];
	const colors = [...colorsFromNodes];
	grid((r, g, b, a) => colors.push(facts.colorFrom256RGB(r, g, b, a)), levels, levels, levels, full ? [1, 0.5] : [1]);
	for (const c of colors) {
		facts.hslFromColor(c);
		facts.hwbFromColor(c);
		const xyz = facts.RGBtoXYZ(c);
		facts.xyzToRGB(xyz);
		facts.XYZtoLAB(xyz);
		facts.XYZtoLAB(xyz, false);
		facts.XYZtoOKLAB(xyz);
		facts.XYZtoOKLAB(xyz, false);
		facts.labFromColor(c);
		facts.labFromColor(c, false);
		facts.oklabFromColor(c);
		facts.oklabFromColor(c, false);
		facts.lchFromColor(c);
		facts.oklchFromColor(c);
	}
	const hues = [-30, 0, 30, 59.9, 60, 90, 120, 139.5, 180, 240, 300, 359, 360, 725];
	const unit = [0, 0.02, 0.25, 0.5, 0.75, 0.92, 1];
	grid(facts.colorFromHSL, hues, unit, unit);
	grid((h, s, l) => facts.colorFromHSL(h, s, l, 0.5), [0, 120], [0.5], [0.5]);
	grid(facts.colorFromHWB, hues, unit, unit);
	const labL = [0, 7.9, 8, 20, 46.41, 50, 90, 100];
	const labAB = [-125, -50, -39.24, 0, 33.51, 50, 125];
	grid(facts.colorFromLAB, labL, labAB, labAB);
	grid((l, a, b) => facts.xyzFromLAB({ l, a, b }), labL, labAB, labAB);
	const lchC = [0, 51.6, 100, 230];
	grid(facts.colorFromLCH, labL, lchC, hues);
	grid(facts.labFromLCH, labL, lchC, hues);
	const okL = [0, 0.25, 0.52, 0.55, 0.86, 1];
	const okAB = [-0.4, -0.14, -0.11, 0, 0.08, 0.11, 0.4];
	grid(facts.colorFromOKLAB, okL, okAB, okAB);
	grid((l, a, b) => facts.xyzFromOKLAB({ l, a, b }), okL, okAB, okAB);
	grid(facts.colorFromOKLCH, okL, [0, 0.13, 0.4], hues);
	grid((x, y, z) => facts.xyzToRGB({ x, y, z, alpha: 1 }), [0, 9.22, 50, 99, 120], [0, 15.58, 76, 100], [0, 5.54, 71, 108.883]);
	grid((x, y, z) => facts.XYZtoOKLAB({ x, y, z, alpha: 1 }), [0, 8.8, 70.1], [0, 15.19, 61.33], [0, 5.24, 71.52]);
}

const entryKinds = ['properties', 'atDirectives', 'pseudoClasses', 'pseudoElements'];

function customEntries() {
	const md = (value) => ({ kind: 'markdown', value });
	const pt = (value) => ({ kind: 'plaintext', value });
	const refs = [{ name: 'MDN', url: 'https://x' }, { name: 'Spec', url: 'https://y' }];
	return [
		{ name: 'a', description: 'Text <b> *x*' },
		{ name: 'b', description: md('**md** <x>'), references: refs },
		{ name: 'c', description: pt('plain *text*'), syntax: '<length>', status: 'nonstandard' },
		{ name: 'd', description: '', references: refs },
		{ name: 'e', description: md(''), syntax: 'x' },
		{ name: 'f', description: 'Obsolete', status: 'obsolete', baseline: { status: 'high', baseline_low_date: '2015-01-01' } },
		{ name: 'g', description: 'Exp', status: 'experimental', baseline: { status: 'false' }, browsers: ['C1', 'FF2'] },
		{ name: 'h', description: 'Low', baseline: { status: 'low' }, syntax: '' },
		{ name: 'i', description: 'No browsers', baseline: { status: 'false' } },
		{ name: 'j', description: 'All', baseline: { status: 'false' }, browsers: ['E1', 'FF1', 'FFA1', 'S1', 'SM1', 'C1', 'CA1'] },
		{ name: 'k', description: 'Refs', references: [] },
		{ name: 'l', description: pt(''), status: 'standard' },
		{ name: 'm', description: { kind: 'other', value: 'v *x*' } },
		{ name: 'n' },
	];
}

const settingsList = [undefined, { documentation: false }, { references: false }, { documentation: false, references: false }];

function entryResults(entries) {
	return entries.map((e) => ({
		entry: e,
		descriptions: [true, false].flatMap((markdown) => settingsList.map((s) => {
			const r = facts.getEntryDescription(e, markdown, s);
			return r ? [r.kind, r.value] : null;
		})),
		missing: facts.getMissingBaselineBrowsers(e.browsers),
	}));
}

function allEntries() {
	const out = [];
	for (const k of entryKinds) {
		for (const e of cssData[k] ?? []) {
			out.push(e);
			for (const v of e.values ?? []) out.push(v);
			for (const d of e.descriptors ?? []) {
				out.push(d);
				for (const v of d.values ?? []) out.push(v);
			}
		}
	}
	return out;
}

function managerResults() {
	const custom = {
		version: 1.1,
		properties: [{ name: 'color', description: 'dup' }, { name: 'x-new' }, 
			{ name: 'x-exp', status: 'experimental' }, { name: 'x-std', status: 'standard' }, { noName: 1 }],
		atDirectives: [{ name: '@x' }, { name: '@media', description: 'dup' }],
		pseudoClasses: 'not an array',
		pseudoElements: [{ name: '::x' }],
	};
	const m = new CSSDataManager({ useDefaultDataProvider: true, customDataProviders: [new CSSDataProvider(custom)] });
	const names = new Set(['color', 'COLOR', 'Color', 'x-new', 'X-NEW', '10', 'tostring',
		'toString', 'x-exp', 'x-std', 'foo', '']);
	for (const p of cssData.properties) {
		names.add(p.name);
		names.add(p.name.toUpperCase());
	}
	return {
		custom,
		properties: m.getProperties().map((p) => p.name),
		atDirectives: m.getAtDirectives().map((p) => p.name),
		pseudoClasses: m.getPseudoClasses().map((p) => p.name),
		pseudoElements: m.getPseudoElements().map((p) => p.name),
		lookups: [...names].map((n) => [n, m.isKnownProperty(n), m.isStandardProperty(n), m.getProperty(n)?.description ?? null]),
	};
}

process.on('exit', () => {
	recording = false;
	const testCalls = calls.splice(0);
	for (const t of colorTexts()) texts.set('css\0' + t, { dialect: 'css', text: t });
	const nodeCases = [...texts.values()].map((t) => nodeResults(t.dialect, t.text));
	const colorsFromNodes = nodeCases.flatMap((c) => c.nodes.map((n) => n.color)).filter(Boolean)
		.map((c) => Object.fromEntries(Object.entries(c).map(([k, v]) => [k, Number(v)])));
	const unique = [...new Map(colorsFromNodes.map((c) => [JSON.stringify(c), c])).values()];
	const generated = (full) => {
		recording = true;
		generatedCalls(full ? unique : unique.filter((_, i) => i % 10 === 0), full);
		recording = false;
		return calls.splice(0);
	};
	const all = allEntries();
	const fixture = {
		testCalls,
		calls: generated(false),
		nodes: nodeCases.filter((_, i) => i % 4 === 0 || i < texts.size - colorTexts().length),
		entries: entryResults([...customEntries(), ...all.filter((_, i) => i % 40 === 0)]),
		manager: managerResults(),
	};
	fs.writeFileSync(out, JSON.stringify(fixture));
	console.error(`${testCalls.length} test calls, ${fixture.calls.length} calls, ${fixture.nodes.length} texts, ${fixture.entries.length} entries`);
	if (sweepOut) {
		const sweep = { testCalls: [], calls: generated(true), nodes: nodeCases, entries: entryResults(all) };
		fs.writeFileSync(sweepOut, JSON.stringify(sweep));
		console.error(`sweep: ${sweep.calls.length} calls, ${nodeCases.length} texts, ${all.length} entries`);
	}
});
