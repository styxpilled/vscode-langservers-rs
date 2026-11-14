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

- `hexColorLength`: the parser makes a `HexColorValue` only for 3, 4, 6 or 8 hex
  digits, so the rule is never reported. Ported as is (no cost).
- `visitNumericValue` looks up `validProperties` with the full property name as
  written (not lowercase); the other checks use the lowercase name.
- `getNonPrefixedPropertyName` is not lowercase; the standard property names in
  the same map are. `-WEBKIT-Transition` and `transition` have different keys.
- `duplicateProperties` reports a declaration once for each other declaration of
  the same name.
- `parseFloat` (`checkLineWidth`, zero units): `utils::parse_float`.
