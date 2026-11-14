// Runs compiled upstream feature tests and records each language service call with its result.
// Usage: node ls-run.mjs <vscode-json-languageservice dir> <out.json> <test file names...>
// Example: node ls-run.mjs upstream/vscode-json-languageservice out.json completion.test.js hover.test.js links.test.js documentSymbols.test.js
import fs from 'fs';
import path from 'path';
import url from 'url';

const [root, out, ...files] = process.argv.slice(2);
const testDir = path.resolve(root, 'lib/esm/test');

// The wrapper replaces `getLanguageService` and records the calls of each instance.
fs.writeFileSync(path.join(testDir, '__ls.js'), `
export * from '../jsonLanguageService.js';
import { getLanguageService as original } from '../jsonLanguageService.js';

export const tests = [];
let current = null;
export function startTest(name) { current = { name, ops: [] }; tests.push(current); }

const dump = v => v === undefined ? null : JSON.parse(JSON.stringify(v));
let nextId = 0;

export function getLanguageService(params = {}) {
	const ops = current.ops;
	const svc = nextId++;
	ops.push({ op: 'new', svc, clientCapabilities: dump(params.clientCapabilities), contributions: !!params.contributions?.length, requestService: !!params.schemaRequestService });
	const request = params.schemaRequestService && (uri => Promise.resolve(params.schemaRequestService(uri)).then(
		content => { ops.push({ op: 'request', svc, uri, content }); return content; },
		error => { ops.push({ op: 'request', svc, uri, error: { message: String(error?.message ?? error) } }); throw error; }));
	const ls = original({ ...params, schemaRequestService: request });
	const doc = d => ({ uri: d.uri, text: d.getText() });
	const rec = (op, fn) => async (document, ...args) => {
		const entry = { op, svc, doc: doc(document), args: dump(args.filter(a => typeof a !== 'object' || !a || !('root' in a))) };
		ops.push(entry);
		const result = await fn(document, ...args);
		entry.result = dump(result);
		return result;
	};
	return {
		...ls,
		configure(settings) { ops.push({ op: 'configure', svc, settings: dump(settings) }); return ls.configure(structuredClone(settings)); },
		doComplete: rec('complete', (d, p, j) => ls.doComplete(d, p, j)),
		doHover: rec('hover', (d, p, j) => ls.doHover(d, p, j)),
		findLinks: rec('links', (d, j) => ls.findLinks(d, j)),
		findDocumentColors: rec('colors', (d, j, c) => ls.findDocumentColors(d, j, c)),
	};
}
`);
fs.writeFileSync(path.join(testDir, '__shim.js'), `
import { startTest } from './__ls.js';
const names = [];
export const queue = [];
export function suite(name, fn) { names.push(name); fn(); names.pop(); }
export function test(name, fn) { queue.push({ name: [...names, name].join(' / '), fn }); }
test.skip = () => {};
test.only = test;
suite.skip = () => {};
export function after() {}
export function before() {}
export async function runQueue() {
	for (const t of queue) {
		startTest(t.name);
		try { await t.fn.call({ timeout() {} }); } catch (e) { console.error('upstream test failed: ' + t.name + ': ' + e); }
	}
	queue.length = 0;
}
`);

const { tests } = await import(url.pathToFileURL(path.join(testDir, '__ls.js')).href);
const shim = await import(url.pathToFileURL(path.join(testDir, '__shim.js')).href);
for (const file of files) {
	const src = fs.readFileSync(path.join(testDir, file), 'utf8')
		.replace(/from 'node:test'/g, "from './__shim.js'")
		.replace(/from ['"]\.\.\/jsonLanguageService\.js['"]/g, "from './__ls.js'");
	const copy = path.join(testDir, '__' + file);
	fs.writeFileSync(copy, src);
	const before = shim.queue.length;
	await import(url.pathToFileURL(copy).href);
	for (let i = before; i < shim.queue.length; i++) {
		shim.queue[i].name = file.replace('.test.js', '') + ' / ' + shim.queue[i].name;
	}
	await shim.runQueue();
}
const kept = tests.filter(t => t.ops.some(o => ['complete', 'hover', 'links', 'colors'].includes(o.op)));
fs.writeFileSync(out, JSON.stringify(kept, null, 1) + '\n');
console.log(`${kept.length} tests, ${kept.reduce((n, t) => n + t.ops.length, 0)} ops`);
