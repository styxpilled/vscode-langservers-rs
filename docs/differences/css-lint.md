# CSS lint and validation: differences from upstream

Upstream: `vscode-css-languageservice`, `lint.ts`, `lintRules.ts`,
`lintUtil.ts`, `cssValidation.ts`. Fixture generator:
`scripts/css/trace-lint.mjs`.

## Not ported

- `visitFunction` compares `Function.getName()` with `rgb(`, `rgba(`, `hsl(` and
  `hsla(`. The name has no `(`, so `argumentsInColorFunction` is never reported.
  Rust: no `visitFunction`. A fixed rule would report false errors for the space
  syntax (`rgb(0 0 0 / 50%)`), so it is not fixed either.
- `validProperties` (in `LintVisitor`) and `NodesByRootMap.data` are plain
  objects.
  - `validProperties['constructor']` is a function, so a property named
    `constructor` (or `toString`, `__proto__`, ...) is never reported as unknown
    or duplicate. Rust: a set of the configured names.
  - A keyframe name or a property suffix `constructor` / `__proto__` makes
    `NodesByRootMap.add` throw (`entry.names` is `undefined`). Upstream
    validation then fails. Rust: normal names.
  - `for (const suffix in data)` visits array-index keys (`"0"`, `"12"`) first,
    in numeric order (`-webkit-0: 1`). Rust: insertion order, as for the data
    manager (`css-language-facts.md`).

## Ported

- `parseFloat` (`checkLineWidth`, zero units): `utils::parse_float`.
