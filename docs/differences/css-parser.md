# CSS scanner and parsers: differences from upstream

Upstream: `vscode-css-languageservice`, `cssScanner.ts`, `scssScanner.ts`,
`lessScanner.ts`, `cssParser.ts`, `scssParser.ts`, `lessParser.ts`,
`cssNodes.ts`. Fixture generators: `scripts/css/trace-scanner.mjs`,
`trace-parser.mjs`, `gen-parser-sweep.mjs`, `gen-nested-sweep.mjs`.

## Not ported

- A hex escape keeps the low 16 bits (`String.fromCharCode`). A result in the
  surrogate range stays a lone surrogate (two escapes join to a pair). Rust:
  U+FFFD.
- The unit of a dimension is looked up in a plain JS object, so `1constructor`
  and `1__proto__` get a non-token type. Rust: `Dimension`.
- The lazy list getters (`getArguments()` and others) add an empty `Nodelist` to
  the tree. Rust does not change the tree.

## Changed

- Nodes of failed parse attempts are dropped from the arena (upstream leaves
  them to the garbage collector). No caller sees them.
- Token lookaheads skip parse attempts that must fail. The output is the same:
  - `may_be_ruleset` (CSS dialect, before `try_parse_ruleset`): before a `(` or
    `[`, the selectors and the commas between them consume only `Ident`,
    `Delim`, `Hash`, `Colon` and `Comma` tokens. If the first other token is not
    `{`, `(` or `[`, the attempt fails. Its errors are never on the first token,
    and `last_error_token` is the only state that a failed attempt keeps, so the
    skip has no effect on the errors.
  - `may_be_namespace_prefix` (CSS and Less): a prefix is `|`, or `*` or an
    identifier (one token outside SCSS) before a `|`.

## Ported

- A `\` at the end of the text is an escape of NUL: the token ends one past the
  text. Slices of the text clamp.
- `markError` compares tokens by identity: each scanned token has an id.
