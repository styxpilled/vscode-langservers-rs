# Experimental branch

The `experimental` branch does not have to match the JS behavior. Where a quirk
of upstream needs extra logic or costs performance, the branch removes it. Each
change below gives what upstream does, what the branch does, why, and the
measured effect. Replays against Node show only the differences of this file and
of `testing.md`.

`docs/differences/` keeps the differences of `main` that the branch also has.

## Deep nesting

Upstream parsers and tree walks are recursive. Node throws `RangeError` at about
3000 levels, and the request fails. In Rust a stack overflow stops the process
(`catch_unwind` does not catch it): before this change, 3000 levels of `[`,
`a{`, `(` or `<div>` (6 to 15 KB of text) stopped each server.

- Server thread. Each server runs its loop on a thread with a 256 MiB stack
  (`lsp_core::run_server`). This is address space; the system commits pages only
  on use.
- Nesting limits, 1000 levels. Deeper parts are cut, so the walks after the
  parse also have a bounded depth:
  - JSON (`jsonc::MAX_DEPTH`, in `jsonc` and in the `JsonDocument` parser): a
    deeper array or object is empty and covers the text to its closing bracket.
    Error "Nesting too deep" (code `0x211`; `jsonc`:
    `ParseErrorCode::NestingTooDeep`).
  - CSS (`parser::MAX_DEPTH`): a recursion guard (`Parser::nested`) on the parse
    functions where each recursion passes: declarations (blocks), expressions
    (parentheses, function arguments, interpolations), selectors (pseudo-class
    arguments), boolean expressions, media and supports conditions, SCSS list
    elements. The links of a binary expression chain and of an SCSS `@else if`
    chain count too. At the limit the function gives the error
    `css-nestingtoodeep`, skips to the closing bracket of the enclosing group,
    and returns nothing; the error recovery of the caller follows. A binary
    chain stops; the rest reads as more terms (`+1` is a term with a sign). The
    selector hover tree nests at each combinator; it stops at the limit.
  - CSS skips in the deep part. Restores make the parser come back to the
    skipped text from each outer level. So at the first skip, one scan builds
    the end of each bracket group, and a skip jumps over each inner group; when
    the enclosing group is not closed, the text ends for the parser at the skip.
    Without this, 8000 levels of closed `:not(` took 1.1 s (now 6 ms).
  - CSS cost on usual documents: the guard is a counter at each call of these
    functions, not at each token. A first version counted the brackets at each
    token: parse 4% slower on bootstrap, so it was replaced.
  - HTML (`parser::MAX_DEPTH`): at the limit, a start tag first closes the
    deepest open element (as browsers do; Chromium stops at 512). No diagnostic:
    the HTML mode has no parse diagnostics.
- Effect: all features except formatting answer on 100000 levels of 64 generated
  documents (each nesting construct of the three servers, open and closed;
  `scripts/deep-nesting.mjs`), also with a 4 MiB stack: 256 MiB is a margin of
  64 times. The slowest take 2 to 4 s (600 KB of text with up to 100000
  diagnostics) and grow linearly. Formatting is not limited: its output grows as
  the square of the depth (indentation), as upstream. `tests/stdio.rs` of each
  server (`deep_nesting`), `jsonc` `nesting_limit`, HTML `nesting_limit`.
- Nested symbols (`documentSymbol` as `DocumentSymbol`, all three servers): a
  client that reads JSON with the default `serde_json` recursion limit (128
  levels) cannot read a response with symbols 62 levels deep, and loses the full
  response. The symbols below `lsp_core::MAX_SYMBOL_DEPTH` (50) become children
  of their ancestor at that depth, in document order. No symbol is lost.
  `lsp-core` `symbol_depth`; `tests/stdio.rs` of each server.
- Not limited: the JavaScript mode (tsrs, one more document). No overflow up to
  6000 levels, but its time grows as the cube of the depth (3000 levels: 1.9 s,
  6000: 14.5 s, 12000: more than 60 s).
- Clients: `serde_json` reads 128 levels by default, so the test clients cannot
  read a nested result (hierarchical symbols, selection ranges) deeper than
  that. Upstream gives the same results. Not checked in Zed.

## Result limits

