# Testing

Scripts are in `scripts/`. Fixtures that tests read are in
`crates/*/tests/fixtures/`. Large data (sweeps, sessions, test files, replay
results) is in `.scratch/` (not tracked). Run the scripts from the repository
root.

## Checks for each change

- `cargo test --workspace`, `cargo clippy --workspace --all-targets`,
  `cargo fmt --check`.
- The sweeps of the changed area (all CSS:
  `sh scripts/css-sweeps.sh [--no-completion]`, about 10 minutes, completion is
  8 of them; all HTML: `sh scripts/html-sweeps.sh`, about 2 minutes).
- `sh scripts/replay-all.sh`: only the expected differences (last section).
- Before you trust a passing fixture or sweep, change the code on purpose and
  check that the test fails. Use debug builds and `timeout` for this: some
  mutations loop forever.

## Methods

- Generated tests: a generator runs the upstream TS test files under Node with
  stub assertion helpers, records the calls, and writes Rust test functions. The
  helpers at the top of each Rust test file are written by hand.
- Trace fixtures: a script imports all compiled upstream test files, patches the
  upstream entry points, and records each top-level call with its full result.
  The Rust test replays the calls. The fixtures record more than the upstream
  asserts check. Local paths in a fixture become `/repo`.
- Sweeps: each feature at each offset of many documents (test texts, fixture
  inputs, generated texts, three dialects), recorded from upstream. Sweep files
  are large and are read from an absolute path in an environment variable;
  without it the sweep test skips.
- Robustness: each feature on each prefix of malformed non-ASCII text, no
  panics. Extend the robustness tests for each new feature.
- Session replay: sessions recorded in Zed (or generated) go to the Node and the
  Rust server, and the results are compared.

## Upstream builds

`upstream/vscode-json-languageservice`: npm refuses the git dependency
`json-schema-test-suite`, so remove it from `package.json` first:

```
node -e 'const f="package.json";const p=JSON.parse(require("fs").readFileSync(f));delete p.devDependencies["json-schema-test-suite"];require("fs").writeFileSync(f,JSON.stringify(p,null,2))'
npm install --ignore-scripts --no-package-lock
git checkout package.json
npx tsc -p ./src
```

`node_modules/json-schema-test-suite` is a junction to
`upstream/JSON-Schema-Test-Suite`. Tests:
`node --test ./lib/esm/test/*.test.js`.

`upstream/vscode-css-languageservice` and
`upstream/vscode-html-languageservice`:
`npm install --ignore-scripts --no-package-lock`, `npx tsc -p ./src`. CSS tests:
`node --test 'lib/esm/test/**/*.test.js'`.

## JSON

- Generators: `gen-jsonc-tests.mjs <upstream/node-jsonc-parser/src/test> <out>`,
  `gen-ls-tests.mjs <upstream/vscode-json-languageservice/src/test> <out>`
  (folding, formatter, selection ranges, sort), `gen-schema-unit-tests.mjs`
  (glob, vocabularies),
  `gen-schema-contributions.mjs <src> crates/json-languageservice/src/schemas.json`.
- `tests/upstream_trace.rs` (`parser.test`, `schema.test`):
  `node scripts/trace/run.mjs upstream/vscode-json-languageservice crates/json-languageservice/tests/fixtures/upstream_trace.json parser.test.js schema.test.js`.
  One test is skipped: it uses one JS object at two places of a schema, which
  JSON cannot express. Upstream mutates schema objects that tests pass to
  several calls, so the recorder clones each input. Some tests do not await
  their calls; the shim waits 50 ms after each test.
- `tests/upstream_features.rs` (completion, hover, links, document symbols):
  `node scripts/trace/ls-run.mjs upstream/vscode-json-languageservice crates/json-languageservice/tests/fixtures/upstream_features.json completion.test.js hover.test.js links.test.js documentSymbols.test.js`.
