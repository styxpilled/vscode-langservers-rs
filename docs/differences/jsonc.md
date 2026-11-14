# jsonc: differences from upstream

Upstream: `node-jsonc-parser`, `scanner.ts`, `parser.ts`, `format.ts`,
`edit.ts`, `main.ts`. Tests: generated from `src/test/` (`gen-jsonc-tests.mjs`).

## Not ported

- Comment token values start one character before the slash, which also breaks
  `stripComments` with a replacement character. Rust: at the slash.
- An unclosed block comment at the end of the text moves past the end. Rust: it
  stops at the end.
- `\u` escapes: an unpaired surrogate stays in the string. Rust: U+FFFD. A
  surrogate pair becomes one character.
- String interning (`string-intern.ts`): a JS optimization.

## Changed

- Offsets are UTF-8 bytes.
- `parse` returns `serde_json::Value`. Whole numbers below 2^53 become integers.
  Infinite numbers become `null`.
- `modify` returns an error where upstream throws or reads past an array.
