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