- Sweep (completion and hover at each offset, 7266 calls):
  `node scripts/trace/ls-sweep.mjs upstream/vscode-json-languageservice crates/json-languageservice/tests/fixtures/upstream_features.json .scratch/data/sweep.json`,
  then
  `JSON_LS_SWEEP=<path> cargo test -p json-languageservice --test upstream_features`.
- `tests/schema_test_suite.rs`: all of JSON-Schema-Test-Suite. Skips without the
  clone.
- `tests/schema_reset.rs`: schema reload after content changes.
- Robustness: `port.rs` in `jsonc`, `robustness.rs` in the service.
- Server: `crates/json-language-server/tests/stdio.rs` (the binary over stdio).

## CSS

All trace scripts are in `scripts/css/` and take
`upstream/vscode-css-languageservice` as the first argument. A script with sweep
arguments also writes the sweep; without them, only the fixture.

| Area           | Script                                                  | Test                               | Sweep variable         |
| -------------- | ------------------------------------------------------- | ---------------------------------- | ---------------------- |
| Scanner        | `trace-scanner.mjs <fixture>`                           | `tests/scanner.rs`                 |                        |
| Parser         | `trace-parser.mjs <fixture> [inputs]`                   | `tests/parser.rs`                  | `CSS_PARSER_SWEEP`     |
| Language facts | `trace-facts.mjs <fixture> <sweep>`                     | `tests/language_facts.rs`          | `CSS_FACTS_SWEEP`      |
| Lint           | `trace-lint.mjs <fixture> [inputs sweep]`               | `tests/lint.rs`                    | `CSS_LINT_SWEEP`       |
| Hover          | `trace-hover.mjs <fixture> [parser fixture sweep]`      | `tests/hover.rs`                   | `CSS_HOVER_SWEEP`      |
| Completion     | `trace-completion.mjs <fixture> [parser fixture sweep]` | `tests/completion.rs`              | `CSS_COMPLETION_SWEEP` |
| Navigation     | `trace-navigation.mjs <fixture> [parser fixture sweep]` | `tests/navigation.rs`              | `CSS_NAVIGATION_SWEEP` |
| Formatter      | `trace-format.mjs <fixture> [parser fixture sweep]`     | `tests/formatter.rs`               | `CSS_FORMAT_SWEEP`     |
| Links          | `trace-links.mjs <fixture or -> [sweep]`                | `tests/links.rs`                   | `CSS_LINKS_SWEEP`      |
| URI            | `trace-uri.mjs <fixture>`                               | `crates/vscode-uri/tests/utils.rs` |                        |

- Parser sweep (114129 parses): `gen-parser-sweep.mjs` writes
  `css-sweep-inputs.json` (each prefix of each style sheet of the parser
  fixture, also with non-ASCII text, three dialects); then
  `node --max-old-space-size=8000 trace-parser.mjs <upstream> .scratch/data/css-parser-sweep.json .scratch/data/css-sweep-inputs.json`.
  The `robustness` tests read `CSS_SWEEP_INPUTS` (the same inputs).
- Nested sweep (13266 parses): `gen-nested-sweep.mjs` writes
  `css-nested-inputs.json` (rule sets and declarations in nested bodies,
  namespace prefixes); then `trace-parser.mjs` with
  `.scratch/data/css-nested-sweep.json`. It checks the parser lookaheads.
- Language facts: numbers are strings in the fixture. Float results accept 1 ULP
  or 1e-12.
- Completion sweep: a hash of the canonical JSON of each result (89169 offsets;
  Node 7 minutes, the Rust test 8 minutes in release).
- Navigation sweep: highlights, definition, rename and selection ranges at each
  offset; symbols, colors, folding, code actions; color presentations on a grid
  of colors. It patches upstream `difference` (`differences/css-navigation.md`).
- Formatter: the script patches two upstream quirks; `UNPATCHED=1` turns the
  patches off.
- Links: the fixture records each file system and `resolveReference` call; the
  Rust test answers from it, and a call that upstream did not make fails. The
  sweep uses a virtual file system. Debug build is enough (2 s).
