// Records `vscode-uri` `Utils.joinPath`, `dirname`, `basename`,
// `resolvePath`, `URI.file` and `toString(true)` on generated inputs (the
// bundled `path.posix` too). `URI.file` results depend on the platform.
// Usage: node trace-uri.mjs <vscode-css-languageservice dir> <fixture.json>
import fs from 'fs';
import path from 'path';
import url from 'url';

const [root, out] = process.argv.slice(2);
const { URI, Utils } = await import(url.pathToFileURL(path.resolve(root, 'node_modules/vscode-uri/lib/esm/index.mjs')).href);

const attempt = (f) => {
	try {
		return f();
	} catch {
		return null;
	}
};

const segments = ['', '.', '..', 'a', 'b/', '/c', './d', '../e', 'f/../g', '//h', 'i/./j/', '...', '.k', 'l..', '../../m', 'n/..', '/'];
const paths = [];
for (const a of segments) for (const b of segments) paths.push(a + b);

const uris = [
	'file:///c:/Users/a/b.css', 'file:///C:/x/y/', 'file:///', 'file://server/share/x.scss', 'test://test', 'test://test/',
	'test://test/a/b.scss', 'untitled:Untitled-1', 'zed://schemas/x', 'http://h:8080/p/q.css?x=1#f', 'file:///a%20b/c%23d.css',
	'file:///ä/ö.css', 'a/b', '', 'foo:bar/baz', 'foo:/x', 'x y:z', 'file:///a/b/../c/./d.css', 'C:/a/b.css', 'file:///a?b#c',
];
const joins = ['', 'node_modules', 'package.json', '../x', '/abs', 'a b', 'ä#?.css', '..', '.', './_index.scss', '../../..'];

const results = { join: [], normalize: [], dirname: [], basename: [], uris: [] };
const { posix } = await import('path');
for (const p of paths) {
	results.normalize.push([p, posix.normalize(p)]);
	results.dirname.push([p, posix.dirname(p)]);
	results.basename.push([p, posix.basename(p)]);
}
for (let i = 0; i < paths.length; i += 3) results.join.push([[paths[i], paths[(i * 7) % paths.length]], posix.join(paths[i], paths[(i * 7) % paths.length])]);
for (const u of uris) {
	const parsed = attempt(() => URI.parse(u));
	const r = { uri: u, string: parsed && parsed.toString(true) };
	if (parsed) {
		r.dirname = attempt(() => Utils.dirname(parsed).toString(true));
		r.basename = Utils.basename(parsed);
		r.join = joins.map((j) => attempt(() => Utils.joinPath(parsed, j).toString(true)));
		r.join2 = attempt(() => Utils.joinPath(parsed, 'node_modules', '@a/b', 'package.json').toString(true));
		r.resolve = joins.concat(paths.filter((_, i) => i % 5 === 0)).map((j) => [j, attempt(() => Utils.resolvePath(parsed, j).toString(true))]);
	}
	results.uris.push(r);
}
const files = ['C:\\Users\\a b\\x.css', 'c:/x', '/usr/a', '\\\\server\\share\\x', '//server', '//server/', 'rel/x', '', '/a#b?c', 'C:\\ä%20'];
results.files = files.map((f) => [f, URI.file(f).toString(true), URI.file(f).toString()]);
fs.writeFileSync(out, JSON.stringify(results, null, 1));
