# Design

## Rules

- Port the upstream behavior exactly. Where upstream has a bug, or depends on JS
  behavior that does not make sense in Rust, change it: write a short code
  comment and an entry in `differences/`. Do not port a quirk that does not make
  sense.
- Internal offsets are UTF-8 bytes. Convert to UTF-16 only at the LSP boundary.
- Language service functions return `lsp-types` values, or own types where
  `lsp-types` cannot hold the upstream value.
- No async runtime. A synchronous main loop (`crossbeam_channel::select!`),
  worker threads for HTTP.
- Messages are English only.
- `serde_json` has `preserve_order` (JS key order) and `float_roundtrip` (exact
  float parsing; without it a parsed float can be 1 ULP off).
- Float math can differ from Node in the last bits (V8 math library). Tests
  accept 1 ULP, or 1e-12 after a subtraction. Do not port the V8 math library.
  Values that are rounded for output, and all text, are exact.

## Shared crates

- `crates/text-document`: `TextDocument` (port of
  `vscode-languageserver-textdocument`), with `update` for incremental sync. An
  index of non-ASCII characters makes each UTF-16 conversion a binary search.
- `crates/vscode-uri`: the parts of `vscode-uri` that the services use: `Uri`
  (`parse`, `file`, `with_path`, `fs_path`, `to_string_encoded`,
  `to_string_unencoded`), `Utils` (`join_path`, `dirname`, `basename`,
  `resolve_path`), `posix` (bundled `path.posix`), `resolve_relative_path`,
  `decode_uri_component`.
- `crates/lsp-transport`: stdio threads for the servers, and `run_server`, which
  runs the server loop on a thread with a 256 MiB stack (`experimental.md`,
  "Deep nesting"). Responses and notifications are serialized directly from the
  `lsp-types` values (a `serde_json::Value` takes about 8 times the memory of
  its JSON text).
- `crates/lsp-recorder`: `lsp-recorder` and `lsp-replay` (see `testing.md`).

## JSON

### `crates/jsonc`

Port of `node-jsonc-parser`: scanner, error-tolerant parser, `parse_tree` (arena
`Tree`), `visit` (`Visitor` trait), `get_location`, `format`, `modify`,
`apply_edits`, `strip_comments`. No LSP dependency. The parser is recursive, as
upstream. On `experimental` it reads at most `MAX_DEPTH` levels
(`experimental.md`, "Deep nesting").

### `crates/json-languageservice`

- `parser.rs`: `JsonDocument`, an arena AST (`AstKind`, `NodeId`) with parent
  links. Nodes are 40 bytes: `offset` and `length` are private `u32` (a document
  of 4 GiB or more panics). Child lists are one shared `Vec`, filled after the
  parse from the parent links (`children(id)`). String values are in one shared
  buffer (`AstKind::String(StringSpan)`, `JsonDocument::string`). Duplicate-key
  maps borrow the keys without escapes from the text and are reused from a pool.
  The scanner copies a token value only when it differs from the text.
- Features without schemas are free functions: `folding.rs`,
  `selection_ranges.rs`, `format.rs`, `symbols.rs`, `colors.rs`, `sort.rs`. Sort
  keeps the document as lines, and `jsonc::apply_edits` builds the result in one
  pass. `strings.rs` has the JS string and number helpers (`js_number_to_string`
  uses `ryu-js`).
