// Formats the fixtures of the VS Code HTML server `formatting.test.ts` with the
// Node reference of node-main-server.cjs (JavaScript mode off) and writes the
// results. The upstream expected files format the script content.
// Usage: node record-fixture-format.cjs <upstream/vscode> <out dir>
const Module = require('module');
const fs = require('fs');
const path = require('path');

const [vscode, out] = process.argv.slice(2);
const root = path.resolve(__dirname, '../../upstream');
const services = {
	'vscode-html-languageservice': path.join(root, 'vscode-html-languageservice/lib/esm/htmlLanguageService.js'),
	'vscode-css-languageservice': path.join(root, 'vscode-css-languageservice/lib/esm/cssLanguageService.js'),
};
const resolve = Module._resolveFilename;
Module._resolveFilename = function (request, ...rest) {
	return services[request] ?? resolve.call(this, request, ...rest);
};
const lib = path.join(process.env.LOCALAPPDATA, 'Zed/extensions/work/html/node_modules/@zed-industries/vscode-langservers-extracted/packages/html/lib');
const { getLanguageModes, TextDocument, Range, FormattingOptions, ClientCapabilities } = require(path.join(lib, 'modes/languageModes.js'));
const { format } = require(path.join(lib, 'modes/formatting.js'));

const inputs = path.join(vscode, 'extensions/html-language-features/server/src/test/fixtures/inputs');
const cases = [
	['19813.html', '19813.html', FormattingOptions.create(2, true)],
	['19813.html', '19813-4spaces.html', FormattingOptions.create(4, true)],
	['19813.html', '19813-tab.html', FormattingOptions.create(1, false)],
	['21634.html', '21634.html', FormattingOptions.create(2, true)],
];
(async () => {
	fs.mkdirSync(out, { recursive: true });
	for (const [input, expected, options] of cases) {
		const workspace = { settings: undefined, folders: [{ name: 'foo', uri: 'test://foo' }] };
		const modes = getLanguageModes({ css: true, javascript: false }, workspace, ClientCapabilities.LATEST, {});
		const text = fs.readFileSync(path.join(inputs, input), 'utf8').replace(/\r\n/g, '\n');
		const document = TextDocument.create('test://test/test.html', 'html', 0, text);
		const range = Range.create(document.positionAt(0), document.positionAt(text.length));
		const edits = await format(modes, document, range, options, undefined, { css: true, javascript: false });
		fs.writeFileSync(path.join(out, expected), TextDocument.applyEdits(document, edits));
		modes.dispose();
	}
})();