- Generated sources:
  `gen-browser-data.mjs <upstream> crates/css-languageservice/src/data/browsers.json`,
  `gen-builtin-data.mjs <upstream> crates/css-languageservice/src/language_facts`.
- Server: `crates/css-language-server/tests/stdio.rs` (the binary over stdio),
  unit tests in `src/fs.rs` (the upstream server tests, fixtures copied to
  `tests/fixtures`) and `src/settings.rs`.

## HTML

- Generated sources:
  `node scripts/html/gen-data.mjs upstream/vscode-html-languageservice crates/html-languageservice/src`
  (`data/html5.json`, `parser/entities.rs`).
- `tests/upstream.rs`:
  `node scripts/html/trace.mjs upstream/vscode-html-languageservice crates/html-languageservice/tests/fixtures/upstream.json`.
  Records each top-level call of the upstream tests: scanner token streams (plus
  extra inputs), parses with `findNodeBefore` and `findNodeAt` at each offset,
  and the features with their results. `resolveReference`, `readDirectory` and
  completion participant calls are recorded; the Rust test answers from them.
  Scanner cases where upstream splits a surrogate pair are skipped
  (`differences/html-service.md`).
- Format sweep (`tests/formatter.rs`, 15406 results):
  `node scripts/html/trace-format.mjs upstream/vscode-html-languageservice crates/html-languageservice/tests/fixtures/upstream.json upstream/vscode .scratch/data/html-format-sweep.json`,
  then
  `HTML_FORMAT_SWEEP=<absolute path> cargo test -p html-languageservice --test formatter`.
  The texts of the fixture, of the VS Code server test fixtures and a list of
  extra texts, with 14 option sets: the full document, each line and
  pseudo-random ranges. The same test formats each prefix of each text with the
  templating option sets (no panics).
- Feature sweep (`tests/sweep.rs`, 158128 results):
  `node scripts/html/trace-sweep.mjs upstream/vscode-html-languageservice crates/html-languageservice/tests/fixtures/upstream.json upstream/vscode .scratch/data/html-sweep.json`,
  then
  `HTML_SWEEP=<absolute path> cargo test --release -p html-languageservice --test sweep`.
  The texts of the format sweep (`scripts/html/texts.mjs`) and a list of extra
  texts. At each offset, a hash of completion (`doComplete2` with a virtual file
  system and the participant contexts), hover, quote and tag completion,
  highlights, rename, matching tag, linked editing and selection ranges; each
  7th offset also completion and hover without markdown. The full results of
  folding, symbols and links. Path completion in a value that is not quoted on
  one line is skipped (`experimental.md`; 137 results, 13 of them where upstream
  throws).
- Robustness (`robustness` in `tests/sweep.rs`, same variable, 90 s in release):
  each feature at each offset of each prefix of two non-ASCII variants of each
  sweep text, no panics.
- Server unit tests (`crates/html-language-server/src/tests.rs`): the upstream
  `embedded`, `folding`, `formatting`, `selectionRanges`, `completions`,
  `javascriptMode`, `semanticTokens`, `documentContext`, `hover` and `rename`
  tests, and settings. The JavaScript cases are in `mod javascript` (feature
  `javascript`); a tsrs value that differs from TypeScript 5 has a
  `// Upstream (TypeScript 5):` comment. The formatting fixtures in
  `tests/fixtures` are the upstream files, with the script content formatted by
  tsrs. Without the `javascript` feature the script content is not formatted:
  the expected files of that build (`expected-nojs`) come from the Node
  reference without the JavaScript mode
  (`node scripts/html/record-fixture-format.cjs upstream/vscode <dir>`).
- Server over stdio (`tests/stdio.rs`): the binary with settings requests.
- Server robustness (`src/robustness.rs`): each mode feature at each position of
  each prefix of non-ASCII texts with embedded CSS, no panics. 60 s in release
  with the JavaScript mode:
  `HTML_SERVER_ROBUSTNESS=1 cargo test --release -p html-language-server robustness`.

