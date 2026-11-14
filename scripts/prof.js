// Usage: node prof.js <profile.json.gz> [stack filter regex] [top N] [--lines] [--callers regex] [--process regex]
// Frames in .exe modules are symbolized with llvm-symbolizer (with inlined frames).
const zlib = require("zlib"), fs = require("fs"), cp = require("child_process");
const args = process.argv.slice(2);
const lines = args.includes("--lines");
const ci = args.indexOf("--callers");
const callersRe = ci >= 0 ? new RegExp(args.splice(ci, 2)[1]) : null;
const pi = args.indexOf("--process");
const processRe = pi >= 0 ? new RegExp(args.splice(pi, 2)[1]) : /./;
const callers = new Map();
const [file, filter, topArg] = args.filter(a => a !== "--lines");
const top = +topArg || 30;
const SYMBOLIZER = "C:\\Program Files\\LLVM\\bin\\llvm-symbolizer.exe";
const p = JSON.parse(zlib.gunzipSync(fs.readFileSync(file)));
const thread = p.threads.filter(t => processRe.test(t.processName)).reduce((a, b) => (a.samples.length > b.samples.length ? a : b));
const { frameTable, funcTable, stackTable, samples, resourceTable } = thread;

const short = n => n
  .replace(/::h[0-9a-f]{16}$/, "")
  .replace(/\b[a-z_]+::/g, s => (s === "core::" || s === "alloc::" || s === "std::" ? s : s))
  .slice(0, 160);

// frame index -> list of names, innermost first
const frameNames = new Array(frameTable.length);
const byLib = new Map();
for (let i = 0; i < frameTable.length; i++) {
  const f = frameTable.func[i];
  const res = funcTable.resource[f];
  const lib = res == null || res < 0 ? null : p.libs[resourceTable.lib[res]];
  if (lib && lib.path.toLowerCase().endsWith(".exe") && !lib.path.toLowerCase().includes("\\windows\\")) {
    if (!byLib.has(lib.path)) byLib.set(lib.path, []);
    byLib.get(lib.path).push(i);
  } else {
    frameNames[i] = [(lib ? lib.name + "!" : "") + thread.stringArray[funcTable.name[f]]];
  }
}
for (const [path, frames] of byLib) {
  const addrs = frames.map(i => "0x" + frameTable.address[i].toString(16));
  const out = cp.execFileSync(SYMBOLIZER, ["--obj=" + path, "--relative-address", "--output-style=JSON", "--demangle"],
    { input: addrs.join("\n"), maxBuffer: 1 << 28 }).toString();
  out.trim().split(/\r?\n/).forEach((l, k) => {
    const r = JSON.parse(l);
    frameNames[frames[k]] = (r.Symbol || []).map(s =>
      short(s.FunctionName || "?") + (lines ? ` @${(s.FileName || "").split(/[\\/]/).pop()}:${s.Line}` : ""));
    if (!frameNames[frames[k]].length) frameNames[frames[k]] = [addrs[k]];
  });
}

const re = filter ? new RegExp(filter) : null;
const self = new Map(), incl = new Map();
let total = 0;
for (let i = 0; i < samples.length; i++) {
  let s = samples.stack[i];
  if (s == null) continue;
  const names = [];
  for (; s != null; s = stackTable.prefix[s]) names.push(...frameNames[stackTable.frame[s]]);
  if (re && !names.some(n => re.test(n))) continue;
  const w = samples.weight ? samples.weight[i] : 1;
  total += w;
  self.set(names[0], (self.get(names[0]) || 0) + w);
  for (const n of new Set(names)) incl.set(n, (incl.get(n) || 0) + w);
  if (callersRe) {
    const k = names.findIndex(n => callersRe.test(n));
    if (k >= 0) {
      const chain = names.slice(k + 1).filter(n => !/^(alloc|core|std)::/.test(n)).slice(0, 2).join("  <-  ");
      callers.set(chain, (callers.get(chain) || 0) + w);
    }
  }
}
const show = (title, m) => {
  console.log(`\n== ${title} (total ${total} samples) ==`);
  [...m].sort((a, b) => b[1] - a[1]).slice(0, top)
    .forEach(([n, c]) => console.log(`${(100 * c / total).toFixed(1).padStart(5)}%  ${n}`));
};
show("self", self);
show("inclusive", incl);
if (callersRe) show("callers (first two non-std frames)", callers);