Upstream servers limit results only when the client gives a limit. The VS Code
client gives 5000 symbols (`json.maxItemsComputed`), 5000 folding ranges
(`editor.foldingMaximumRegions`) and 500 colors (`editor.colorDecoratorsLimit`).
Zed gives none. The branch uses these values when the client gives none
(`lsp_core::DEFAULT_RESULT_LIMIT`, `DEFAULT_COLOR_LIMIT`): folding ranges in all
three servers; symbols and colors in the JSON server, as only its upstream has
these settings. Zed shows no notice when a result is cut. `tests/stdio.rs` of
each server (`default_limits`, `default_folding_limit`). Effect on `perf.jsonl`
(33 MB): JSON `documentSymbol` 2540 ms against 35 ms, `foldingRange` 91 ms
against 55 ms, peak memory 636 MB against 315 MB.

## Completion lists

Upstream sends each completion item complete: `textEdit` with the range,
`insertTextFormat`, `documentation` and the retrigger `command`. A CSS property
list is about 400 KB, and 73% of it is documentation. When the client declares
support (`completionList.itemDefaults`, `completionItem.resolveSupport`), the
CSS and HTML servers send smaller lists (`lsp_core::completion`):

- `itemDefaults.editRange`: the range of the service (CSS), or the most common
  item range (HTML). An item with this range has `textEditText` in place of
  `textEdit`, and no text when the text is the label.
- `itemDefaults.insertTextFormat`: snippet when most items are snippets; the
  other items give their format.
- `documentation`, `detail` and `command` come with `completionItem/resolve`.
  The server keeps them for the last list; `data` is `[generation, index]`. A
  resolve for an older list gives the item unchanged. Zed resolves the selected
  and the visible items. Zed does not run the retrigger command (it runs only
  the commands of `executeCommandProvider`).
- Empty `tags` are left out.

Script items (JavaScript mode) do not change. `lsp-core` tests, `tests/stdio.rs`
(`compact_completion`) of both servers. Effect on the Zed sessions: CSS 96
completions 38.2 MB against 4.2 MB (average 398 KB against 44 KB, largest 1157
KB against 119 KB), 317 ms against 101 ms in `lsp-replay`; HTML 121 completions
2.40 MB against 0.13 MB.

Not done: serialization of constant item lists once. A CSS completion now takes
about 1 ms with the transfer.

## Pull diagnostics

Upstream validates the document at each `textDocument/diagnostic` and sends a
`full` report without `resultId`. Zed pulls 50 ms after the last edit
(`lsp_pull_diagnostics.debounce_ms`), and also on other events: 196 of 925 JSON,
98 of 392 CSS and 190 of 646 HTML pulls in the Zed sessions come when the
document and the settings did not change.

The branch (`lsp_core::server`, all three servers):

- Report cache. The server keeps the last report of each open document with a
  `resultId`. A change of the text drops it. A configuration change, a schema
  change and a custom data change increment a generation, and a report of an
  older generation is validated again. A pull with the `resultId` of a current
  report gets `{ "kind": "unchanged" }`; Zed then keeps its diagnostics. When a
  new validation gives the same items, the `resultId` stays. A pull that waits
  for `workspace/configuration` keeps the generation of the request, so settings
  that the client sent before a configuration change do not give a current
  report. CSS and HTML: an unchanged report needs no settings. Custom data
  (`css/customDataChanged`, `html/customDataChanged`) increments the generation
  but sends no refresh, as upstream.
- Idle validation. After `didOpen` and `didChange`, the server validates the
  document when no message waits, so the pull is a lookup. One document for each
  idle turn. CSS and HTML validate only when the settings of the document are
  known; JSON only when its schemas are loaded (else the schema prefetch loads
  them). The work is on the main thread: a request that arrives during an idle
  validation waits for it.
- JSON: the syntax-only report of a pull without its schema has no `resultId`.

Push diagnostics do not change. `tests/stdio.rs`: `diagnostic_result_ids`
(JSON), `scoped_settings` (CSS). Zed facts: `zed.md`.

Effect. On the Zed sessions in `lsp-replay`: diagnostic response bytes JSON 1054
against 789 KB, CSS 338 against 231 KB, HTML 55 against 47 KB; the same results,
request times equal (the replay sends the pull at once after the change). A
change on a large JSON document, then a pull or a hover after a pause
(`scripts/pull-latency.py`, median of 6):

