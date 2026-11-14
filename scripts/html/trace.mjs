// Runs the upstream tests and records each top-level call of the scanner, the
// parser and the language service features with its full result.
// Usage: node trace.mjs <vscode-html-languageservice dir> <out.json>
// Offsets are UTF-16 offsets. Documents are recorded as text; the Rust test
// parses them again. `resolveReference` and `readDirectory` calls are recorded
// with their results; the Rust test answers from them.
import fs from 'fs';
import module from 'module';
import path from 'path';
import url from 'url';

const [root, out] = process.argv.slice(2);
const lib = url.pathToFileURL(path.resolve(root, 'lib/esm')).href;
const scannerUrl = `${lib}/parser/htmlScanner.js`;

const freeFunctions = {
	[`${lib}/services/htmlFormatter.js`]: ['format'],
	[`${lib}/services/htmlHighlighting.js`]: ['findDocumentHighlights'],
	[`${lib}/services/htmlSymbolsProvider.js`]: ['findDocumentSymbols', 'findDocumentSymbols2'],
	[`${lib}/services/htmlRename.js`]: ['doRename'],
	[`${lib}/services/htmlMatchingTagPosition.js`]: ['findMatchingTagPosition'],
	[`${lib}/services/htmlLinkedEditing.js`]: ['findLinkedEditingRanges'],
	[scannerUrl]: ['createScanner'],
};

let depth = 0;
const calls = [];
const scannerInputs = new Map();
const parses = [];

const docOf = (d) => ({ uri: d.uri, languageId: d.languageId, text: d.getText() });
const json = (v) => JSON.parse(JSON.stringify(v ?? null));

function providersOf(dm) {
	const providers = dm.getDataProviders();
	const builtIn = providers[0]?.getId() === 'html5';
	const custom = providers.slice(builtIn ? 1 : 0).map((p) => ({
		id: p.getId(),
		data: {
			version: 1,
			tags: p._tags,
			globalAttributes: p._globalAttributes,
			valueSets: Object.entries(p._valueSetMap).map(([name, values]) => ({ name, values })),
		},
	}));
	return { builtIn, custom };
}

function capsOf(options) {
	return options && 'clientCapabilities' in options ? json(options.clientCapabilities) : undefined;
}

function contextLog(ctx, log) {
	if (!ctx) return ctx;
	return {
		resolveReference(ref, base) {
			const result = ctx.resolveReference(ref, base);
			log.push({ ref, base, result: result ?? null });
			return result;
		},
	};
}

const recordArgs = {
	doComplete(document, position, htmlDocument, settings) {
		return { document: docOf(document), position, settings: json(settings) };
	},
	doComplete2(document, position, htmlDocument, documentContext, settings) {
		return { document: docOf(document), position, settings: json(settings) };
	},
	doQuoteComplete(document, position, htmlDocument, settings) {
		return { document: docOf(document), position, settings: json(settings) };
	},
	doTagComplete(document, position) {
		return { document: docOf(document), position };
	},
	doHover(document, position, htmlDocument, options) {
		return { document: docOf(document), position, options: json(options) };
	},
	getFoldingRanges(document, context) {
		return { document: docOf(document), context: json(context) };
	},
	getSelectionRanges(document, positions) {
		return { document: docOf(document), positions };
	},
	findDocumentLinks(document) {
		return { document: docOf(document) };
	},
	format(document, range, options) {
		return { document: docOf(document), range: json(range), options: json(options) };
	},
	findDocumentHighlights(document, position) {
		return { document: docOf(document), position };
	},
	findDocumentSymbols(document) {
		return { document: docOf(document) };
	},
	findDocumentSymbols2(document) {
		return { document: docOf(document) };
	},
	doRename(document, position, newName) {
		return { document: docOf(document), position, newName };
	},
	findMatchingTagPosition(document, position) {
		return { document: docOf(document), position };
	},
	findLinkedEditingRanges(document, position) {
		return { document: docOf(document), position };
	},
};

