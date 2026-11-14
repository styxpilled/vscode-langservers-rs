// Records the hover and selector printing calls of the upstream tests with
// their results. With sweep arguments, also hovers at each offset of each
// style sheet input (CSS, SCSS, Less) and prints each selector.
// Usage: node trace-hover.mjs <vscode-css-languageservice dir> <fixture.json> [parser fixture sweep.json]
// A node is given as its parse (`dialect`, `fn`, `input`) and the child index
// path from the parse result. Offsets in the sweep are UTF-16 offsets, without
// the offsets inside a surrogate pair.
import fs from 'fs';
import module from 'module';
import path from 'path';
import url from 'url';

const [root, out, parserFixture, sweepOut] = process.argv.slice(2);
const libDir = path.resolve(root, 'lib/esm');
const lib = url.pathToFileURL(libDir).href;
const printingUrl = `${lib}/services/selectorPrinting.js`;

let depth = 0;
const calls = [];
const origin = new WeakMap();

function locate(node) {
	const p = [];
	while (node.parent) {
		p.unshift(node.parent.getChildren().indexOf(node));
		node = node.parent;
	}
	const o = origin.get(node);
	if (!o) throw new Error('node without a recorded parse');
	return { ...o, path: p };
}

function dumpElement(e) {
	const out = {};
	if (e.constructor.name !== 'Element') out.k = e.constructor.name;
	if (e.attributes) out.a = e.attributes.map((a) => [a.name, a.value ?? null]);
	if (e.children) out.c = e.children.map(dumpElement);
	return out;
}

globalThis.__printingWrap = (name, fn) => function (...args) {
	const top = depth === 0;
	depth++;
	try {
		const result = fn.apply(this, args);
		if (top) calls.push({ fn: name, node: locate(args[0]), parent: args[1] ? dumpElement(args[1]) : null, result: result ? dumpElement(result) : null });
		return result;
	} finally {
		depth--;
	}
};

module.registerHooks({
	load(u, context, next) {
		if (u === printingUrl) {
			const source = [
				`export * from ${JSON.stringify(printingUrl + '?orig')};`,
				`import * as m from ${JSON.stringify(printingUrl + '?orig')};`,
				`export const toElement = globalThis.__printingWrap('toElement', m.toElement);`,
				`export const selectorToElement = globalThis.__printingWrap('selectorToElement', m.selectorToElement);`,
			].join('\n');
			return { format: 'module', source, shortCircuit: true };
		}
		return next(u, context);
	},
});

const nodes = await import(`${lib}/parser/cssNodes.js`);
const { Parser } = await import(`${lib}/parser/cssParser.js`);
const { SCSSParser } = await import(`${lib}/parser/scssParser.js`);
const { LESSParser } = await import(`${lib}/parser/lessParser.js`);
const printing = await import(printingUrl);
const { CSSHover } = await import(`${lib}/services/cssHover.js`);
const { CSSDataManager } = await import(`${lib}/languageFacts/dataManager.js`);
const { TextDocument } = await import(`${lib}/cssLanguageTypes.js`);
const { cssData } = await import(`${lib}/data/webCustomData.js`);

const dialectOf = (p) => p instanceof SCSSParser ? 'scss' : p instanceof LESSParser ? 'less' : 'css';
const create = (d) => d === 'scss' ? new SCSSParser() : d === 'less' ? new LESSParser() : new Parser();

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

for (const name of ['selectorToMarkedString', 'simpleSelectorToMarkedString']) {
	const orig = printing.SelectorPrinting.prototype[name];
	printing.SelectorPrinting.prototype[name] = function (node, contexts) {
		const top = depth === 0;
		depth++;
		try {
			const result = orig.call(this, node, contexts);
			if (top) calls.push({ fn: name, node: locate(node), contexts: contexts ?? null, ...dataOf(this.cssDataManager), result });
			return result;
		} finally {
			depth--;
		}
	};
}

