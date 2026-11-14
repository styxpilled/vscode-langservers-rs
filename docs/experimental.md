# Experimental branch

The `experimental` branch does not have to match the JS behavior. Where a quirk
of upstream needs extra logic or costs performance, the branch removes it. Each
change below gives what upstream does, what the branch does, why, and the
measured effect. Replays against Node show only the differences of this file and
of `testing.md`.

`docs/differences/` keeps the differences of `main` that the branch also has.

## CSS completion and language facts

- Baseline status. Upstream writes `Baseline since undefined` when an entry has
  no `baseline_low_date`. The branch leaves out the parenthesis. The text is
  visible in hover and completion documentation. `tests/language_facts.rs`
  removes the text from the Node values.
- Deprecated values. Upstream `getValueEnumProposals` tags each value with the
  status of the property. The branch uses the status of the value. In the
  built-in data, 308 values have a status other than their property (for example
  `clip: auto`, `clip` is obsolete). The sweeps do not complete these values;
  `tests/completion.rs` (`deprecated_tag_of_values`) tests them.
- Existing values of an unknown property. Upstream collects them in an object,
  so `Object.keys` puts array-index keys (`1`, `10`) first. The branch keeps
  source order in an `IndexSet`, without the partition and sort. Replay
  (`css/features.jsonl`): one response, `1px` before `1` in `example.scss` 37:6.
  `tests/completion.rs` (`existing_values_in_source_order`).
- Property sort text. Upstream writes `(255 - relevance).toString(16)`, which
  has base-16 fraction digits for a relevance with a fraction (custom data). The
  branch rounds to an integer before it writes the hex digits. Built-in
  relevances are integers: no replay difference.

## CSS hover

- Hover of an unknown at-rule. Upstream looks up the text of the `UnknownAtRule`
  node. The parser finishes this node only at the end of the text; after `;` or
  `}` it covers only the at-keyword, so the lookup fails only for an at-rule at
  the end of the text without `;` or `}`. The branch looks up the name child,
  with the name as range, and only when the position is in the name. Six
  built-in at-rules parse as unknown (`@counter-style`, `@document`,
  `@font-feature-values`, `@font-palette-values`, `@position-try`,
  `@view-transition`). `tests/hover.rs` (`unknown_at_rule_by_name`).
- Kept: the parser does not finish an unknown at-rule after `;` or `}`. With the
  node over the whole at-rule, document highlights and rename find the at-rule
  from a position in its body and offer its whole text.
- Selector printing. Upstream `Element.clone` makes a plain `Element` from a
  `LabelElement`, so the printer also tests for the name `…`. The branch keeps
  the kind and tests only the kind: an element with the escaped name `\2026`
  prints as `<…>`, not as a label (`element_named_ellipsis`). Dead code is
  removed: `clone` with children (only `cloneWithParent` calls `clone`, without
  children) and `prepend` (`segments[0]` of a `&` combinator is always empty).

## CSS lint

- Duplicate declarations. Upstream reports a declaration once for each other
  declaration of the same name: three `color` declarations give six diagnostics,
  in O(n²). The branch counts the names and reports each declaration once.
  `tests/lint.rs` (`duplicate_properties_once`).
- Hex color length. Upstream makes a `HexColorValue` only for 3, 4, 6 or 8 hex
  digits, so `hexColorLength` is never reported, and `#12345` gives the parse
  error "property value expected". The branch makes a `HexColorValue` for any
  run of hex digits; lint reports "Hex colors must consist of three, four, six
  or eight hex numbers" (in SCSS and Less variables too; a Less variable had no
  diagnostic). A hex value of another length is not a color for completion or
  color decorators. `hex_color_length`. The parser and lint sweeps skip inputs
  with such a value (parser: 434 and 86 parses, the parses that differ; lint:
  626 inputs); the lint sweep merges the repeated duplicate entries of upstream.
- Lowercase names. Upstream looks up `validProperties` for `zeroUnits` with the
  name as written, and `getNonPrefixedPropertyName` is not lowercase, so
  `-WEBKIT-Transition` asks for the standard property `Transition`. The branch
  uses the lowercase name in all lookups. `lowercase_names`.

## CSS colors

- Hex presentation. Upstream `toTwoDigitHex` adds a `0` only when the hex text
  does not have two digits, so a component above 1 gives three digits and a
  negative one `-1` and the like (`#017f0-7f80`). The branch clamps each
  component to `00`..`ff`. `js_to_radix16` is removed. `tests/navigation.rs`
  (`hex_presentation_clamps`).
- Rounding. Upstream uses `Math.round` (a half rounds up), `toFixed` (a tie
  rounds away from zero) and `Math.round((v + Number.EPSILON) * 100) / 100`. The
  branch uses `f64::round` and one `round_to(v, digits)`. `js_round` and
  `js_to_fixed` are removed.
- Trigonometry. Upstream gets `cbrt`, `atan2`, `sin` and `cos` from V8 (fdlibm);
  `main` uses the `libm` crate for the same results. The branch uses `std`; the
  `libm` dependency is removed.
- Alpha. The `Lab` and `Lch` alpha is `f64`, not `Option<f64>`: it is never
  `undefined`.
- Effect: the presentations of 281216 colors (each component in steps of 5,
  alpha 1 and 0.5) are the same as on `main`. The color fixtures and the
  navigation and facts sweeps pass.

## CSS server settings

The HTML server uses the same helpers for its formatter settings.

