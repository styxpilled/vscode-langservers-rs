# Zed

Facts from `upstream/zed` (sparse clone). The settings that make Zed use the
ports are in the `README.md` of the repository. To record a session, put
`lsp-recorder` in front of the server command (`testing.md`).

## JSON

From `crates/languages/src/json.rs` and
`crates/project/src/lsp_store/json_language_server_ext.rs`:

- Server name `json-language-server`, started with `--stdio`.
- `initializationOptions`: `{ "provideFormatter": true }`.
- `workspace/configuration` section `json`: `schemas` (with `fileMatch`, `url`,
  inline `schema`) and the other `json.*` settings. Zed sends
  `workspace/didChangeConfiguration` with the full settings after `initialize`
  and on each settings change.
- Schemas are `zed://schemas/...` URIs. Zed answers the `vscode/content` request
  for them and reads its params as `Vec<String>`. Zed sends `json/schemaContent`
  when such a schema changes.
- The other custom messages (`json/schemaAssociations`, `json/validate`,
  `json/sort`, ...) are VS Code only. The server implements them for parity.
- Zed formats JSON with Prettier by default. To use the server formatter:
  `"languages": { "JSON": { "formatter": "language_server" }, "JSONC": { ... } }`.
- Zed sets no `resultLimit`, so document symbols are unlimited.
- Zed does not send a file with a line longer than
  `global_lsp_settings.max_buffer_line_length` (default 20000).
- Protocol detail: `vscode-jsonrpc` sends a single parameter that is not an
  object as `[param]`. The server sends `[uri]` in `vscode/content` and unwraps
  `[param]` in the `json/*` notifications.

## CSS

From `crates/languages/src/css.rs`:

- Server name `vscode-css-language-server`, started with `--stdio`.
- `initializationOptions`: `{ "provideFormatter": true }`.
- `workspace/configuration` sections `css`, `scss`, `less`, each with `lint`.
  The server asks for each document with `scopeUri` and the language ID as the
  section.
- Zed has a built-in adapter for CSS only. SCSS and Less come from extensions.
- `fs/content`, `fs/stat`, `fs/readDir` requests are only for schemes in
  `handledSchemas` (default `file`, read directly). Zed uses only `file` URIs.
- `css/customDataChanged` loads custom data.

### Changes from the Node server in Zed

Zed runs the 4.10.0 server with `vscode-css-languageservice` 6.3.10. The port
follows `main` of both. The scanner, the formatter, the browser data and the
public API are the same. A Zed user sees:

- Fewer false parse errors: `:nth-child()` hover (6.3.10 throws), binary
  expression ranges include the left operand (`calc(2 * var(--x))`), SCSS `#{}`
  in unknown at-rules, `@supports` interpolation, keyframe selectors in mixins
  (SCSS and Less), bracketed lists, `lib.$x: v`, `scroll-state()`, `attr()` with
  `type()`.
- Completion is empty in comments. The `rgb` snippet text is new.
- Plain CSS `@import` and bare module names resolve in `node_modules`.
- No lint warnings in `:import()` blocks. No symbol for `lib.$x: v`.
- Code actions in place of commands: `{ codeActionKinds: ['quickfix'] }` and
  `doCodeActions2`, in place of `codeActionProvider: true` and `doCodeActions`.

## HTML

From `extensions/html` (0.3.2):

- Server name `vscode-html-language-server`, started with `--stdio`. The
  extension installs `@zed-industries/vscode-langservers-extracted` (latest,
  4.10.8), or uses a `vscode-html-language-server` found on the `PATH` of the
  worktree.
- Language ids: `HTML` → `html`, `CSS` → `css`.
- `initializationOptions`: `{ "provideFormatter": true }`. There is no
  `embeddedLanguages`, so the server default `{ css: true, javascript: true }`
  applies: the JavaScript mode is on. The package has TypeScript 5.9.3 and its
  libraries, but not `lib/jquery.d.ts`: `loadLibrary('jquery')` gives an empty
  file, so `$` has no types. The port also has no jQuery types (VS Code has
  them).
- Workspace configuration: the `lsp.vscode-html-language-server.settings` of the
  user. Without user settings, `workspace/configuration` gets `null` for each
  section. Then upstream gives no diagnostics for a document with CSS
  (`differences/html-server.md`).
- The server is a fork: `zed-industries/vscode`, branch
  `html-tags-as-workspace-edits`, the server of May 2024 plus tag auto
  insertion. With `workspace.workspaceEdit.snippetEditSupport` (Zed sends it),
  each change of one character (`>`, `/` or `=`) in HTML gives a
  `workspace/applyEdit` with a snippet edit (`$0</div>` or `"$1"`).
  `html.tagAutoclosing` other than `true` turns it off. The port has the auto
  insertion. The other changes on `main` since May 2024 that a replay shows:
  `workspace.textDocumentContent` in the `initialize` result, and definitions in
  the library files.
