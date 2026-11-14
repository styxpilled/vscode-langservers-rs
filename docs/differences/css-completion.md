# CSS completion and symbol scope: differences from upstream

Upstream: `vscode-css-languageservice`, `cssCompletion.ts`, `scssCompletion.ts`,
`lessCompletion.ts`, `cssSymbolScope.ts`. Fixture generator:
`scripts/css/trace-completion.mjs`.

## Not ported

- Each new `SCSSCompletion` appends the references to the documentation of the
  static `@use`, `@forward` and `sass:*` items again, so the second language
  service has two `[Sass documentation](...)` links. Rust: once. The recorder
  resets the documentation before each call.
- The `@use` and `@forward` items also carry a `references` field in the
  response. It is not a completion item field. Rust: not sent. The recorder
  removes it.
- `doComplete` throws in some positions (the sweep records `"error"`):
  - At the start of a variable declaration in a block (SCSS `a { |$x: 1 }`):
    `getPropertyProposals` calls `getProperty()` on a `VariableDeclaration`.
    Rust: proposals as for a declaration without a property.
  - In `@content(|)`: `getCompletionsForFunctionArgument` calls
    `getIdentifier()` on a `MixinContentReference`. Rust: no var proposals.
- `findSymbolsAtOffset` keeps names in a plain object, so a symbol named
  `constructor`, `toString`, `__proto__` (and other `Object.prototype` names) is
  never proposed. Rust: a set.
- `Set` (values, colors, custom properties) is a plain object: the key
  `__proto__` is dropped. Rust: kept.
- `getLimitedString` cuts at 140 UTF-16 units, also inside a surrogate pair.
  Rust: the cut is before the pair.
- A start character before 0 (a word longer than the position, only with a
  position past the end of the line) stays negative upstream. Rust: 0.
- The lazy list getters change the tree (`css-parser.md`). The sweep parses
  again for each offset, so the results do not depend on earlier calls.

## Ported

- `Set.getEntries` is `Object.keys`: keys that are array indexes (`5`, `10`)
  come first, in numeric order. This changes the order of the existing values of
  an unknown property.
- `getValueEnumProposals` tags a value as deprecated by the status of the
  property, not of the value.
- Sort text of a property: `(255 - relevance).toString(16)`; a relevance with a
  fraction from custom data gives a fraction in base 16 (`js_to_radix16`).
- `getCompletionsForSelector` changes `currentWord` and the default range for
  `:` and `::`. The rest of the call uses the new values (also the participant
  context of `onCssProperty` for nested rule sets). The SCSS selector functions
  come before the change, so they keep the old range.