function wrap(name, fn, extra) {
	return function (...args) {
		const self = this;
		if (name === 'createScanner') {
			if (depth === 0) {
				const [input, initialOffset = 0, initialState = 0, emitPseudoCloseTags = false] = args;
				const e = { input, initialOffset, initialState, emitPseudoCloseTags };
				scannerInputs.set(JSON.stringify(e), e);
			}
			return fn.apply(this, args);
		}
		const top = depth === 0;
		const record = top ? { fn: name, ...recordArgs[name](...args), ...(extra ? extra.call(this) : {}) } : null;
		const log = [];
		if (top && (name === 'doComplete2' || name === 'findDocumentLinks')) {
			const i = name === 'doComplete2' ? 3 : 1;
			args[i] = contextLog(args[i], log);
			record.resolveReference = log;
		}
		if (top && name === 'doComplete2') {
			const fsp = self.lsOptions.fileSystemProvider;
			if (fsp?.readDirectory && !fsp.__wrapped) {
				const readDirectory = fsp.readDirectory.bind(fsp);
				fsp.readDirectory = async (uri) => {
					const log = globalThis.__readDirectoryLog;
					try {
						const result = await readDirectory(uri);
						log?.push({ uri, result });
						return result;
					} catch (e) {
						log?.push({ uri, result: null });
						throw e;
					}
				};
				fsp.__wrapped = true;
			}
			record.readDirectory = globalThis.__readDirectoryLog = [];
		}
		if (top && name === 'doComplete' && this.completionParticipants.length) {
			const participants = this.completionParticipants;
			const events = [];
			this.completionParticipants = participants.map((p) => ({
				onHtmlAttributeValue: p.onHtmlAttributeValue && ((c) => {
					events.push({ attributeValue: { tag: c.tag, attribute: c.attribute, value: c.value, range: c.range, attributes: c.attributes ?? null } });
					return p.onHtmlAttributeValue(c);
				}),
				onHtmlContent: p.onHtmlContent && ((c) => {
					events.push({ content: { position: c.position } });
					return p.onHtmlContent(c);
				}),
			}));
			record.participants = events;
			record.participantHandlers = {
				attributeValue: participants.some((p) => p.onHtmlAttributeValue),
				content: participants.some((p) => p.onHtmlContent),
			};
			try {
				return call();
			} finally {
				this.completionParticipants = participants;
			}
		}
		return call();

		function call() {
			depth++;
			let result;
			try {
				result = fn.apply(self, args);
			} catch (e) {
				if (top) {
					record.throws = String(e);
					calls.push(record);
				}
				throw e;
			} finally {
				depth--;
			}
			if (top) {
				if (result instanceof Promise) {
					calls.push(record);
					return result.then((r) => {
						record.result = json(r);
						return r;
					}, (e) => {
						record.throws = String(e);
						throw e;
					});
				}
				record.result = json(result);
				calls.push(record);
			}
			return result;
		}
	};
}

module.registerHooks({
	load(u, context, next) {
		const names = freeFunctions[u];
		if (names) {
			const source = [
				`export * from ${JSON.stringify(u + '?orig')};`,
				`import * as m from ${JSON.stringify(u + '?orig')};`,
				...names.map((n) => `export const ${n} = globalThis.__wrap(${JSON.stringify(n)}, m.${n});`),
			].join('\n');
			return { format: 'module', source, shortCircuit: true };
		}
		return next(u, context);
	},
});
globalThis.__wrap = (name, fn) => wrap(name, fn);

const { HTMLCompletion } = await import(`${lib}/services/htmlCompletion.js`);
const { HTMLHover } = await import(`${lib}/services/htmlHover.js`);
const { HTMLFolding } = await import(`${lib}/services/htmlFolding.js`);
const { HTMLSelectionRange } = await import(`${lib}/services/htmlSelectionRange.js`);
const { HTMLDocumentLinks } = await import(`${lib}/services/htmlLinks.js`);
const { HTMLParser } = await import(`${lib}/parser/htmlParser.js`);
const { TokenType, ScannerState } = await import(`${lib}/htmlLanguageTypes.js`);
const scannerOrig = await import(scannerUrl + '?orig');

function lsExtra() {
	const dm = this.dataManager ?? this.htmlParser?.dataManager;
	const fsp = this.lsOptions?.fileSystemProvider;
	return { providers: dm ? providersOf(dm) : null, capabilities: capsOf(this.lsOptions), hasFileSystem: !!fsp?.readDirectory };
}