| Document                    | Pause  | Pull, no idle work | Pull, idle validation |
| --------------------------- | ------ | ------------------ | --------------------- |
| `large-file.json` (26 MB)   | 0 ms   | 107 ms             | 109 ms                |
| `large-file.json`           | 50 ms  | 84 ms              | 58 ms                 |
| `large-file.json`           | 200 ms | 85 ms              | 0 ms                  |
| `large-pretty.json` (33 MB) | 50 ms  | 97 ms              | 83 ms                 |

A hover has the same times: the idle work parses the document. From the recorded
time between each change and the next pull, the idle validation is complete
before 684 of the 686 JSON pulls that follow a change. CSS and HTML validation
takes less than 1 ms.

## CSS parse time

Upstream tries some parse functions again at the same offset. In nested input
each level is then parsed two or three times, and the time is exponential in the
depth: `:not(` 22 times takes 3.4 s in Node, 30 times stops the server for
minutes; the same for SCSS `#{` in a value (a term tries a module member, a
function and an identifier, each with the full interpolation). The parser now
keeps the offsets (with `attr_depth`) where `try_as_selector`, `parse_function`
and `parse_module_member` failed after more than two tokens, and fails at once
there again. The result of these functions depends only on the position. Most
calls fail on the first or second token: they do not go into the set, and the
set is empty in usual documents (a first version kept each failure: parse 22%
slower on bootstrap). Effect: 30 levels in 10 ms, 1000 levels in about 0.1 s.
Parse results do not change: the parser fixtures, the sweeps and the replays
pass. `tests/stdio.rs` (`deep_nesting`).

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

## CSS links and path completion

Upstream is async, and a throw rejects the promise: `findDocumentLinks2` gives no
links and `doComplete2` no completion (the servers answer `[]` and `null`).

- Links. A link whose resolution throws (an invalid URI in `vscode-uri`, for
  example `gantry-media://g.png`) is left out; the other links stay. In
  `package.json`, content that is not an object and an entry that is not a
  string (`"sass": 5`, `"exports": { ".": { "sass": {} } }`) count as absent,
  and the resolution goes on as without them. `links_where_upstream_rejects`.
- Path completion range. Upstream takes the value range with
  `shiftRange(range, 1, -1)` and moves the end by the length of the name after
  the last `/`. This assumes one line and two quotes: a value on two lines (an
  escaped newline) gives a negative character and upstream throws; an unclosed
  quote loses the last character. The branch takes the value as byte offsets of
  the text, without its quotes, as the HTML service does. A file name that
  `joinPath` rejects is not compared with the document.
- Path completion position. The completion also gives a URL context when the
  cursor is next to the value: on `url` (`u|rl("a/b")`), before `url(`, or
  after the `)`; the context range is then not the value. Upstream completes
  there too, with the text before the cursor from the value and a range away
  from the cursor (a cursor at the opening quote or after the closing quote
  takes the whole value). The branch completes paths only when the context
  range is the value and the cursor is between the quotes.
  `path_completion_offsets`.

The links sweep skips the calls that upstream rejects and the path completions
where the branch does not complete or where upstream computes the range on one
line with two quotes (738 of 15354 calls). Replay: the generated sessions
`css/features.jsonl` and `css/push.jsonl` complete at each offset, and differ at
the positions next to a `url(...)` or an import value.

## CSS server settings

The HTML server uses the same helpers for its formatter settings.

- Failed settings requests. Upstream caches the `workspace/configuration`
  promise; a rejected request, or a `null` result (`s[0]` throws), stays
  rejected until the next `didChangeConfiguration`: completion and hover answer
  `null`, diagnostics are empty. The branch uses the `configure` settings for
  that request and does not cache the result, so the next request asks again.
  `tests/stdio.rs` (`scoped_settings`).
- `settings: null` in `didChangeConfiguration`. Upstream throws (reading `css`)
  and keeps the last settings. The branch takes it as no settings, as the HTML
  server does. `tests/stdio.rs` (`push_diagnostics`).
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
  quotes: a name on two lines gives a negative character and upstream throws (no
  completion); an empty value gives a reversed range; an unclosed quote loses
  the last character. The branch takes the value without its quotes as byte
  offsets and makes the range from offsets. `do_complete2` always answers. The
  feature sweep skips path completion in a value that is not quoted on one line
  (137 results, 13 of them where upstream throws). `path_completion_ranges`.
