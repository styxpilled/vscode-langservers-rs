# CSS server: differences from upstream

Upstream: `microsoft/vscode` `extensions/css-language-features/server`,
`cssServer.ts`, `utils/validation.ts`, `requests.ts`, `node/nodeFs.ts`,
`utils/documentContext.ts`, `customData.ts`. Tests:
`crates/css-language-server/tests/stdio.rs`, unit tests. The 4.10.0 server in
Zed differs only in the code actions (`zed.md`).

## Not ported

- Push diagnostics: after the `await validate(...)`,
  `delete pendingValidationRequests[uri]` also deletes a newer timer that a
  change during the await set. That timer then sees
  `request !== pendingValidationRequests[uri]` and does not validate: the
  diagnostics stay stale until the next change. Rust: each timer validates the
  current text when it fires (and again when its settings arrive). A change
  during the wait can give one extra publish with the same content.
- `handledSchemas` that is a string is iterated by character (no handler). Rust:
  a non-array value is the default `['file']`.
- A negative `tabSize` builds broken indents (or throws in `new Array`). Rust:
  4, as for 0.

## Changed

- Schemes without a built-in handler (all except `file`): upstream sends
  `fs/stat`, `fs/readDir`, `fs/content` to the client. Rust: the call fails, as
  when the client rejects it. The language service is synchronous; Zed uses only
  `file` URIs. Custom data from other schemes gives an empty provider.
- `resolveReference` throws when `URI.parse` rejects the base. Rust: returns no
  reference.
- Settings are read field by field with the JS semantics that the services use
  (`validate === false`, `?? true` then truthiness for the completion flags,
  `!== false` for hover). `importAliases` values that are not strings are
  dropped (upstream joins them as paths and can throw).
- The initialize result has `serverInfo`.

## Ported

- `getDocumentSettings` caches the `workspace/configuration` promise for each
  document until the next `didChangeConfiguration` or `didClose`. A rejected
  request (or a `null` result: `s[0]` throws) stays rejected: completion and
  hover answer `null`, pull diagnostics an empty report, push diagnostics log an
  error. A request that waits for the settings uses the response even if the
  cache was reset meanwhile.
- `settings[0]` that is `undefined` (an empty result array) selects the
  `configure` settings of the language service; `null` selects no settings.
- `didChangeConfiguration` with `settings: null` throws: logged, no change.
- `braceStyle` follows `beautify-css.js`: a list (string split on runs of other
  characters, or an array), the last item wins, only `expand` is not `collapse`,
  and an unknown item throws (the request answers `[]` and logs). `tabSize` and
  `maxPreserveNewLines` go through `parseInt`.
- `foldingRange.rangeLimit` and `rangeFormatting.editLimit`: `0` and non-numbers
  are `Number.MAX_VALUE`.
