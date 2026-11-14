// Writes the `schemaContributions` of `services/configuration.ts` as JSON, after the
// description loop at the end of that file has run.
// Usage: node gen-schema-contributions.mjs <vscode-json-languageservice/src> <out.json>
import fs from 'fs';
import path from 'path';

const [src, out] = process.argv.slice(2);
const read = f => fs.readFileSync(path.join(src, f), 'utf8');
const flat = f => read(f).replace(/^import .*$/gm, '').replace('export default', 'return');

let config = read('services/configuration.ts')
	.replace(/^import .*$/gm, '')
	.replace(/export const schemaContributions\s*:\s*ISchemaContributions\s*=/, 'const schemaContributions =')
	.replace(/const descriptions\s*:\s*\{[^}]*\}\s*=/, 'const descriptions =');
config += '\nreturn schemaContributions;';

const l10n = { t: s => s };
const draft201909Flat = new Function(flat('services/schemas/draft-2019-09-flat.ts'))();
const draft202012Flat = new Function(flat('services/schemas/draft-2020-12-flat.ts'))();
const contributions = new Function('l10n', 'draft201909Flat', 'draft202012Flat', config)(l10n, draft201909Flat, draft202012Flat);
fs.writeFileSync(out, JSON.stringify(contributions.schemas, null, '\t') + '\n');
