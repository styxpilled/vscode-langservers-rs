import fs from "fs";
for (const f of process.argv.slice(2)) {
  console.log("==", f);
  const lines = fs.readFileSync(f, "utf8").split("\n").filter(Boolean);
  const pending = {};
  for (const l of lines) {
    const r = JSON.parse(l);
    const m = r.msg;
    if (!m) { console.log(r.t, r.dir, JSON.stringify(r).slice(0, 200)); continue; }
    let s = r.t + " " + r.dir + " " + (m.method || "resp " + m.id);
    if (m.method === "textDocument/didOpen") s += " " + m.params.textDocument.uri + " len=" + m.params.textDocument.text.length + " lang=" + m.params.textDocument.languageId;
    else if (m.params?.textDocument) s += " " + m.params.textDocument.uri;
    if (m.method && m.id !== undefined) pending[m.id + r.dir] = r.t;
    if (!m.method) {
      const k = m.id + (r.dir === "out" ? "in" : "out");
      if (pending[k] !== undefined) s += " dt=" + (r.t - pending[k]);
      s += " " + String(JSON.stringify(m.result ?? m.error)).slice(0, 160);
    }
    if (m.method === "textDocument/publishDiagnostics") s += " n=" + m.params.diagnostics.length;
    if (m.method === "workspace/didChangeConfiguration") s += " " + JSON.stringify(m.params).length + "B";
    console.log(s);
  }
}
