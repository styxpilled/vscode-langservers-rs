# CSS language facts: differences from upstream

Upstream: `vscode-css-languageservice`, `src/languageFacts/`. Fixture generator:
`scripts/css/trace-facts.mjs`. It has no inputs for the quirks that are not
ported; `tests/language_facts.rs` (`quirks_not_ported`) tests the Rust behavior.

## Not ported

- `dataManager.ts` keeps entries in plain objects (`_propertySet` etc.).
  - `constructor` and `__proto__` are on the prototype. An entry with this name
    is never added, and `isKnownProperty` / `isStandardProperty` are `true` for
    them. Rust: normal names.
  - `Object.values` returns array-index names (`"2"`, `"10"`) first, in numeric
    order. Rust: insertion order.
- `colors.ts`, `isColorValue` / `getColorValue`: `colors[name]` also finds
  `constructor` and `__proto__`, so `isColorValue` is `true` for them (and
  `getColorValue` is `null`). Rust: not a color.
- `colorFunctionNameRegExp` has the `iu` flags: `ſ` (U+017F) matches `s` and the
  Kelvin sign (U+212A) matches `k`, so `hſl()` is a color constructor without a
  color value. Rust: ASCII case only.
- `getAngle` matches the unit without case, but the `switch` compares it
  exactly: `0.5TURN` is 0.5 degrees. Rust: units without case.
- `xyzFromLAB` loops over all keys of its result, so the cube transform also
  changes `alpha` (0.5 becomes 0.125). Only the direct result has it;
  `colorFromLAB` uses the input alpha. Rust: alpha unchanged.
- The V8 math library. V8 13.6 (Node 24) `Math.pow` is close to the platform
  `pow` (about 0.2% differ by 1 ULP), `atan2` is fdlibm, `cbrt`, `sin`, `cos`
  match neither. Rust uses `std` (`docs/experimental.md`). V8 `main` moved all
  of them to llvm-libc. The fixture test accepts 1 ULP, or less than 1e-12 for
  the differences that a subtraction makes larger. Color values of nodes
  (rounded to 0..255) and all text are exact.

## Changed

- Custom data: an entry with a field of the wrong JSON type is dropped. Upstream
  keeps each entry with a string `name`.