## Sessions and replays

Sessions are in `.scratch/sessions/` (CSS in `css/`, HTML in `html/`).

- `lsp-recorder --out <file or folder> -- <server command>`: copies stdio and
  writes each message as a JSON line (`t`, `dir` = `in` / `out` / `meta`,
  `msg`). Use it as the server command in Zed to record a session.
- `lsp-replay [--times] <session.jsonl> <result.json> -- <server command>`:
  sends the client messages, waits for each response, answers server requests
  from the recording. The result has `millis` for each request,
  `startup_millis`, `end_memory_mb` and `peak_memory_mb`. `--times` keeps only
  response sizes. It waits 60 s for each response (`LSP_REPLAY_TIMEOUT`
  overrides it, in seconds).
- `lsp-replay --diff <a.json> <b.json>`: first difference of each response, of
  the last diagnostics for each URI, and of the server requests (with the number
  of requests that only one side sent).
- `scripts/timeline.mjs <session>`: messages with response times.
- Generated sessions, from the `initialize` and configuration answers of a Zed
  session:
  - `scripts/css/gen-css-session.mjs <zed session> <out> [--push]`: each CSS
    feature at each third offset of `.scratch/test_files/css/`.
  - `scripts/html/gen-html-session.mjs <zed session> <out> [--push]`: each HTML
    feature at each offset of `.scratch/test_files/html/`, with typing that
    gives snippet edits, and a generated document for CSS folding at the end of
    a style region, a range format that starts in a style region and the hover
    settings.

`sh scripts/replay-all.sh [session ...]` replays each session to Node and Rust
and diffs the results. Zed locks `target/release/*.exe`, so the Rust servers
come from `target/replay`. The Node servers:

- JSON: the Zed install in
  `%LOCALAPPDATA%/Zed/languages/<server>/node_modules/vscode-langservers-extracted/bin/`.
- CSS: `scripts/css/node-main-server.cjs`, the 4.10.0 server code with the
  upstream `main` language service. `CSS_NODE=zed` uses the Zed install as is.
- HTML: `scripts/html/node-main-server.cjs`, Zed's 4.10.8 server with the
  upstream `main` services, without the JavaScript mode. The Rust server is then
  the build without the `javascript` feature
  (`cargo build --release --target-dir target/replay-nojs -p html-language-server --no-default-features`).
  `HTML_JS=1` keeps the JavaScript mode in both.

### Expected replay differences

Also each difference of `experimental.md`.

JSON:

- `codeActionProvider` in the `initialize` result.
- The formatter edit limit (`differences/json-server.md`).
- One completion change in upstream since 4.10.0 (`insertText` of a property).
- The exit code of a session with `shutdown` before `initialize`: Rust follows
  the LSP specification.
- The order of the `vscode/content` requests (`zed://schemas/jsonc` and
  `zed://schemas/settings`).
- Fewer `vscode/content` requests (the content cache,
  `differences/json-server.md`). A diagnostic request between a configuration
  change and the next `json/schemaContent` can then use the earlier schema
  content (Zed cancels these requests).

CSS, against `main`:

- `codeActionProvider` in the `initialize` result.
- The `references` field of the SCSS `@use` and `@forward` completion items.
- Completion at the start of a variable declaration in a block (upstream throws
  and answers `null`).

CSS, against the Zed install (`CSS_NODE=zed`), also: commands in place of code
actions, and the 6.3.10 to `main` changes in `zed.md`.

HTML:

- `serverInfo` in the `initialize` result.
- The id of the `client/registerCapability` registration (one server request
  only in each side).
- Zed sessions: diagnostics of documents with CSS. Zed answers `null` for the
  `css` section and upstream throws (`differences/html-server.md`).

HTML with `HTML_JS=1`, also: the JavaScript mode differences in
`differences/html-server.md`, and the `main` changes that the 4.10.8 server does
not have (`zed.md`).
