# Performance

Release builds on Windows. Quadratic time is the main risk: JS hides some costs
(string concatenation, UTF-16 offsets). Measure each feature on a large file.
The commands for each measurement are in "Benchmarks".

## Servers

Replays with `lsp-replay`. The Node servers are those of `replay-all.sh`
(`testing.md`).

| Measurement                            | Rust        | Node          |
| -------------------------------------- | ----------- | ------------- |
| JSON startup (Zed sessions)            | 3 to 16 ms  | 140 to 160 ms |
| JSON memory at end (Zed sessions)      | 12 to 31 MB | 73 to 140 MB  |
| CSS startup (Zed sessions)             | 5 to 16 ms  | 170 to 185 ms |
| CSS memory at end (Zed sessions)       | 10 to 13 MB | 80 to 115 MB  |
| CSS memory at end (feature session)    | 23 MB       | 197 MB        |
| HTML startup (`html-perf.jsonl`)       | 8 ms        | 363 ms        |
| HTML memory at end (`html-perf.jsonl`) | 14 MB       | 348 MB        |
| HTML peak memory (`html-perf.jsonl`)   | 119 MB      | 610 MB        |
| HTML session total (`html-perf.jsonl`) | 5.4 s       | 163.8 s       |
| HTML with JavaScript, startup          | 13 ms       | 315 ms        |
| HTML with JavaScript, memory at end    | 50 MB       | 334 MB        |
| HTML with JavaScript, peak memory      | 54 MB       | 365 MB        |
| HTML with JavaScript, session total    | 7.6 s       | 11.1 s        |

The HTML with JavaScript rows are `features.jsonl` with `HTML_JS=1` (Node:
TypeScript 5.9.3).

### JSON requests

`perf.sh`, each feature on large files:

| Request                                 | Rust    | Node    |
| --------------------------------------- | ------- | ------- |
| `documentSymbol`, 33 MB (166 MB result) | 2518 ms | 3451 ms |
| `formatting`, 26 MB (150 MB result)     | 482 ms  | 2905 ms |
| `diagnostic`, 33 MB (parse, no schema)  | 135 ms  | 315 ms  |
| `foldingRange`, 33 MB                   | 87 ms   | 227 ms  |
| `diagnostic` with 2.2 MB schema load    | 51 ms   | 39 ms   |
| `completion`, 19800 items               | 85 ms   | 89 ms   |

`documentSymbol`: the library takes 450 ms; the rest is serialization and the
client. Zed sets no result limit.

### HTML requests

`html-perf.jsonl`: generated large documents, JavaScript mode off. Rust: the
best of 3 runs; Node: 1 run. The responses are the same (`lsp-replay --diff`).

| Request (`large.html`, 1.8 MB)         | Rust      | Node       |
| -------------------------------------- | --------- | ---------- |
| `documentSymbol` (16.4 MB result)      | 214 ms    | 325 ms     |
| `selectionRange`, 7 positions          | 0 ms      | 749 ms     |
| `formatting` (2.2 MB result)           | 57 ms     | 292 ms     |
| `foldingRange`                         | 10 ms     | 80 ms      |
| `documentLink`                         | 46 ms     | 107 ms     |
| `diagnostic` (first, then cached)      | 14, 6 ms  | 136, 53 ms |
| `rangeFormatting`, a tenth             | 6 ms      | 132 ms     |
| `completion` after each edit (reparse) | 28 ms     | 109 ms     |
| other requests                         | 0 to 6 ms | 0 to 23 ms |

| Request (other documents)                                      | Rust  | Node    |
| -------------------------------------------------------------- | ----- | ------- |
| `minified.html` (0.8 MB, one line), `rangeFormatting`          | 3 ms  | 154.3 s |
| `styles.html` (bootstrap 4 times in a `<style>`), `diagnostic` | 19 ms | 157 ms  |
| `styles.html`, `documentHighlight` in CSS                      | 4 ms  | 44 ms   |

Each other request on `minified.html` is also faster than Node.

- `selectionRange` uses the cached parse. Upstream parses the document once for
  each position (`getSelectionRanges(document, [position])`).
- Handlers return typed results, not `json!` values. For `documentSymbol`, a
  `Value` tree takes 115 ms to build and 100 ms to drop (of 399 ms), and adds 70
  MB to the peak memory. Requests that wait for settings (completion, hover,
  formatting, diagnostics) serialize to a `RawValue`: a completion of 0.9 MB
  takes 5 ms (8 ms with a `Value`).
