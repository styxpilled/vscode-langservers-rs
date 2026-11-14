// Runs completion and hover at each offset of the documents in a recorded feature trace and of
// extra samples, and writes the results in the format of ls-run.mjs.
// Usage: node ls-sweep.mjs <vscode-json-languageservice dir> <upstream_features.json> <out.json>
import fs from 'fs';
import path from 'path';
import url from 'url';

const [root, traceFile, out] = process.argv.slice(2);
const ls = await import(url.pathToFileURL(path.resolve(root, 'lib/esm/jsonLanguageService.js')).href);
const { getLanguageService, TextDocument, ClientCapabilities } = ls;

const here = path.dirname(url.fileURLToPath(import.meta.url));
const extra = JSON.parse(fs.readFileSync(path.join(here, 'sweep-samples.json'), 'utf8'));

const cases = [];
for (const test of JSON.parse(fs.readFileSync(traceFile, 'utf8'))) {
	if (test.ops.some(o => o.contributions)) {
		continue;
	}
	let settings = null;
	for (const op of test.ops) {
		if (op.op === 'configure') {
			settings = op.settings;
		} else if (op.op === 'new') {
			settings = null;
		} else if (op.op === 'complete' || op.op === 'hover') {
			const caps = test.ops.find(o => o.op === 'new' && o.svc === op.svc).clientCapabilities;
			const requests = test.ops.filter(o => o.op === 'request' && o.svc === op.svc);
			cases.push({ name: test.name, caps, settings, doc: op.doc, requests });
		}
	}
}
for (const sample of extra.samples) {
	for (const caps of [ClientCapabilities.LATEST, null]) {
		cases.push({ name: 'sample', caps, settings: { schemas: [{ uri: 'http://x/schema.json', fileMatch: ['*.json'], schema: extra.schemas[sample.schema] }] }, doc: { uri: 'test://test/test.json', text: sample.text }, requests: [] });
	}
}

const seen = new Set();
const tests = [];
for (const c of cases) {
	const key = JSON.stringify([c.caps, c.settings, c.doc]);
	if (seen.has(key)) {
		continue;
	}
	seen.add(key);
	const svc = tests.length;
	const ops = [{ op: 'new', svc, clientCapabilities: c.caps, contributions: false, requestService: c.requests.length > 0 }, ...c.requests];
	ops.forEach(o => o.svc = svc);
	if (c.settings) {
		ops.push({ op: 'configure', svc, settings: c.settings });
	}
	const responses = new Map(c.requests.map(r => [r.uri, r]));
	const service = getLanguageService({
		clientCapabilities: c.caps ?? {},
		schemaRequestService: c.requests.length ? (uri => responses.get(uri)?.content !== undefined ? Promise.resolve(responses.get(uri).content) : Promise.reject('Resource not found')) : undefined,
	});
	if (c.settings) {
		service.configure(structuredClone(c.settings));
	}
	const document = TextDocument.create(c.doc.uri, 'json', 0, c.doc.text);
	const jsonDoc = service.parseJSONDocument(document);
	for (let offset = 0; offset <= c.doc.text.length; offset++) {
		const position = document.positionAt(offset);
		const prev = c.doc.text.charCodeAt(offset - 1);
		if (prev >= 0xd800 && prev <= 0xdbff) {
			continue;
		}
		for (const [op, fn] of [['complete', service.doComplete], ['hover', service.doHover]]) {
			let result;
			try {
				result = await fn(document, position, jsonDoc);
			} catch (e) {
				continue;
			}
			ops.push({ op, svc, doc: c.doc, args: [position], result: result === undefined ? null : JSON.parse(JSON.stringify(result)) });
		}
	}
	tests.push({ name: c.name, ops });
}
fs.writeFileSync(out, JSON.stringify(tests) + '\n');
console.log(`${tests.length} documents, ${tests.reduce((n, t) => n + t.ops.length, 0)} ops`);
