# CSS hover and selector printing: differences from upstream

Upstream: `vscode-css-languageservice`, `cssHover.ts`, `selectorPrinting.ts`.
Fixture generator: `scripts/css/trace-hover.mjs`.

## Not ported

- An attribute selector without a value (`[o]`) has the value `undefined`.
  `addAttr`, `append` and `prepend` join values as text, so `[o][o]` prints
  `<element o="undefined undefined">`, and `a[o] { &-x { } }` gives
  `o="undefined-x"`. Rust: a missing value is skipped in the join (`[o][o]`
  prints `<element o>`).

## Ported

- `:nth-child(n of S)`: the selectors of `S` are parsed again with a new CSS
  parser (also in SCSS and Less), from 2 UTF-16 units after the second token.