- `documentSymbol`: parse 24 ms, symbols 20 ms, serialization about 22 ms, the
  rest is transfer and the client. Each `SymbolInformation` has its own `Uri`:
  the clones are 30% of the symbols.

### HTML JavaScript mode

Completion in a script (about 1050 items): 2.5 ms in the library (Node median 7
ms); other script requests 0 to 9 ms. The first request of a session parses the
libraries (about 15 ms). Completion items are typed `Serialize` values (13 to 16
ms with `json!` values). The binary is 26 MB with the mode, 5.6 MB without; the
clean release build of tsrs takes about 50 s.

## HTML library

`html_bench` on `large.html`, `minified.html` and `styles.html`, best of 10.
Node: the same steps on upstream `lib/esm` (a scratch script, not tracked).

| Step     | large Rust | large Node | minified Rust | minified Node | styles Rust | styles Node |
| -------- | ---------- | ---------- | ------------- | ------------- | ----------- | ----------- |
| parse    | 24.0 ms    | 54.1 ms    | 11.4 ms       | 24.5 ms       | 0.6 ms      | 1.5 ms      |
| symbols  | 19.4 ms    | 21.6 ms    | 8.7 ms        | 5.5 ms        | 0.3 ms      | 0.5 ms      |
| symbols2 | 17.6 ms    | 17.5 ms    | 7.7 ms        | 3.6 ms        | 0.2 ms      | 0.4 ms      |
| folding  | 7.0 ms     | 56.7 ms    | 2.5 ms        | 22.7 ms       | 0.2 ms      | 1.5 ms      |
| links    | 12.3 ms    | 48.7 ms    | 5.8 ms        | 22.5 ms       | 0.4 ms      | 1.4 ms      |
| format   | 52.3 ms    | 196.5 ms   | 25.1 ms       | 89.4 ms       | 9.4 ms      | 61.7 ms     |

Symbols on `minified.html` are slower than Node: one `String` for each name and
container name, and a `Uri` clone for each symbol, on the Windows heap. Node
allocates in its nursery and can collect after the timed call.

The Windows heap is the largest cost. `parse`: `String` copies of tag names,
attribute names and values (41%), growth of the attribute and child `Vec`s
(20%), the scanner (20%). `format`: the token, tag and output item `Vec`s are
allocated with their final size (counted, or about 3.5 tokens for each `<`);
without that, page faults of the growth are 20%.

Costs that profiles found and that the port avoids:

- `isVoidElement` with ICU collation for each tag (a third of the parse). The
  search compares bytes.
- A lowercase `String` of each tag and attribute name in the scanner and the
  parser, and a copy of each pending attribute name.
- In the formatter: a `String` for each output item, a `Vec` of items for each
  line, an indent `String` for each line, and 3 to 4 lowercase copies of each
  tag name.
- `findDocumentSymbols` builds the `DocumentSymbol` tree, then copies it.

## CSS library

`css_bench` on bootstrap, formatted and repeated 4 times (506 KB, 30477 lines,
`<file> 4 40`):

| Step                     | Time    |
| ------------------------ | ------- |
| parse                    | 8.1 ms  |
| validation               | 5.9 ms  |
| symbols                  | 3.3 ms  |
| colors                   | 1.4 ms  |
| folding                  | 2.5 ms  |
| format                   | 5.5 ms  |
| completion, 207 requests | 31.8 ms |

Parse profile: selectors 40%, expressions 27%, scanner 26%. An 8 MB style sheet
formats in 104 ms after a parse of 135 ms. Format profile: the scanner is a
quarter; the printer reuses its buffers (item lists, statement marks, the walk
stack), which were 40% as new allocations.

A property-name completion has 888 items in 968 KB of JSON; 702 KB of it is the
markdown documentation. The service builds it in about 0.13 ms; the request
takes 8 to 10 ms in a replay (serialization and transfer), the same as Node.

## Benchmarks

JSON:

- Library:
  `cargo run --release --target-dir target/replay -p json-languageservice --example parse_bench -- <file>`,
  and `--example schema_bench -- .scratch/schema-mem/uses-big-schema.json`.
- `node scripts/gen-schema-session.mjs <zed session> .scratch/schema-mem 20000`:
  a 2.2 MB schema, a document that uses it, and a session.