const doHover = CSSHover.prototype.doHover;
CSSHover.prototype.doHover = function (document, position, stylesheet, settings = this.defaultSettings) {
	depth++;
	try {
		const result = doHover.call(this, document, position, stylesheet, settings);
		calls.push({
			...origin.get(stylesheet), fn: 'doHover', languageId: document.languageId, position, settings: settings ?? null,
			capabilities: this.clientCapabilities ?? null, ...dataOf(this.cssDataManager), result,
		});
		return result;
	} finally {
		depth--;
	}
};

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

	const texts = new Set();
	for (const c of JSON.parse(fs.readFileSync(parserFixture, 'utf8'))) {
		if (c.fn === '_parseStylesheet') texts.add(c.input);
	}
	for (const c of calls) texts.add(c.node?.input ?? c.input);
	texts.add('é { cölor: red } 😀:hover, a😀 > b::before { }\n@media screen { @scope (.a) to (.b) { .c#d[e~="f"] { } } }');
	texts.add('a { &:hover { & + & { } } } @at-root .x { } .y { @at-root .z { } }');
	texts.add(':is(#a, .b) :where(#c) :not(a.b) :has(> c) :host(.x) :host-context(#y) ::slotted(span.z) :nth-child(2n+1 of li.a, #b) :nth-last-child(n of .c) :nth-child(-n of #d) ::-webkit-x :NOT(#e)');
	texts.add('[a|="b"] [c^=d] [e$="f"] [g*=\'h\'] [i~=j] [k=l i] [ns|m] [*|n] [o] .\\31 0 #\\61 b \\65 l');

	const dm = new CSSDataManager({ useDefaultDataProvider: true });
	const hovers = [new CSSHover(undefined, dm), new CSSHover({ textDocument: { hover: { contentFormat: ['plaintext'] } } }, dm)];
	const printer = new printing.SelectorPrinting(dm);
	const contents = new Map();
	const id = (v) => {
		const k = JSON.stringify(v);
		if (!contents.has(k)) contents.set(k, contents.size);
		return contents.get(k);
	};
	const results = [];
	depth++;
	for (const text of texts) {
		for (const dialect of ['css', 'scss', 'less']) {
			const doc = TextDocument.create('test://test/test.' + dialect, dialect, 0, text);
			const sheet = parseStylesheet.call(create(dialect), doc);
			const r = { dialect, input: text, hover: [], plain: [], selectors: [] };
			for (let offset = 0; offset <= text.length; offset++) {
				if (/[\uDC00-\uDFFF]/.test(text[offset] ?? '') && /[\uD800-\uDBFF]/.test(text[offset - 1] ?? '')) continue;
				const pos = doc.positionAt(offset);
				const h = hovers[0].doHover(doc, pos, sheet);
				r.hover.push(h ? [id(h.contents), doc.offsetAt(h.range.start), doc.offsetAt(h.range.end)] : null);
				if (offset % 3 === 0) {
					const p = hovers[1].doHover(doc, pos, sheet);
					r.plain.push(p ? id(p.contents) : null);
				}
			}
			const walk = (node) => {
				if (node.type === nodes.NodeType.Selector) {
					const e = printing.selectorToElement(node);
					r.selectors.push([id(e ? dumpElement(e) : null), id(printer.selectorToMarkedString(node, ['ctx']))]);
				}
				if (node.type === nodes.NodeType.SimpleSelector) {
					r.selectors.push([id(dumpElement(printing.toElement(node))), id(printer.simpleSelectorToMarkedString(node))]);
				}
				node.getChildren().forEach(walk);
			};
			walk(sheet);
			results.push(r);
		}
	}
	fs.writeFileSync(sweepOut, JSON.stringify({ contents: [...contents.keys()].map((k) => JSON.parse(k)), results }));
	console.error(`${results.length} sweep texts, ${contents.size} distinct results`);
});
