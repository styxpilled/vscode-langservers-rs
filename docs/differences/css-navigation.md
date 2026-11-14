# CSS navigation, code actions, folding, selection ranges: differences from upstream

Upstream: `vscode-css-languageservice`, `cssNavigation.ts` (parts without a file
system), `cssCodeActions.ts`, `cssFolding.ts`, `cssSelectionRange.ts`. Fixture
generator: `scripts/css/trace-navigation.mjs`.

## Not ported

- `difference` (`utils/strings.ts`, used for the "Rename to" quick fixes) pushes
  the same array for each row of the LCS matrix. A character can then match more
  than once, so the score can be higher than the real common subsequence:
  upstream suggests `bleed`, `fill`, `flex` for `rule`. Rust: one array for each
  row (`flex`, `left`). The sweep patches upstream `difference` in the same way;
  the test calls of upstream give the same result with both. The upstream
  behavior is commented out in `difference` (`code_actions.rs`); with it, the
  sweep without the patch matches. Effect (one-letter deletions of all built-in
  property names): the first suggestion is the same in 14532 of 14600 cases; the
  second and third differ in about one third of the cases.

## Ported

- `findDocumentSymbols` for `@scope`: if the first child is not a `ScopeLimits`,
  that child is still the name node (only for the selection range, and only if
  it is in the range).
- An empty symbol name becomes `<undefined>`.
- `toTwoDigitHex` adds a `0` only when the hex text does not have two digits: a
  component above 1 gives three digits, a negative one gives `-1` and the like.
- `getColorPresentations` writes the `lab`, `lch`, `oklab` and `oklch` alpha
  with `${}`; the alpha is never `undefined` there.
- Folding: a `#region` comment must have its text on one line
  (`\s*(.*?)\s*\*\/`); a region comment with a line break inside the text is a
  plain comment range. SCSS and Less also match `// #region`. The scanner comes
  from `document.languageId`, not from the parse.
- Folding: `rangeLimit` 0 means no limit.
- Selection ranges: a node with the same range as its parent is skipped.
- `localeCompare` sorts the quick fixes with equal scores (ICU, as for JSON
  sort). For the built-in property names (lowercase ASCII and `-`) this is the
  byte order.