- Schemas:
  - `schema/tree.rs`: a schema document as a compact read-only JSON tree, built
    from the `jsonc` visitor. Keys keep the document order.
  - `schema/doc.rs`: a loaded document and its index, made in one walk at load:
    the base URI and resource root of each node, the embedded resources
    (`$id`), the anchors and the dynamic anchors.
  - `schema/mod.rs`, `schema/compile.rs`: `Schemas` is a snapshot of the loaded
    documents. A node is compiled into a typed `JsonSchema` on first use
    (`OnceLock`), and its `$ref` is resolved then (`experimental.md`, "Schema
    loading"). `SchemaId` is a document base id plus the node index. Rare
    keywords are boxed (`limits()`, `vscode()`).
  - `schema_service.rs`: `JsonSchemaService` (`fileMatch` globs, documents
    loaded through a synchronous request closure, snapshots). `run` runs an
    operation again after it loads the documents that a `$ref` needed.
  - `validator.rs` (drafts 4, 6, 7, 2019-09, 2020-12), `validation.rs`,
    `vocabularies.rs`, `glob.rs`, `configuration.rs` with `schemas.json`
    (built-in meta-schemas, generated). Regular expressions: `regress`.
- `completion.rs`, `hover.rs`, `links.rs`, and `language_service.rs`
  (`LanguageService`: one schema service, the built-in contributions,
  `configure`, the features that use schemas). `configure` removes all loaded
  schemas.

### `crates/json-language-server`

- `server.rs`: `jsonServer.ts` and `utils/validation.ts`: all requests and
  notifications, client capability checks, settings, push diagnostics (500 ms
  delay) or pull diagnostics, result limits, dynamic formatter registration,
  `runSafe` (a panic gives the error value and a log message).
- `schema_request.rs`: schema content through `file:` (read inline), HTTP
  (`ureq` on a worker thread, with the `request-light` messages and a disk
  cache, `experimental.md`) and
  `vscode/content` (request to the client), as `handledSchemaProtocols` selects.
- Deferred requests: a request for schema content that is not loaded starts the
  fetch and fails. The server discards the result, resets the schema, and runs
  the request or validation again when the content arrives. `$/cancelRequest`
  removes a waiting request. A pull diagnostic does not wait (`experimental.md`,
  "Schema prefetch").
- Schema reload: a `json/schemaContent` or file change removes the loaded
  document of that URI only; the documents that reference it resolve the
  `$ref` again. Compiled `fileMatch` globs are cached across resets.
- Parsed documents are cached for each open document.

## CSS

### `crates/css-languageservice`

- `parser/scanner.rs`: one `Scanner` with a `Dialect` (CSS, SCSS, Less) in place
  of the subclasses. `Token` is `Copy` (type, offset, length, `escaped` flag)
  and has no text. `Scanner::text(&token)` gives the source slice, or for a
  token with escapes, scans the token again and collects the text.
- `parser/nodes.rs`: `Ast` arena.
  - `Kind` has one variant for each upstream class. `node_type()` is the fixed
    type of the class, or the stored type for the classes without a `type`
    getter. `ast.kind(id)` replaces `instanceof`; `Kind::is_body_declaration()`
    and similar replace the abstract classes.
  - `Node` is `Copy`, 48 bytes, no heap data. Offsets are `i32`; the API gives
    `isize` (empty node lists have `-1`). `NodeId` is a `NonZeroU32`. Children
    are a doubly linked list; `ast.children(id)` is an iterator. `Ast::accept`
    walks the links without a stack.
  - Fields of `setNode(field, ...)`: two slots in the node, more in `Extra`.
    `Extra` (side table) also has the rare scalars: issues, `reference_types`,
    colon and semicolon positions, `needs_semicolon`.
  - Getters of the upstream classes are in `impl Ast` (`full_property_name`,
    `numeric_value`, `list_children`, ...). The lazy list getters do not add
    nodes to the tree.
- `parser/mod.rs` (core), `css.rs`, `scss.rs`, `less.rs`: one `Parser`. A method
  that a subclass overrides matches on `dialect`. Each scanned token has an id,
  because `markError` compares tokens by identity.
  - Failed parse attempts upstream leave their nodes to the garbage collector
    (half of all nodes on bootstrap). `Parser::restore_and_discard` (also in
    `try_parse`) and `Parser::discard` truncate the arena. Use them only where
    no caller keeps the id of a dropped node.
  - Token lookaheads skip attempts that must fail: `may_be_ruleset` and
    `may_be_namespace_prefix`. The output does not change.
- `parser/symbol_scope.rs`: `Symbols` (scopes and symbols in arenas).
- `data.rs` and `data/browsers.json` (generated, loaded on first use):
  `CSSDataV1` types, `CssDataProvider`.
- `language_facts/`: `data_manager.rs`, `entry.rs` (`get_entry_description`;
  descriptions for the default hover settings are cached in each entry),
  `colors.rs` (`Color` with `f64`, serialized as JS numbers), generated
  `builtin_data.rs` and `color_data.rs`.
- `services/`: `lint.rs`, `lint_rules.rs`, `lint_util.rs`, `validation.rs`,
  `selector_printing.rs`, `hover.rs`, `completion.rs` (SCSS and Less overrides
  match on the dialect; `CompletionParticipant`), `navigation.rs`,
  `code_actions.rs`, `folding.rs`, `selection_range.rs`, `links.rs`
  (`LinkResolver`, `FileSystemProvider`, `DocumentContext`),
  `path_completion.rs`, `formatter.rs`.
- `beautify.rs`: port of `beautify-css.js` (js-beautify 2.0.3). Tokens are
  slices of the source text. No wrap code: upstream never sets a wrap point.
- The service is synchronous. Where upstream rejects a promise, the Rust
  function returns `None` or an empty result.
- `lsp_types::Color` has `f32` fields. The color requests use the own `Color`
  with `f64`, so `documentColor` values and `colorPresentation` input are exact.
  The JSON service does the same.

### `crates/css-language-server`

- `server.rs`: `cssServer.ts`, `customData.ts`, `runner.ts`, `validation.ts`.
  Same main loop as the JSON server. One `CssDataManager` for the three
  dialects. The `didChangeConfiguration` settings are kept as JSON. Parsed style
  sheets are cached for each open document.
- Scoped settings: completion, hover and validation wait for the
  `workspace/configuration` response of their document. `$/cancelRequest`
  answers a waiting request with `RequestCancelled`.
- `settings.rs`: JSON settings to the service types, with the JS semantics of
  upstream (`parseInt`, truthiness, `braceStyle` lists).
- `fs.rs`: `RequestService` (`nodeFs.ts` with `std::fs`, `file` scheme only) and
  `WorkspaceContext` (`getDocumentContext`).
- A library and a binary: the HTML server uses `fs` and `settings`.

## HTML

### `crates/html-languageservice`

- `parser/scanner.rs`: `Scanner` over a `&str` with byte offsets. The upstream
  regular expressions are hand-written matchers.
- `parser/mod.rs`: `HtmlDocument`, an arena of `Node` (`NodeId`). Index 0 is the
  pseudo node that holds the roots; `find_node_at` and `find_node_before` can
  return it (`is_document`). Attributes are a `Vec` in source order. The scanner
  keeps names as slices of the text; tag names are ASCII, so the parser compares
  them with ASCII case folding. `parser/entities.rs` is generated.
- `language_facts.rs`: `HtmlDataProvider` (`data/html5.json`, generated, loaded
  on first use), `HtmlDataManager` (void elements computed once for each
  `set_data_providers`), `generate_documentation`.
- `services/`: `completion.rs` (`CompletionParticipant`, `do_complete2` with a
  synchronous file system; the participants are arguments, not service state),
  `path_completion.rs`, `hover.rs`, `folding.rs`, `navigation.rs` (highlights,
  symbols, rename, linked editing, matching tag), `selection_range.rs`,
  `links.rs`. File system and document context traits come from
  `css-languageservice` (`services::links`).
- `services/formatter.rs`: `htmlFormatter.ts`. It converts
  `HtmlFormatConfiguration` to `beautify::BeautifyOptions` as js-beautify's
  `Options` does.
- `beautify/`: `beautify-html.js`. `tokenizer.rs` has the `InputScanner`, the
  patterns and the `Tokenizer`; the regular expressions, also the template
  patterns that upstream builds at run time, are hand-written matchers. Tokens
  are a `Vec` with `opened` and `closed` indices; the parser tokens
  (`TagOpenParserToken`) are an arena, the tag stack is a `Vec` of indices.
  `output.rs` is `Output` with line wrapping (UTF-16 counts). The items of all
  lines are one `Vec` of `Cow` slices of the source; a line is a start index.
  Tag names are `Cow` (no copy when lowercase ASCII). `<style>` content goes to
  `css_languageservice::beautify::css_beautify` with `loose_directives` (the
  HTML copy of `beautify-css.js` has a different directive pattern).

### `crates/html-language-server`

- `server.rs`: `htmlServer.ts` and the tag auto insertion of Zed's fork. Same
  main loop and scoped settings as the CSS server; the configuration request has
  four sections. `html/customDataContent` requests go to the client in batches.
- `modes.rs`: the modes (`Mode`: HTML, CSS, `Script`), `formatting.ts`,
  `htmlFolding.ts`, `selectionRanges.ts`, `semanticTokens.ts`. `Document` caches
  the parse, the regions and the embedded CSS, JavaScript and TypeScript
  documents. Requests that wait for settings serialize their result once to a
  `RawValue`.
- `javascript.rs` (feature `javascript`, on by default): `javascriptMode.ts` on
  tsrs (`oxc-project/tsrs`, the Rust port of TypeScript 7). One
  `tsrs_project::Session` for each language (`javascript`, `typescript`),
  created at the first request of that language. The file system is in memory:
  the bundled TypeScript libraries, without jQuery types. As upstream, each
  session has one current file (`/html-server/javascript.js` or
  `typescript.ts`); each request sets its text to the embedded document, and the
  session parses only that file again. Results come as tsrs `lsproto` values;
  `lsproto::json::marshal` makes them `serde_json` values, and the mode changes
  them to the shape of upstream. Diagnostics and formatting use the program and
  `format::format_selection` directly, to get upstream's diagnostic set and
  format settings. tsrs needs Rust 1.99 (`rust-toolchain.toml`).
- `embedded.rs`: `embeddedSupport.ts`. The embedded documents keep the lines and
  UTF-16 columns, so positions go between documents unchanged. Module scripts
  give separate documents for validation.
- `settings.rs`: `html.*` settings with the JS semantics of js-beautify.
- `context.rs`: the HTML `getDocumentContext` (schemes, hierarchical bases).
