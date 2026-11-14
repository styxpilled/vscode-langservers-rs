// Runs the Node HTML server of Zed's `@zed-industries/vscode-langservers-extracted`
// (4.10.8) with the upstream `main` clones of vscode-html-languageservice and
// vscode-css-languageservice (the port targets), and without the JavaScript mode
// unless HTML_JS=1.
// Usage: node node-main-server.cjs --stdio
const Module = require('module');
const path = require('path');

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
const modes = require(path.join(lib, 'modes/languageModes.js'));
const getLanguageModes = modes.getLanguageModes;
if (process.env.HTML_JS !== '1') {
	modes.getLanguageModes = (supported, ...args) => getLanguageModes({ ...supported, javascript: false }, ...args);
}
require(path.join(lib, 'node/htmlServerMain.js'));
