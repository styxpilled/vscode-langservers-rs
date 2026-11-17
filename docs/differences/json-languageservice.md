# JSON language service: differences from upstream

Upstream: `vscode-json-languageservice`. Tests: generated tests,
`tests/upstream_trace.rs`, `tests/upstream_features.rs`, the sweep and
JSON-Schema-Test-Suite (`testing.md`).

## Not ported

- `JSONWorkerContribution` (completion and hover contributions), `doResolve`,
  `findDefinition` (upstream returns `[]`). The VS Code server passes no
  contributions.
- The schema prefetch pass: it only starts requests early.
- Completion: a `propertyNames` `enum`, `examples` or `const` value that is not
  a string throws. Rust: ignored.
- Completion: the property lookup sees `Object.prototype` members, so a key such
  as `constructor` finds a function and hides `additionalProperties`. Rust: own
  keys only.
- Completion: `allOf` / `anyOf` / `oneOf` cycles overflow the stack. Rust: they
  stop.
- Completion: a label longer than 60 UTF-16 units is cut at 57 units, also
  inside a surrogate pair. Rust: the half pair is removed.
- A schema field with an unexpected JSON type is used as is, which can throw (a
  non-string `$ref`, `null` as a schema). Rust: ignored. Schema content that is
  not an object becomes `{}`.
- A JSON pointer with invalid percent-encoding rejects the promise. Rust: the
  "can not be resolved" error.

## Changed

- A resolved schema is a compiled snapshot. Upstream keeps the live objects,
  which a later resolution of another schema can still change.
- `doValidation` with an inline schema takes JSON.
- The order of `patternProperties` problems for keys that look like array
  indexes can differ (upstream iterates an object in JS key order).
- Folding ranges of objects and arrays have no kind. Upstream sends the
  non-standard kinds `object` and `array`, which `lsp-types` cannot hold.
- Symbol functions take a result limit and have no limit callback.
- Sort: `icu_collator` for the ICU collation of `localeCompare`. Linear: the
  document is a list of lines. Upstream copies the text for each property.
- `LanguageService` holds only the features that use the schema service.
- Hover: "cursor inside an object or array" uses the start of the last
  character, not the last byte.

## Ported

- Sort swaps reversed line ranges on malformed input, as `substring` does.