- Server:
  `node scripts/gen-perf-session.mjs <zed session> .scratch/data/perf.jsonl`
  writes a session with each feature on `large-pretty.json` (33 MB),
  `large-file.json` (26 MB) and the large schema document. Then
  `sh scripts/perf.sh <label>` (label `node` runs Node) and
  `node scripts/perf-table.mjs .scratch/replays/perf-<a>.json ...`.

CSS:

- Library:
  `cargo run --release --target-dir target/replay -p css-languageservice --example css_bench -- <style sheet> [repeat] [iterations]`.
  Formats the style sheet, repeats it, and times parse, validation, symbols,
  colors, folding, format, and completion and hover at each 97th offset of the
  first 20 KB. Input: `.scratch/test_files/css/bootstrap.min.css`.

HTML:

- Library:
  `cargo run --release --target-dir target/replay -p html-languageservice --example html_bench -- <document> [iterations]`.
  Times parse, symbols (`SymbolInformation` and `DocumentSymbol`), folding,
  links and format. The time of a step includes the drop of the result of the
  previous iteration.
- Server:
  `node scripts/html/gen-html-perf-session.mjs <zed session> .scratch/data/html-perf.jsonl`
  writes `large.html`, `minified.html` and `styles.html` to
  `.scratch/test_files/html-perf/` and a session: each feature, completion,
  hover, highlights, linked editing and rename at 7 kinds of positions, and 5
  typing edits each followed by completion. Then
  `target/replay/release/lsp-replay.exe --times .scratch/data/html-perf.jsonl <out.json> -- <server> --stdio`.
  Node needs `LSP_REPLAY_TIMEOUT=600` (seconds; default 60) for its range format
  of the one-line document.

## Profiling

Profile before each change; guesses were often wrong.

- Build with `CARGO_PROFILE_RELEASE_DEBUG=true` (use its own target folder).
- `samply record --save-only --unstable-presymbolicate -o <out>.json.gz -r 4000 <exe> <args>`
  (records through xperf).
- samply does not resolve the functions of the binaries (only `fun_<rva>`), but
  the PDB is correct.
  `node scripts/prof.js <out>.json.gz [stack regex] [top N] [--lines] [--callers regex] [--process regex]`
  symbolizes with `llvm-symbolizer` (inlined frames too) and prints self and
  inclusive time. The regex keeps only samples with a matching frame, for
  example `'parse_stylesheet$'`. It reads the thread with the most samples;
  `--process` limits it to the processes that match.
- A server under `lsp-replay`: record the replay, then use
  `--process html-language`. The server waits most of the time; the stack regex
  `'Server::(on_|handle|dispatch)|lsp_core'` keeps the busy samples. A
  sample is 0.25 ms at `-r 4000`.

## Open ideas

Each needs a profile first.

- CSS completion documentation through `completionItem/resolve`: about 73% of a
  property completion response. Zed supports resolve of `documentation` and
  `detail`. Differs from upstream.
- CSS parse: each declaration gets an `Extra` for its colon and semicolon
  positions (about 100 bytes, 7% of the parse). `Node` has no free room.
- JSON parse (96 ms for 33 MB): `index_children`, growth of the node `Vec`.
  Errors must keep their order: duplicate-key diagnostics are emitted during the
  parse.
- JSON schema load (2.2 MB, Node about 40 ms): one `Box<str>` for each key in
  `schema/tree.rs` (keys repeat: an interner is possible). `jsonc::visit` keeps
  a `path` with one `String` for each property, also when the visitor does not
  read it.
- JSON document symbols: name and detail `String`s, range conversion for each
  node.
- Freeing large data blocks the next request (`configure` frees the old schemas,
  11 ms for 20k nodes). A possible fix: drop on a background thread.
- JSON validation: fewer `String` allocations in `processed_properties`, no
  `node_value` for each `enum` check.
- A global allocator. mimalloc in the HTML server (`html-perf.jsonl`):
  formatting 141 to 71 ms, completion after an edit 29 to 16 ms, but memory at
  end 15 to 79 MB and peak 150 to 204 MB. With `MIMALLOC_PURGE_DELAY=0`:
  formatting 106 ms, completion 21 ms, end 20 MB, peak 144 MB. Adds a C
  dependency. Not used (the user's choice).
- HTML parse: the `String`s of tag names, attribute names and values (41% of the
  parse). Ranges in the text or interned names change the `Node` API.
- HTML links: `resolve_reference` parses the base URI, its `dirname` and the
  scheme of the document for each link (about 30% of `documentLink`). A cache of
  the parsed base for each request.
