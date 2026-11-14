// Records the `findDocumentLinks2` and `PathCompletionParticipant.computeCompletions`
// calls of the upstream tests, with each file system and `resolveReference`
// call that they make. With a sweep argument, also runs links (sync and
// async) and path completion on generated texts against a virtual file system.
// Usage: node trace-links.mjs <vscode-css-languageservice dir> <fixture.json>
//        node trace-links.mjs <vscode-css-languageservice dir> - <sweep.json>
// The absolute path of the upstream folder becomes `/ws` in the output.
import { AsyncLocalStorage } from 'async_hooks';
import fs from 'fs';
import module from 'module';
import path from 'path';
import url from 'url';

const [root, out, sweepOut] = process.argv.slice(2);
const lib = url.pathToFileURL(path.resolve(root, 'lib/esm')).href;
const fsProviderUrl = `${lib}/test/testUtil/fsProvider.js`;
const contextUrl = `${lib}/test/testUtil/documentContext.js`;

const als = new AsyncLocalStorage();
const record = (entry) => als.getStore()?.push(entry);
const copy = (v) => (v === undefined ? null : JSON.parse(JSON.stringify(v)));

function recordingFs(provider) {
	const wrap = (fn, name) => async (uri) => {
		try {
			const result = await fn(uri);
			record({ fn: name, uri, result: copy(name === 'stat' ? { type: result.type, size: result.size } : result) });
			return result;
		} catch (e) {
			record({ fn: name, uri, thrown: true });
			throw e;
		}
	};
	return { stat: wrap(provider.stat, 'stat'), readDirectory: wrap(provider.readDirectory, 'readDirectory'), getContent: wrap(provider.getContent, 'getContent') };
}
function recordingContext(context) {
	return {
		resolveReference(ref, base) {
			const result = context.resolveReference(ref, base);
			record({ fn: 'resolveReference', ref, base, result: result ?? null });
			return result;
		},
	};
}
globalThis.__wrapFs = (f) => (...a) => recordingFs(f(...a));
globalThis.__wrapContext = (f) => (...a) => recordingContext(f(...a));

module.registerHooks({
	load(u, context, next) {
		const hooked = { [fsProviderUrl]: ['getFsProvider', '__wrapFs'], [contextUrl]: ['getDocumentContext', '__wrapContext'] }[u];
		if (hooked) {
			const [name, wrapper] = hooked;
			const source = [
				`export * from ${JSON.stringify(u + '?orig')};`,
				`import * as m from ${JSON.stringify(u + '?orig')};`,
				`export const ${name} = globalThis.${wrapper}(m.${name});`,
			].join('\n');
			return { format: 'module', source, shortCircuit: true };
		}
		return next(u, context);
	},
});

const { CSSNavigation } = await import(`${lib}/services/cssNavigation.js`);
const { SCSSNavigation } = await import(`${lib}/services/scssNavigation.js`);
const { PathCompletionParticipant } = await import(`${lib}/services/pathCompletion.js`);
const ls = await import(`${lib}/cssLanguageService.js`);
const { getDocumentContext } = await import(contextUrl);

const calls = [];
const docOf = (d) => ({ uri: d.uri, languageId: d.languageId, version: d.version, text: d.getText() });
const nav = CSSNavigation.prototype;
for (const name of ['findDocumentLinks', 'findDocumentLinks2']) {
	const orig = nav[name];
	nav[name] = function (document, stylesheet, context) {
		const entry = { fn: name, scss: this instanceof SCSSNavigation, hasFs: !!this.fileSystemProvider, aliases: copy(this.defaultSettings), document: docOf(document), calls: [] };
		calls.push(entry);
		const done = (result) => {
			entry.result = copy(result);
			return result;
		};
		return als.run(entry.calls, () => {
			const r = orig.call(this, document, stylesheet, context);
			if (!(r instanceof Promise)) return done(r);
			return r.then(done, (e) => {
				entry.result = null;
				entry.rejected = String(e);
				throw e;
			});
		});
	};
}
const pc = PathCompletionParticipant.prototype;
const computeCompletions = pc.computeCompletions;
pc.computeCompletions = function (document, context) {
	const entry = { fn: 'computeCompletions', document: docOf(document), literal: copy(this.literalCompletions), import: copy(this.importCompletions), calls: [] };
	calls.push(entry);
	return als.run(entry.calls, async () => {
		try {
			const result = await computeCompletions.call(this, document, context);
			entry.result = copy(result);
			return result;
		} catch (e) {
			entry.result = null;
			entry.rejected = String(e);
			throw e;
		}
	});
};

