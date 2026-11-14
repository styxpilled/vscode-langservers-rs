# HTML server: differences from upstream

Upstream: `microsoft/vscode` `extensions/html-language-features/server`,
`htmlServer.ts`, `modes/`, `utils/`, `customData.ts`, `requests.ts`, plus the
tag auto insertion of Zed's fork (`zed.md`). The JavaScript mode uses tsrs
(TypeScript 7.1) in place of TypeScript 5 (last section). Tests: `src/tests.rs`,
`src/robustness.rs`, replays (`testing.md`).

## Not ported

- `importedScripts`: upstream does not read them.
- `LanguageModelCache` limits (10 entries, 60 s): each open document keeps its
  parse until it changes.

## Changed

- Custom data: after `updateDataProviders`, the parsed documents are dropped, so
  they use the new void elements. Upstream keeps them in the cache until it
  expires.
- Push diagnostics: as in the CSS server (`css-server.md`), each timer validates
  the current text. A rejected settings request logs and publishes no
  diagnostics.
- File system: as in the CSS server, schemes other than `file` fail in place of
  `fs/readDir` requests.
- `resolveReference` returns no reference where `URI.parse` throws.
- `html.format` settings are read field by field with the semantics of
  js-beautify's `Options`: booleans by truthiness, numbers by `parseInt` (`NaN`
  gives the js-beautify default), lists only from strings, `wrapAttributes` and
  `indentScripts` must be a valid name (else the request answers `[]` and logs,
  as upstream throws). `unformattedContentDelimiter` that is not a string is not
  set. A negative `tabSize` is 4.
- A `css` settings section that is `null` (Zed's `html` extension answers `null`
  for each section) or a `css.lint` that is `null` gives the default lint.
  Upstream passes `null` to `LintConfigurationSettings`, the validation throws,
  and the document gets no diagnostics.
- A quoted attribute value without the closing quote that ends with a character
  outside the BMP: the region drops the full last character. Upstream drops one
  UTF-16 unit and keeps the high surrogate.
- Registration ids are `html-server-registration-<n>`, not UUIDs.
- The initialize result has `serverInfo`.
- Auto insertion (fork): a `globalSettings.html` that is not an object skips the
  insertion (the fork throws and logs).

## Ported

- `getDocumentSettings` asks for the sections `css`, `html`, `javascript`,
  `js/ts` only when the request needs them: completion in a mode, hover in the
  HTML mode (`doHover.length`), validation of a document with CSS, formatting.
  CSS hover always reads the global settings; HTML hover without scoped settings
  reads none.
- `getLanguagesInDocument` stops at three embedded languages and then does not
  add `html`.
- Selection ranges in the HTML mode repeat the innermost range.
- `didChangeConfiguration` with `settings: null` is accepted.
- An `html.format.unformatted` that is truthy and not a string makes formatting
  fail (`[]`, logged).

## JavaScript mode

The results come from tsrs and have the shape of upstream. The reference is the
Node server with the JavaScript mode (`HTML_JS=1`, TypeScript 5.9.3).

### Not ported

- jQuery types (`jquery.d.ts`): Zed's package does not have them either, so `$`
  has no types (`zed.md`). The upstream completion test of `$.getJSON` checks
  that `getJSON` is not offered.

### Changed

- Libraries: tsrs does not load `lib.es2020.full.d.ts` as a `lib` entry; the
  mode lists the libraries it references. The DOM library is that of TypeScript
  7.1, so completions have other global names.
- `moduleResolution` is Bundler (TypeScript 7 has no Classic).
- Library URIs are `html-server://<language>/libs/<file name>`.
- Hover: the markdown of tsrs (signature block with a final line break, tsrs
  JSDoc and tag format), not upstream's parts joined by blank lines. A class
  hover at `new` has only the constructor signature, without the class JSDoc.
  The Zed fork answers a plain string.
- Completion: kinds are the LSP kinds of tsrs (upstream maps the TypeScript kind
  names; the shared labels have the same kinds and sort texts in the replay).
  Items that need an insert text (for example `["a-b"]`) are removed, as
  upstream asks TypeScript not to give them. Order of items can differ. Resolve
  gives the plain text `detail` and `documentation` of tsrs.
- Signature help, document symbols (the navigation tree of tsrs, not
  `getNavigationBarItems`; the order can differ), selection ranges (some inner
  ranges differ), semantic tokens and folding ranges come from the LSP functions
  of tsrs. Folding keeps upstream's `startLine < endLine` filter and kind regex.
- Definition: an empty result is `[]`; TypeScript 5 gives `null` at some
  positions.
- Each language has its own `Session`, created at its first request. The
  `typescript` session is created only for `text/typescript` scripts.

### Ported

- The diagnostic set: syntactic and semantic, without codes 1108 and 2792, all
  `Error`, flattened messages. Module scripts are separate documents.
- `updateHostSettings`, also the error when `js/ts` has no
  `implicitProjectConfig`.
- The formatter: `convertOptions` with the swapped brace settings, initial
  indent, last line. `enabledModes`: the `typescript` mode does not format after
  the HTML pass. The Node reference (the May 2024 server) breaks the script
  formatting of `19813.html` with tabs; the port gives the expected file of
  upstream `main`.
- The word range of completions, the semantic token legend.
- `isValidationEnabled`: `typescript` is off when `html.validate` is set.
