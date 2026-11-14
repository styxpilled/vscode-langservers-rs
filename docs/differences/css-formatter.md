# CSS formatter: differences from upstream

Upstream: `vscode-css-languageservice`, `cssFormatter.ts` and
`beautify/beautify-css.js` (js-beautify 2.0.3). Fixture generator:
`scripts/css/trace-format.mjs`.

## Not ported

- `eatString` appends `null` to the result after a backslash at the end of the
  input (`result += this._input.next()`): `.a { content: "abc\` becomes
  `"abc\null`. Rust appends nothing. The trace patches upstream with `?? ''`.
- `variableOrRule in this.NESTED_AT_RULE` (and `CONDITIONAL_GROUP_RULE`) uses
  `in`, so the `Object.prototype` names (`@constructor`, `@toString`,
  `@__proto__`, ...) are nested at-rules. Rust: only the listed names. The trace
  patches upstream with `Object.hasOwn`.
- `maxPreserveNewLines` and `wrapLineLength` go through `parseInt`, so `1e21`
  and `1e-7` become 1 and `2.5` becomes 2. Rust takes integers; the server
  converts the setting.
- `braceStyle` goes through `_get_selection_list`: a string is split into a list
  (`"collapse,expand"`, the last value wins), and an unknown value throws. Rust
  takes `collapse` or `expand`; the server converts the setting as upstream
  (`css-server.md`).

The test calls give the same result with and without the patches
(`UNPATCHED=1`).

### Dead code

- Line wrapping: `print_string` sets `non_breaking_space` before each token, so
  `__add_space_before_token` never sets a wrap point. `wrapLineLength` has no
  effect.
- With tabs, an indent size of 1 becomes 4. The indent is one tab for each
  level; the size only changes the wrap character counts.
- Alignment, raw tokens, `eol` other than `\n` (`cssFormatter.ts` always passes
  `\n`).

## Ported

- The leading spaces and tabs of the text (of the range, for range formatting)
  are the base indent of every line.
- `\r\n` and `\r` become `\n`; the edit text has `\n` only.
- Range formatting: the range extends over the spaces and tabs before it (all of
  them at the start of a line, else all but one) and after it (only up to the
  end of a line). `isInRule` looks back from the start offset, that character
  included. Inside a rule, the text gets a `{\n` prefix, and the first two
  UTF-16 units of the result are removed.
- `lookBack` ignores ASCII case (`URL(`).
- `foundNestedPseudoClass` starts one character after the character that follows
  the colon: `h:(i) {` is a property, not a nested pseudo-class.
- `get_code` drops an empty last token (an empty variable name, `$ `).
- `max_preserve_newlines` default is 32786. One newline is always kept, also
  with `preserveNewLines: false`.
- The ignore directive end (`/*\sbeautify\signore:end\s*/`) accepts any JS white
  space character; the start must be in a comment that matches
  `/* beautify( \w+:\w+)+ */`, and the last ` ignore:` value in the comment
  wins.
- A reversed range: the text between the offsets (`substring` swaps them).
