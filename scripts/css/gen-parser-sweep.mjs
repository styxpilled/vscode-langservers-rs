// Writes parse inputs for the parser sweep: each prefix of each style sheet
// input of the parser fixture, in each dialect, also with non-ASCII text.
// Usage: node gen-parser-sweep.mjs <parser fixture> <out inputs.json>
import fs from 'fs';
const [fixture, out] = process.argv.slice(2);
const cases = JSON.parse(fs.readFileSync(fixture, 'utf8'));
const texts = new Set(cases.filter((c) => c.fn === '_parseStylesheet').map((c) => c.input));
const inputs = new Map();
for (const text of texts) {
	const variants = [text, text.replace(/[a-z]/, 'é').replace(/ /, ' \u{1F600}')];
	for (const v of variants) {
		for (let i = 0; i <= v.length; i++) {
			const input = v.slice(0, i);
			if (/[\uD800-\uDBFF]$/.test(input)) continue;
			for (const dialect of ['css', 'scss', 'less']) {
				const e = { dialect, fn: '_parseStylesheet', input };
				inputs.set(JSON.stringify(e), e);
			}
		}
	}
}
fs.writeFileSync(out, JSON.stringify([...inputs.values()]));
console.error(`${inputs.size} inputs`);
