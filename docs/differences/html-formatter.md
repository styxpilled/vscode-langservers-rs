# HTML formatter: differences from upstream

Upstream: `vscode-html-languageservice`, `htmlFormatter.ts`,
`beautify/beautify-html.js` (js-beautify 2.0.3). CSS content goes to the
style sheet printer (`experimental.md`, "CSS formatter"). Sweep generator:
`scripts/html/trace-format.mjs`.

## Not ported

- The numbers go through `parseInt`. Rust takes integers.
- A value outside the list of `wrapAttributes`, `indentScripts`, `templating` or
  `css.braceStyle` throws upstream. Rust does not deserialize the configuration.
- An empty `templating` list throws upstream. Rust: no templating.
- If the first token is unknown (`{` with Django and Smarty templating) and a
  tag follows it, `last_token` is `null` and upstream throws a `TypeError`
  (`{<div></div>`). Rust keeps the start token as the previous token.
- `InputScanner.next()` reads one UTF-16 unit, so an unknown token can be half
  of a surrogate pair. Rust reads one character. The output text is the same.
- The end tag of unformatted content is the regular expression
  `'</' + tag_name + '[\n\r\t ]*?>'` with the `i` flag, so the regular
  expression characters in a tag name of `unformatted` or `contentUnformatted`
  change the pattern. Rust compares the tag name as text, ignoring case (Unicode
  lowercase).

### Dead code

- `eol` other than `\n`, `indent_level`, `disabled`, raw output,
  `remove_indent`, `ensure_empty_line_above`: `htmlFormatter.ts` does not use
  them.
- The clamp of the range end to the document end: the range always comes from
  `positionAt`.

## Ported

- `js_beautify` returns its input: the content of a `<script>` keeps its line
  breaks, only the first line gets the indent.
- `templating: true` is `django`, `erb`, `handlebars` and `php`. `false` or no
  value is no templating. A list is used as it is; `['auto']` is the same as
  `true`.
- `<style>` options: `_mergeOpts(options, 'css')` copies each field of `css`
  over the HTML options, also when the field is not set. So with `css` set,
  `preserveNewLines` and `maxPreserveNewLines` of the HTML options have no
  effect on `<style>` content.
- The ignore directives of `<style>` content match
  `/\*\s*beautify(\s+\w+:\w+)+\s*\*/` (the CSS service:
  `/* beautify( \w+:\w+)+ */`).
- Range formatting: the range extends to the enclosing
  `<!-- beautify ignore:start -->` and `<!-- beautify ignore:end -->` block. If
  the text before the range has a `<` with no `>` after it, the edit replaces
  the range with the same text.
- `endWithNewline` applies only if the range includes the document end.
- `unformattedContentDelimiter`: the first `\r`, `\n` and `\t` escapes become
  the characters.
- `unformatted`, `contentUnformatted` and `extraLiners` are split at `,`,
  trimmed and lowercased.