const testDir = path.resolve(root, 'lib/esm/test');
for (const dir of sweepOut ? [] : ['css', 'scss', 'less']) {
	for (const f of fs.readdirSync(path.join(testDir, dir)).filter((f) => f.endsWith('.test.js'))) {
		await import(url.pathToFileURL(path.join(testDir, dir, f)).href);
	}
}

const absRoot = url.pathToFileURL(path.resolve(root)).href.replace(/^file:\/\/\//, '').replace(/^([A-Z]):/, (m, d) => d.toLowerCase() + ':');
const anonymize = (s) => s.split(absRoot).join('c:/ws');

process.on('exit', () => {
	if (sweepOut) return;
	const recorded = calls.splice(0);
	const text = anonymize(JSON.stringify(recorded));
	if (text.includes(path.basename(path.resolve(root, '..', '..')))) throw new Error('absolute path left in fixture');
	fs.writeFileSync(out, text);
	console.error(`${recorded.length} calls`);
});

if (!sweepOut) {
	await new Promise(() => {});
}

// The sweep.
const files = {
	'a.css': '', 'green/c.css': '', 'green/d.scss': '', 'green/_index.scss': '', 'blue/index.scss': '', 'x/_y.scss': '', 'x/y.css': '',
	'a b/c.css': '', 'ä/ö.scss': '', 'src/assets/styles.css': '', 'src/assets/styles.scss': '', 'about/about.css': '', 'about/.hidden': '',
	'about/about.html': '', 'about/(x),y.png': '', 'scss/_foo.scss': '', 'scss/main.scss': '', 'scss/bar.scss': '',
	'node_modules/@foo/bar/package.json': '{}', 'node_modules/@foo/bar/_baz.scss': '', 'node_modules/@foo/bar/_index.scss': '',
	'node_modules/foo/package.json': '{}', 'node_modules/foo/bar.css': '', 'node_modules/green/package.json': '{}', 'node_modules/green/_e.scss': '',
	'node_modules/green/c.css': '',
	'node_modules/bar/package.json': JSON.stringify({ exports: { '.': { sass: './styles/index.scss' }, './colors.scss': { sass: './styles/colors.scss' }, './button': { sass: './styles/button.scss' }, './js': { default: './x.js' } } }),
	'node_modules/bar-pattern/package.json': JSON.stringify({ exports: { '.': { sass: './styles/index.scss' }, './*.scss': { sass: './styles/*.scss' }, './theme/*': { sass: './styles/theme/*.scss' }, './a.b+(c)/*': { styles: './s/*/*.scss' }, './n/*': 5, './o/*': { sass: 7 } } }),
	'node_modules/conditional/package.json': JSON.stringify({ exports: { sass: './_index.scss', default: './index.js' } }),
	'node_modules/root-sass/package.json': JSON.stringify({ sass: './styles/index.scss' }),
	'node_modules/root-style/package.json': JSON.stringify({ style: './styles/index.scss', sass: '' }),
	'node_modules/root-num/package.json': JSON.stringify({ sass: 5 }),
	'node_modules/null-pkg/package.json': 'null',
	'node_modules/num-pkg/package.json': '5',
	'node_modules/bad-pkg/package.json': '{ bad',
	'node_modules/empty-pkg/package.json': '',
	'node_modules/str-exports/package.json': JSON.stringify({ exports: './index.scss' }),
	'node_modules/arr-exports/package.json': JSON.stringify({ exports: ['./a.scss'] }),
	'node_modules/obj-entry/package.json': JSON.stringify({ exports: { '.': { sass: { x: 1 } }, './y': { sass: ['a'] }, './z': 'str' } }),
	'node_modules/default-js/package.json': JSON.stringify({ exports: { '.': { default: './index.js' } } }),
	'sub/node_modules/near/package.json': '{}', 'sub/node_modules/near/x.scss': '',
	'sub/deep/doc.scss': '', 'sub/doc.css': '',
};
const vroot = 'file:///c:/ws/';
const entries = new Map();
for (const [p, content] of Object.entries(files)) {
	entries.set(vroot + p, { type: 1, content });
	const parts = p.split('/');
	for (let i = 0; i < parts.length; i++) entries.set(vroot + parts.slice(0, i).join('/') + (i ? '' : ''), { type: 2 });
}
entries.set('file:///c:/ws', { type: 2 });
const norm = (u) => u.replace(/\/$/, '').replace(/%20/g, ' ');
const vfs = recordingFs({
	async stat(u) {
		if (!u.startsWith('file:')) throw new Error('Protocol not supported');
		const e = entries.get(norm(u));
		return e ? { type: e.type, size: e.type === 1 ? e.content.length : 0 } : { type: 0, size: -1 };
	},
	async readDirectory(u) {
		if (!u.startsWith('file:')) throw new Error('Protocol not supported');
		const dir = norm(u);
		if (entries.get(dir)?.type !== 2) throw new Error('ENOENT');
		const names = new Map();
		for (const [k, e] of entries) {
			if (k.startsWith(dir + '/') && k.length > dir.length + 1 && !k.slice(dir.length + 1).includes('/')) names.set(k.slice(dir.length + 1), e.type);
		}
		return [...names];
	},
	async getContent(u) {
		const e = entries.get(norm(u));
		if (!e || e.type !== 1) throw new Error('ENOENT');
		return e.content;
	},
});

const targets = [
	'foo.css', './foo.css', '../foo.css', '~foo/hello.html', '~green/c.css', '~green', '~', '~/x.css', 'green/c.css', 'green/d', 'green/e', 'green/',
	'green', 'blue/', 'blue', 'x/y', 'x/_y', '_y', 'x/y.scss', '@foo/bar/baz', '@foo/bar', '@foo', 'foo/bar.css', 'foo', 'near/x', '../near/x',
	'pkg:bar', 'pkg:bar/colors', 'pkg:bar/colors.scss', 'pkg:bar/button', 'pkg:bar/js', 'pkg:bar/none', 'pkg:@foo/bar', 'pkg:root-sass', 'pkg:root-style',
	'pkg:root-style/x', 'pkg:root-num', 'pkg:bar-pattern', 'pkg:bar-pattern/anything', 'pkg:bar-pattern/a.scss/b.scss', 'pkg:bar-pattern/theme/dark.scss',
	'pkg:bar-pattern/a.b+(c)/q', 'pkg:bar-pattern/n/1', 'pkg:bar-pattern/o/1', 'pkg:conditional', 'pkg:null-pkg', 'pkg:num-pkg', 'pkg:bad-pkg',
	'pkg:empty-pkg', 'pkg:str-exports', 'pkg:str-exports/x', 'pkg:arr-exports', 'pkg:arr-exports/0', 'pkg:obj-entry', 'pkg:obj-entry/y', 'pkg:obj-entry/z',
	'pkg:default-js', 'pkg:missing', 'pkg:', 'sass:math', 'data:x', 'http://x/y.css', 'chrome://downloads', 'gantry-media://g.png', 'a b/c.css',
	'a%20b/c.css', 'ä/ö', '@SingleStylesheet', '@AssetsDir/styles.css', '@AssetsDir/', '@Missing/x', '/src/assets/styles.css', '', ' ', '#frag', '?q',
	'C:/x.css', 'file:///c:/ws/a.css', 'x:y', '://x', 'a://x', 'a:b/c', '.', '..', '../..', '../../../../../x',
];
const forms = [
	(t) => `@import '${t}';`, (t) => `@import url(${t});`, (t) => `@import url("${t}") print;`, (t) => `@use '${t}';`,
	(t) => `@forward "${t}";`, (t) => `a { b: url("${t}") }`, (t) => `a { b: url('${t}') }`,
	// A second link: a rejection gives no links at all.
	(t) => `@use '${t}'; @import url(a.css);`,
];
const docs = ['file:///c:/ws/doc', 'file:///c:/ws/sub/deep/doc', 'file:///c:/ws/about/about', 'test://test/test', 'file:///c:/other/doc'];
const aliasSets = [undefined, { '@SingleStylesheet': '/src/assets/styles.css', '@AssetsDir/': '/src/assets/', 'green/': 'blue/', '/': 'x/' }];
const services = {
	css: ls.getCSSLanguageService({ fileSystemProvider: vfs }),
	scss: ls.getSCSSLanguageService({ fileSystemProvider: vfs }),
	less: ls.getLESSLanguageService({ fileSystemProvider: vfs }),
};
const noFs = ls.getCSSLanguageService();
const contexts = [getDocumentContext('file:///c:/ws/'), getDocumentContext()];
for (const [lang, service] of Object.entries(services)) {
	for (const [a, aliases] of aliasSets.entries()) {
		service.configure({ importAliases: aliases });
		for (const [d, base] of docs.entries()) {
			const context = contexts[d % 2];
			for (const t of targets) {
				for (const [fi, form] of forms.entries()) {
					if (a === 1 && (fi % 3 !== 0 || d > 2)) continue;
					const doc = ls.TextDocument.create(`${base}.${lang}`, lang, 1, form(t));
					const sheet = service.parseStylesheet(doc);
					try {
						await service.findDocumentLinks2(doc, sheet, context);
					} catch {}
					if (fi === 0 && a === 0) service.findDocumentLinks(doc, sheet, context);
				}
			}
		}
	}
}
for (const t of ['a.css', '~foo/x', 'foo.css']) {
	const doc = ls.TextDocument.create('file:///c:/ws/doc.css', 'css', 1, `@import '${t}';`);
	await noFs.findDocumentLinks2(doc, noFs.parseStylesheet(doc), contexts[0]);
}

// Path completion at each offset.
const completionTexts = [
	`@import '|';`, `@import "./|";`, `@import 'scss/|';`, `@import "../|`, `@import url(|);`, `@use 'scss/_f|oo.scss';`, `@forward "about/a|bout.css" as x;`,
	`a { b: url(about/|) }`, `a { b: url('./about/ab|out.css') }`, `a { b: url("../ws/about/") }`, `a { b: url("." ) c: url('..') }`, `a { b: url(a b/) }`,
	`a { b: url("é/😀/x y") }`, `@import 'src/assets/ '`, `a { b: url(\n'about/') }`,
];
for (const [lang, service] of Object.entries(services)) {
	for (const base of ['file:///c:/ws/doc', 'file:///c:/ws/about/about', 'file:///c:/ws/scss/main']) {
		for (const raw of completionTexts) {
			const text = raw.replace('|', '');
			const doc = ls.TextDocument.create(`${base}.${lang}`, lang, 1, text);
			const sheet = service.parseStylesheet(doc);
			for (let offset = 0; offset <= text.length; offset++) {
				try {
					await service.doComplete2(doc, doc.positionAt(offset), sheet, contexts[0]);
				} catch {}
			}
		}
	}
}
const sweep = calls.filter((c) => c.fn !== 'computeCompletions' || c.literal.length || c.import.length);
fs.writeFileSync(sweepOut, JSON.stringify(sweep));
console.error(`${sweep.length} sweep calls`);
