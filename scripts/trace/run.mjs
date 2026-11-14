// Runs compiled upstream test files and writes a trace of the schema calls.
// Usage: node run.mjs <vscode-json-languageservice dir> <out.json> <test file names...>
// Example: node run.mjs upstream/vscode-json-languageservice trace.json parser.test.js schema.test.js
import fs from 'fs';
import path from 'path';
import url from 'url';

const [root, out, ...files] = process.argv.slice(2);
const lib = path.resolve(root, 'lib/esm');
const here = path.dirname(url.fileURLToPath(import.meta.url));
const libUrl = url.pathToFileURL(lib).href;

// The recorder and the node:test shim live next to the compiled tests, so that
// relative imports and `__dirname` stay correct.
const recorderSrc = fs.readFileSync(path.join(here, 'recorder.mjs'), 'utf8').replaceAll("'LIB/", `'${libUrl}/`);
fs.writeFileSync(path.join(lib, 'test', '__recorder.js'), recorderSrc);
fs.writeFileSync(path.join(lib, 'test', '__shim.js'), `
import { startTest } from './__recorder.js';
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
		// Some tests do not await all their calls. Let them finish inside this test.
		await new Promise(r => setTimeout(r, 50));
	}
	queue.length = 0;
}
`);

const { tests } = await import(url.pathToFileURL(path.join(lib, 'test', '__recorder.js')).href);
const shim = await import(url.pathToFileURL(path.join(lib, 'test', '__shim.js')).href);
for (const file of files) {
	const src = fs.readFileSync(path.join(lib, 'test', file), 'utf8').replace(/from 'node:test'/g, "from './__shim.js'");
	const copy = path.join(lib, 'test', '__' + file);
	fs.writeFileSync(copy, src);
	const before = shim.queue.length;
	await import(url.pathToFileURL(copy).href);
	for (let i = before; i < shim.queue.length; i++) {
		shim.queue[i].name = file.replace('.test.js', '') + ' / ' + shim.queue[i].name;
	}
	await shim.runQueue();
}
const kept = tests.filter(t => t.ops.some(o => o.op !== 'request' && o.op !== 'resolvePath'));
// The fixture is tracked: the repository path becomes `/repo`.
const repoPath = path.resolve(here, '../..').replace(/^[A-Za-z]:/, '').replaceAll('\\', '/');
fs.writeFileSync(out, JSON.stringify(kept).replaceAll(repoPath, '/repo') + '\n');
console.log(`${kept.length} tests, ${kept.reduce((n, t) => n + t.ops.length, 0)} ops`);
