// Writes the upstream built-in CSS data (`webCustomData.ts`) as JSON.
// Usage: node gen-browser-data.mjs <vscode-css-languageservice dir> <out.json>
import fs from 'fs';
import path from 'path';
import url from 'url';

const [root, out] = process.argv.slice(2);
const lib = url.pathToFileURL(path.resolve(root, 'lib/esm')).href;
const { cssData } = await import(`${lib}/data/webCustomData.js`);
fs.writeFileSync(out, JSON.stringify(cssData));