for (const [cls, names] of [
	[HTMLCompletion, ['doComplete', 'doComplete2', 'doQuoteComplete', 'doTagComplete']],
	[HTMLHover, ['doHover']],
	[HTMLFolding, ['getFoldingRanges']],
	[HTMLSelectionRange, ['getSelectionRanges']],
	[HTMLDocumentLinks, ['findDocumentLinks']],
]) {
	for (const n of names) {
		cls.prototype[n] = wrap(n, cls.prototype[n], lsExtra);
	}
}

const nodeJson = (n) => ({
	tag: n.tag ?? null, start: n.start, end: n.end, startTagEnd: n.startTagEnd ?? null, endTagStart: n.endTagStart ?? null,
	closed: n.closed, attributes: n.attributes ? Object.entries(n.attributes) : null, children: n.children.map(nodeJson),
});
const parse = HTMLParser.prototype.parse;
HTMLParser.prototype.parse = function (text, voidElements, options) {
	const doc = parse.call(this, text, voidElements, options);
	if (depth === 0) {
		const before = [];
		const at = [];
		for (let i = 0; i <= text.length; i++) {
			before.push(nodePath(doc.roots, doc.findNodeBefore(i)));
			at.push(nodePath(doc.roots, doc.findNodeAt(i)));
		}
		parses.push({ text, voidElements, orphans: options?.createNodesForOrphanEndTags === true, roots: doc.roots.map(nodeJson), before, at });
	}
	return doc;
};

// The child indexes from the roots to `node`; [] for the document.
function nodePath(roots, node) {
	const p = [];
	while (node.parent) {
		const siblings = node.parent.parent ? node.parent.children : roots;
		p.unshift(siblings.indexOf(node));
		node = node.parent;
	}
	return p;
}

const tokenName = Object.fromEntries(Object.entries(TokenType).filter(([k]) => isNaN(+k)).map(([k, v]) => [v, k]));
const stateName = Object.fromEntries(Object.entries(ScannerState).filter(([k]) => isNaN(+k)).map(([k, v]) => [v, k]));

function tokens(e) {
	const s = scannerOrig.createScanner(e.input, e.initialOffset, e.initialState, e.emitPseudoCloseTags);
	const result = [];
	for (let t = s.scan(); ; t = s.scan()) {
		result.push([tokenName[t], s.getTokenOffset(), s.getTokenLength(), s.getTokenText(), s.getTokenError() ?? null, stateName[s.getScannerState()]]);
		if (t === TokenType.EOS) break;
	}
	return result;
}

const extras = ['<div 😀=x>', '<div😀>', '<a b="😀">c</a>', '</😀>', '<script>a<!--<script></script>-->b</script>x',
	'<script type="text/html"><b></b></script>', '<style>a</STYLE>', '<a b=c/>', '<a b=\'c', '<!DocType html>',
	'<a b>', '<a b　c>', '<div ÄÖ=1 Σ=2>', '<a href=x y>', '<!-- a', '<a <b', '</a <b', '<a/b>',
	'<a\tb\f=\tc\f>', '</a\t>', '<a\f/>', '<a b\t\r\n=c>'];
for (const input of extras) {
	for (const emitPseudoCloseTags of [false, true]) {
		const e = { input, initialOffset: 0, initialState: 0, emitPseudoCloseTags };
		scannerInputs.set(JSON.stringify(e), e);
	}
}

// A lone surrogate (upstream can split a pair into two tokens).
const lone = /[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/;

process.on('exit', () => {
	const scanner = [...scannerInputs.values()].map((e) => {
		const t = tokens(e);
		const splitsSurrogatePair = t.some((x) => lone.test(x[3]));
		return {
			...e,
			initialState: stateName[e.initialState],
			splitsSurrogatePair,
			tokens: splitsSurrogatePair ? [] : t,
		};
	});
	// The fixture is tracked: the repository path becomes `/repo`.
	const repoPath = path.resolve(url.fileURLToPath(import.meta.url), '../../..').replace(/^[A-Za-z]:/, '').replaceAll('\\', '/');
	fs.writeFileSync(out, JSON.stringify({ scanner, parser: parses, calls }).replaceAll(repoPath, '/repo'));
	console.error(`${scanner.length} scanner inputs, ${parses.length} parses, ${calls.length} calls`);
});

const testDir = path.resolve(root, 'lib/esm/test');
for (const f of fs.readdirSync(testDir).filter((f) => f.endsWith('.test.js'))) {
	await import(url.pathToFileURL(path.join(testDir, f)).href);
}