- Failed settings requests. Upstream caches the `workspace/configuration`
  promise; a rejected request, or a `null` result (`s[0]` throws), stays
  rejected until the next `didChangeConfiguration`: completion and hover answer
  `null`, diagnostics are empty. The branch uses the `configure` settings for
  that request and does not cache the result, so the next request asks again.
  `tests/stdio.rs` (`scoped_settings`).
- Unknown `braceStyle`. Upstream js-beautify throws on an unknown item, and
  formatting answers `[]`. The branch ignores unknown items; the last known item
  wins. `formatting`, `brace_styles`.
- Integers. Upstream reads `tabSize`, `maxPreserveNewLines` (and the HTML
  `wrapLineLength`, `maxPreserveNewLines`) with `parseInt`: `"12px"` is 12, and
  a number with an exponent in `String(n)` gives its first digit (`1e21` is 1).
  The branch takes a number, or a string that is only a number; a fraction is
  cut. `integers`.

## CSS formatter

- Nested pseudo-class. Upstream `foundNestedPseudoClass` starts its search one
  character after the character that follows the colon, so `h:(i) {` in a rule
  is a property (`h: (i) {`). The branch starts at the character after the
  colon. `tests/formatter.rs` (`nested_pseudo_class_after_colon`). The format
  sweep skips the texts with a colon before `(`, `)`, `{`, `}` or `;` (178 of
  24700 results; 17 of them differ).
- Kept: one newline after `property:` also with `preserveNewLines: false`. This
  is the intended "allow at least one new line" of js-beautify: the value stays
  on the next line.

## CSS code actions

- Quick fix order. Upstream sorts the "Rename to" fixes with equal scores with
  `localeCompare` (ICU). The branch uses byte order. For the built-in property
  names (lowercase ASCII and `-`) the two orders are the same: no sweep or
  replay difference. The `icu_collator` dependency is removed.
- `difference` compares characters, not UTF-16 units: a character outside the
  BMP counts once.

## HTML language facts

- Void elements. Upstream `isVoidElement` does a binary search with
  `localeCompare` (ICU) in the list that `sort()` ordered by UTF-16 units: for a
  name outside `[a-z0-9-]` the two orders can differ and the search can fail.
  The branch sorts by bytes and searches by bytes. The built-in void elements
  are `[a-z]`: no sweep or replay difference. The `icu_collator` dependency is
  removed.

## HTML service

`tests/experimental.rs` tests these changes.

- Path completion. Upstream takes the replace range from the value range with
  `shiftRange(range, 1, -1)` and shifts the end character by the length of the
  name after the last `/`. The shift stays on one line, and it assumes two
  quotes: a name on two lines gives a negative character and upstream throws
  (no completion); an empty value gives a reversed range; an unclosed quote
  loses the last character. The branch takes the value without its quotes as
  byte offsets and makes the range from offsets. `do_complete2` always answers.
  The feature sweep skips path completion in a value that is not quoted on one
  line (137 results, 13 of them where upstream throws). `path_completion_ranges`.
- Region markers. Upstream `/^\s*#(region\b)|(endregion\b)/` has no `^` for
  `endregion`, so a comment with `endregion` anywhere ends a region. The branch
  uses `/^\s*#(end)?region\b/`. The JSON folding has the same pattern after
  `//` and the same change. `endregion_only_at_start`.
- Entity hover. Upstream writes the first UTF-16 unit of the character
  (`U+D835` for `&Afr;`). The branch writes the code point (`U+1D504`).
  `entity_hover_code_point`.
- Simplifications without an effect on results:
  - The `type` value of a quoted attribute without closing quote kept all but
    its last character. Without the closing quote the value goes to the end of
    the text, so no script content follows. The branch keeps all of it.
  - `startTagEnd` and `endTagStart` were JS conditions (0 is false). Only an
    orphan end tag at offset 0 has `endTagStart` 0, and the service does not
    make orphan nodes.
  - Symbol names: a node without a tag has no attributes (the scanner skips to
    `>`), so upstream `undefined#id` cannot occur. The branch does not write
    `undefined`.

## HTML server

- Failed settings requests. As in the CSS server: upstream caches a rejected
  `workspace/configuration` request (or a `null` result) until the next
  `didChangeConfiguration`, and completion, hover, formatting and diagnostics
  fail. The branch uses the global settings for that request and does not cache
  the result. `tests/stdio.rs` (`scoped_settings`).
- `html.format.unformatted` that is truthy and not a string. Upstream js-beautify
  throws (`unformattedTags.match`), and formatting answers `[]`. The branch
  ignores the value. `scoped_settings`.
- Languages in a document. Upstream `getLanguagesInDocument` stops at three
  embedded languages and then does not add `html`: a document with CSS,
  JavaScript and TypeScript regions has no HTML mode for links, symbols,
  colors, semantic tokens and diagnostics. The branch has no limit.
  `embedded::tests::languages_in_document`.
- Selection ranges. In the HTML mode, upstream adds the HTML ranges as parent of
  the same HTML ranges, so the innermost range repeats. The branch returns the
  HTML ranges once. `tests::selection_html`. Replay: 6 responses each in
  `html/features.jsonl` and `html/push.jsonl`.

## HTML formatter

- Line lengths. Upstream counts the line lengths for wrapping and the attribute
  alignment in UTF-16 units: a character outside the BMP counts twice. The
  branch counts characters. `tests/formatter.rs` (`line_length_in_characters`).
  The format sweep skips the texts with a character outside the BMP (68 of 15406
  results; 19 of them differ). Effect on `html-perf.jsonl` (ASCII): formatting
  60 to 66 ms against 60 to 61 ms, the same within noise.
