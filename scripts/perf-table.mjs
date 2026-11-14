// Prints the request times of replay results side by side.
// Usage: node perf-table.mjs <a.json> <b.json> ...
import fs from "fs";
const results = process.argv.slice(2).map((f) => JSON.parse(fs.readFileSync(f, "utf8")));
const name = (r) => {
  const uri = r.params?.textDocument?.uri ?? "";
  return `${uri.split("/").pop().padEnd(22)} ${r.method.replace("textDocument/", "").padEnd(16)}`;
};
console.log("".padEnd(39), process.argv.slice(2).map((f) => f.split(/[\\/]/).pop().slice(0, 12).padStart(13)).join(""));
results[0].requests.forEach((r, i) => {
  const size = r.response_bytes ?? JSON.stringify(r.response).length;
  console.log(name(r), results.map((x) => String(x.requests[i]?.millis).padStart(13)).join(""), `  ${(size / 1e6).toFixed(2)} MB`);
});
for (const k of ["millis", "end_memory_mb", "peak_memory_mb"])
  console.log(k.padEnd(39), results.map((x) => String(x[k]).padStart(13)).join(""));