- Region markers. Upstream `/^\s*#(region\b)|(endregion\b)/` has no `^` for
  `endregion`, so a comment with `endregion` anywhere ends a region. The branch
  uses `/^\s*#(end)?region\b/`. The JSON folding has the same pattern after `//`
  and the same change. `endregion_only_at_start`.
- Entity hover. Upstream writes the first UTF-16 unit of the character (`U+D835`
  for `&Afr;`). The branch writes the code point (`U+1D504`).
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

- Document symbols. Upstream always sends `SymbolInformation`, each with its own
  `Uri`. When the client has `hierarchicalDocumentSymbolSupport` (Zed has it),
  the branch sends `DocumentSymbol`: the HTML tree (`findDocumentSymbols2`), the
  CSS tree (the virtual `__` rule of a style attribute is replaced by its
  children), and the script symbols without children. The modes come in the same
  order as before. `tests/stdio.rs` (`hierarchical_symbols`). Effect on
  `large.html` (`html-symbols.jsonl`): response 13.9 MB against 16.4 MB, server
  time (symbols and serialization) 36 to 40 ms against 40 to 48 ms. The round
  trip in `lsp-replay` is longer (230 to 280 ms against 195 to 245 ms): the
  client parses the nested JSON more slowly. Not measured in Zed.
- Failed settings requests. As in the CSS server: upstream caches a rejected
  `workspace/configuration` request (or a `null` result) until the next
  `didChangeConfiguration`, and completion, hover, formatting and diagnostics
  fail. The branch uses the global settings for that request and does not cache
  the result. `tests/stdio.rs` (`scoped_settings`).
- `html.format.unformatted` that is truthy and not a string. Upstream
  js-beautify throws (`unformattedTags.match`), and formatting answers `[]`. The
  branch ignores the value. `scoped_settings`.
- Format setting names. Upstream js-beautify throws for a `wrapAttributes` or
  `indentScripts` that is not a known name or a list of one known name, and for
  an unknown name in `templating`; formatting answers `[]`. The branch uses the
  default for the first two and leaves unknown `templating` names out.
  `settings::tests::format_options`.
- Languages in a document. Upstream `getLanguagesInDocument` stops at three
  embedded languages and then does not add `html`: a document with CSS,
  JavaScript and TypeScript regions has no HTML mode for links, symbols, colors,
  semantic tokens and diagnostics. The branch has no limit.
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

## Schema loading

Upstream loads a schema and each schema that it references, resolves `$ref` by
merging the target into the referencing object on a JS object graph, and keeps
the merged schema. The port did the same on `schema/graph.rs` and then compiled
the whole graph.

The branch keeps each schema document as a read-only tree (`schema/tree.rs`)
with an index of base URIs, embedded `$id` resources and anchors, made at load
(`schema/doc.rs`). A schema node is compiled on first use, and its `$ref` is
resolved then:

- A `$ref` without sibling keywords, or any `$ref` before draft 2019-09, points
  to its target. A chain stops at a node whose resource has a `$dynamicAnchor`
  or `$recursiveAnchor`, so that the validation enters that resource.
- A `$ref` with sibling keywords is one more `allOf` member, as in draft
  2019-09. Two forms keep the upstream merge result: with only annotation
  siblings (`description`, `default`, ...) the node is the target with the
  annotations that the target does not have (the Zed settings schema has 24);
  with `additionalProperties`, the properties of the target count as its own (VS
  Code settings schemas).
- A document loads when a used `$ref` needs it. The operation then runs again
  (`JsonSchemaService::run`). A `$ref` that a feature does not reach loads
  nothing; the Zed settings schema references 58 `zed://schemas/settings/lsp/`
  documents.
- `json/schemaContent` and a file change reload only that document.
- Object keys keep the document order (no JS key order).

Differences: a schema error is found only when a feature reaches its `$ref`. An
`$id` embedded in a document that only an unused `$ref` loads is not found. The
"can not be resolved" error names the document of the anchor. Upstream trace
tests: 3 of 233 skipped for this (`tests/upstream_trace.rs`, `DIFFERENT`); the
JSON-Schema-Test-Suite passes (4950 cases). `tests/schema_refs.rs`.

