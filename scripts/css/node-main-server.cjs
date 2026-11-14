// Runs the Node CSS server of vscode-langservers-extracted 4.10.0 with the
// upstream `main` clone of vscode-css-languageservice (the port target), and
// with `doCodeActions2` in place of `doCodeActions`, as the current server source.
// Usage: node node-main-server.cjs --stdio
const Module = require('module');
const path = require('path');

const ls = require(path.resolve(__dirname, '../../upstream/vscode-css-languageservice/lib/esm/cssLanguageService.js'));
const wrap = (factory) => (...args) => {
	const service = factory(...args);
	return { ...service, doCodeActions: service.doCodeActions2 };
};
const id = path.resolve(__dirname, 'vscode-css-languageservice-main.cjs');
const fake = new Module(id);
fake.filename = id;
fake.loaded = true;
fake.exports = {
	...ls,
	getCSSLanguageService: wrap(ls.getCSSLanguageService),
	getSCSSLanguageService: wrap(ls.getSCSSLanguageService),
	getLESSLanguageService: wrap(ls.getLESSLanguageService),
};
require.cache[id] = fake;
const resolve = Module._resolveFilename;
Module._resolveFilename = function (request, ...rest) {
	return request === 'vscode-css-languageservice' ? id : resolve.call(this, request, ...rest);
};
require(path.join(process.env.LOCALAPPDATA, 'Zed/languages/vscode-css-language-server/node_modules/vscode-langservers-extracted/bin/vscode-css-language-server'));
