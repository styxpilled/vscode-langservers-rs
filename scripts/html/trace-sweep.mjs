// Runs each HTML feature at each offset of the sweep texts with upstream.
// Usage: node trace-sweep.mjs <vscode-html-languageservice dir> <upstream fixture.json> <vscode dir> <sweep.json>
//
// For each text: `offsets`, at each UTF-16 offset that is not inside a
// surrogate pair, the cyrb53 hash of the canonical JSON (sorted keys) of each
// feature result, in the order of `features`; "error" when upstream throws.
// Each 7th offset also `plain`: completion and hover without markdown.
// `document`: the full results of the document features.
//
// Completion is `doComplete2` with a virtual file system and a participant
// that records its contexts. `resolveReference(ref, base)` is
// `base + "|" + ref`, `undefined` when `ref` has a length of 4 mod 5.
import fs from 'fs';
import path from 'path';
import url from 'url';
import { formatTexts, sweepTexts } from './texts.mjs';

const [root, fixture, vscode, out] = process.argv.slice(2);
const lib = url.pathToFileURL(path.resolve(root, 'lib/esm')).href;
const { getLanguageService, TextDocument } = await import(`${lib}/htmlLanguageService.js`);

const sweepExtra = [
	'<a href="foo/bar.html">x</a><img src=\'b/\'><link href=c.css><script src="/d.js"></script>',
	'<a href="https://x.com/a b">a</a><a href="#frag">b</a><a href="mailto:a@b">c</a><a href="">d</a><a href=" e ">e</a>',
	'<base href="https://x.com/dir/"><a href="a.html">x</a><img srcset="a.png 1x, b.png 2x">',
	'<video src="v."><source src="s"><track src=""></video><audio src="a/b/"></audio><iframe src="./"></iframe>',
	'<input type="file" accept="image/*"><form action="f/"><object data="o"></object><embed src="e">',
	'<a href="..">.</a><a href=".">..</a><a href="a/../">x</a><a href="😀/é">y</a>',
	'<p>&amp; &lt; &gt &copy; &#123; &#x1F600; &unknown; &</p><p title="&quot;">&nb</p>',
	'<div class="a" id="b" style="color:red" onclick="f()" aria-label="x" role="button" data-x="y" hidden></div>',
	'<input type="checkbox" checked disabled><select multiple><option selected value="v">o</option></select>',
	'<button type="submit" formmethod=""></button><a target=""></a><meta name="viewport" content="">',
	'<my-element my-attr="v"></my-element><x-y></x-y><svg viewBox="0 0 1 1"><path d=""/></svg>',
	'<html>\n<head>\n<title>é😀</title>\n</head>\n<body>\n<h1 id="h">Ü</h1>\n<h2>😀</h2>\n</body>\n</html>',
	'<div>\n  <ul>\n    <li>a</li>\n    <li>b\n  </ul>\n  <!-- c\n  d -->\n  <p>\n</div>\n<script>\nx\n</script>\n<style>\ny\n</style>',
	'<!-- #region a -->\n<div>\n<!-- #endregion -->\n</div>\n<!-- #region -->\n<p>\n</p>\n<!--#endregion-->',
	'<div><div><div></div></div></div><DIV></div><div></DIV>',
	'<div a=b c="d" e=\'f\' g h=></div><div a="b c="d"></div>',
	'<img><img/><br></br><input></input><p></p></p>',
	'<a\n  href="x"\n  class="y"\n>z</a\n>',
	'<html lang=""><body dir="" ><p translate="" contenteditable=""></p></body></html>',
	'<div>😀<span>é</span>😀</div><😀></😀><é a=é>é</é>',
	'<div\tclass\t=\t"a"\f>\r\n</div\r\n>',
	'<template><slot name="a"></slot></template><dialog open></dialog><details><summary>s</summary></details>',
];

const features = ['complete', 'hover', 'quote', 'tag', 'highlights', 'rename', 'matching', 'linked', 'selection'];

const resolveReference = (ref, base) => (ref.length % 5 === 4 ? undefined : base + '|' + ref);
const documentContext = { resolveReference };
const fileSystemProvider = {
	stat: async () => ({ type: 0, ctime: 0, mtime: 0, size: 0 }),
	readDirectory: async (uri) => (uri.length % 3 === 2 ? [] : [
		['a.html', 1], ['b', 2], ['c.css', 1], ['.hidden', 1], ['img.png', 1], ['icon.SVG', 1], ['d.js', 1],
		['v.mp4', 1], ['s.mp3', 1], ['t.vtt', 1], ['é 😀', 2], ['link', 64],
	]),
};

