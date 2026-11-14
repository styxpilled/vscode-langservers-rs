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

- `Element.clone` always makes a plain `Element`, also for a `LabelElement`.
- Dead code, ported as is: `clone()` with children is only reached from itself
  (`cloneWithParent` clones without children), and `segments[0]` of a `&`
  combinator is always empty (the token starts with `&`), so `prepend` is never
  called.
- `:nth-child(n of S)`: the selectors of `S` are parsed again with a new CSS
  parser (also in SCSS and Less), from 2 UTF-16 units after the second token.
- `UnknownAtRule` hover looks up the text of the whole at-rule, so it finds an
  entry only for an at-rule without a body or arguments.
