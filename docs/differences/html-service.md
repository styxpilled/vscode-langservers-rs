# HTML service: differences from upstream

Upstream: `vscode-html-languageservice`, scanner, parser, language facts,
completion, path completion, hover, folding, highlights, links, symbols, rename,
linked editing, matching tag, selection ranges. Fixture generator:
`scripts/html/trace.mjs`. The formatter: `html-formatter.md`.

## Not ported

- `__proto__` and integer attribute names: upstream keeps the attributes of a
  node in a JS object, so integer names come first in `attributeNames` and an
  attribute `__proto__` is lost. Rust keeps the source order and all names.
- `isPathAttribute` and the tag map of a data provider read JS objects, so a tag
  name such as `constructor` finds the object prototype upstream (a `TypeError`
  in `isPathAttribute`). Rust has no such names.
- The service is synchronous. `doComplete2` reads directories with a synchronous
  `FileSystemProvider`.

## Changed

- `stream.advance(1)` advances one UTF-16 unit upstream. After a character that
  is not valid in a tag (`<div😀>`), upstream splits a surrogate pair into two
  `Unknown` tokens. Rust advances one character: one `Unknown` token.
- A custom data tag without `attributes` has no attributes. Upstream throws in
  completion and hover.
- A custom data entry with a field of the wrong type (for example
  `"void": "yes"`) is dropped.
- `validateAndCleanURI`: where `URI.parse` throws, upstream returns a link
  without target. Rust returns no link.

## Ported

- `Scanner.scan` advances one character when a state does not advance.
- Folding: `rangeLimit` 0 means no limit.
- Hover: the entity range starts after the `&`.
- Links: `validateRef` uses
  `/\b(w[\w\d+.-]*:\/\/)?[^\s()<>]+(?:\([\w\d]+\)|([^[:punct:]\s]|\/?))/`; the
  last group can match nothing, so a link is valid if a word boundary comes
  before a character that is not white space or `()<>`. `#` alone is not valid.
- Links: a `<base>` without a value (`href=""`) stops the search for a base, and
  an empty base resolves links against the document.