const contexts = [];
const recorder = {
	onHtmlAttributeValue: (c) => contexts.push({ attributeValue: { tag: c.tag, attribute: c.attribute, value: c.value, range: c.range, attributes: c.attributes ?? null } }),
	onHtmlContent: (c) => contexts.push({ content: { position: c.position } }),
};
const markdown = getLanguageService({ fileSystemProvider });
const plainText = getLanguageService({ fileSystemProvider, clientCapabilities: {} });
markdown.setCompletionParticipants([recorder]);
plainText.setCompletionParticipants([recorder]);

function canonical(v) {
	if (Array.isArray(v)) return '[' + v.map(canonical).join(',') + ']';
	if (v && typeof v === 'object') {
		return '{' + Object.keys(v).filter((k) => v[k] !== undefined).sort().map((k) => JSON.stringify(k) + ':' + canonical(v[k])).join(',') + '}';
	}
	return JSON.stringify(v ?? null);
}

// cyrb53 over UTF-16 code units.
function cyrb53(str) {
	let h1 = 0xdeadbeef, h2 = 0x41c6ce57;
	for (let i = 0; i < str.length; i++) {
		const ch = str.charCodeAt(i);
		h1 = Math.imul(h1 ^ ch, 2654435761);
		h2 = Math.imul(h2 ^ ch, 1597334677);
	}
	h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507);
	h1 ^= Math.imul(h2 ^ (h2 >>> 13), 3266489909);
	h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507);
	h2 ^= Math.imul(h1 ^ (h1 >>> 13), 3266489909);
	return 4294967296 * (2097151 & h2) + (h1 >>> 0);
}

async function hash(f) {
	try {
		return cyrb53(canonical(JSON.parse(JSON.stringify((await f()) ?? null))));
	} catch {
		return 'error';
	}
}

async function complete(ls, doc, position, html) {
	contexts.length = 0;
	const result = await ls.doComplete2(doc, position, html, documentContext);
	return { result, contexts: [...contexts] };
}

const isLow = (s, i) => /[\uDC00-\uDFFF]/.test(s[i] ?? '') && /[\uD800-\uDBFF]/.test(s[i - 1] ?? '');

const texts = sweepTexts(fixture, vscode, [...formatTexts, ...sweepExtra]);
const results = [];
let total = 0, errors = 0;
for (const text of texts) {
	const doc = TextDocument.create('test://test/dir/test.html', 'html', 1, text);
	const html = markdown.parseHTMLDocument(doc);
	const offsets = [];
	const plain = [];
	for (let o = 0; o <= text.length; o++) {
		if (isLow(text, o)) continue;
		const p = doc.positionAt(o);
		const h = [
			await hash(() => complete(markdown, doc, p, html)),
			await hash(() => markdown.doHover(doc, p, html)),
			await hash(() => markdown.doQuoteComplete(doc, p, html)),
			await hash(() => markdown.doTagComplete(doc, p, html)),
			await hash(() => markdown.findDocumentHighlights(doc, p, html)),
			await hash(() => markdown.doRename(doc, p, 'newName', html)),
			await hash(() => markdown.findMatchingTagPosition(doc, p, html)),
			await hash(() => markdown.findLinkedEditingRanges(doc, p, html)),
			await hash(() => markdown.getSelectionRanges(doc, [p])),
		];
		offsets.push(h);
		if (o % 7 === 0) {
			plain.push([
				await hash(() => complete(plainText, doc, p, html)),
				await hash(() => plainText.doHover(doc, p, html, { references: false })),
			]);
		}
		total += h.length;
		errors += h.filter((x) => x === 'error').length;
	}
	const full = (f) => {
		try {
			return JSON.parse(JSON.stringify(f() ?? null));
		} catch (e) {
			return { error: String(e) };
		}
	};
	results.push({
		text, offsets, plain,
		document: {
			folding: full(() => markdown.getFoldingRanges(doc)),
			folding3: full(() => markdown.getFoldingRanges(doc, { rangeLimit: 3 })),
			symbols: full(() => markdown.findDocumentSymbols(doc, html)),
			symbols2: full(() => markdown.findDocumentSymbols2(doc, html)),
			links: full(() => markdown.findDocumentLinks(doc, documentContext)),
		},
	});
}
fs.writeFileSync(out, JSON.stringify({ features, results }));
console.error(`${texts.length} texts, ${total} results, ${errors} errors`);