Effect, `schema_bench`: Zed settings schema (557 KB) load and validation 13.5
against 5.4 ms, heap 9.3 against 3.3 MB. A generated 2.2 MB schema: validation
45 against 21 ms, heap 27 against 11 MB; the first completion 23 against 43 ms
(it now compiles the 19800 property schemas). The 54 JSON sessions: all requests
8.7 against 4.4 s (`documentColor`, which waits for the loads, 6.7 against 2.6
s), `zed://` content requests 2152 against 80, the sum of peak memory 1412
against 1046 MB. The responses are the same as before, except four
`settings.json` and `package.json` diagnostics, which now equal Node's.

## JSON server

- Equal configuration. Zed sends `workspace/didChangeConfiguration` again with
  the same settings (103 of 142 notifications in 39 sessions). Upstream applies
  each one: it registers all schema associations again (1468 in Zed), drops all
  loaded schemas and sends `workspace/diagnostic/refresh`, so the next
  diagnostic loads and compiles the schema again (25 to 31 ms for the Zed
  settings schema). The branch ignores settings that are equal to the last ones.
  `tests/stdio.rs` (`equal_configuration`). Effect on `session-1794684530299`:
  diagnostics 390 ms against 102 ms (13 against 1 above 10 ms), 14 fewer
  refreshes, the same responses. All 49 JSON sessions: diagnostics 10.2 s
  against 7.8 s, 171 against 280 refreshes; the replays differ only in the
  number of refreshes.
- Schema prefetch. Upstream loads the schema at the first request that needs it,
  and a pull diagnostic waits for the load (900 ms for an HTTP schema). The
  branch queues each document at `didOpen`, at a configuration change and when a
  loaded schema changes. When no message waits, it loads and compiles the schema
  of the next queued document. A pull diagnostic does not wait for schema
  content: it gets the syntax errors without comment and trailing comma
  diagnostics (a schema can allow them). When the schema is loaded and compiled,
  `workspace/diagnostic/refresh` follows. Push diagnostics and the other
  requests still wait for the content. The compile is on the main thread.
  `tests/stdio.rs` (`pull_diagnostics_without_schema`). Effect on the 54 JSON
  sessions: diagnostics 8.8 s against 1.7 s (above 100 ms: 9 against 2; max 1502
  against 160 ms, the parse of a 26 MB document). The replay sends the next
  request at once, so the wait moves to the next request that needs the schema:
  `documentColor` 21 ms against 7.7 s, all requests 9.0 s against 9.6 s. In Zed
  a waiting request does not stop other requests. The replays differ in the
  first diagnostic of a document whose schema is not loaded, and in one more
  refresh for each such load.
- HTTP schema cache. Upstream (the server, as Zed lets it handle `http` and
  `https`) downloads each HTTP schema again at each start: about 900 ms for
  `json.schemastore.org` with its redirect, and nothing without a network. The
  VS Code client has a cache, Zed has none. The branch writes each 2xx response
  with its `ETag` to a cache folder (`%LOCALAPPDATA%`, `~/Library/Caches` or
  `$XDG_CACHE_HOME` / `~/.cache`, then `vscode-langservers-rs/json-schemas`;
  `JSON_SCHEMA_CACHE` sets the folder, an empty value turns the cache off). A
  load with a cached copy reads the file inline and uses it. A worker thread
  then checks the copy with `If-None-Match`: on `304` or a network error the
  copy stays; changed content replaces the cache and reloads the schema as
  `json/schemaContent` does, with a refresh. After a change on the server, the
  requests of the first second use the old copy. `tests/stdio.rs`
  (`http_schema_cache`). Effect on the 54 JSON sessions (one HTTP schema, in 3
  sessions): all requests 4.5 s with no cache, 2.7 s at the first load into an
  empty cache, 1.9 s with the cached copy; `documentColor` 2.7 s against 0.25 s;
  max request 829 against 170 ms. With the cached copy, the replays differ only
  in one fewer refresh in each of the 3 sessions.

## JSON colors

Upstream `findDocumentColors` counts each string with a `color` or `color-hex`
format against the limit, also a string that is not a color, and gives one
color for limit 0. The branch counts colors: at most `limit`, none for 0. The
servers never pass 0 (a 0 setting is the default limit), so only a document
with more strings of a color format than the limit (500 in Zed) can show the
difference. `tests/colors.rs`.
