# CSS links, module resolution, path completion: differences from upstream

Upstream: `vscode-css-languageservice`, the file system parts of
`cssNavigation.ts`, `scssNavigation.ts`, `pathCompletion.ts`,
`utils/resources.ts`, `doComplete2`; `vscode-uri`. Fixture generator:
`scripts/css/trace-links.mjs`.

## Not ported

- `importAliases`: `target in settings` uses `in`, so the `Object.prototype`
  names (`constructor`, `toString`, ...) match; `joinPath` then throws on the
  function value. Rust: only the keys of the settings.
- `pkg:` subpath patterns: `entry.replace('*', match[1])` applies the JS
  replacement patterns (`$&`, `` $` ``, `$'`, `$$`) of the captured text. Rust
  inserts the text as it is.
- A quoted link or path value with a non-BMP last character: `slice(1, -1)` cuts
  the surrogate pair. Rust removes the full character.

## Changed

- `FileSystemProvider`: `readDirectory` and `getContent` are optional upstream;
  Rust requires the three functions (the server has them). `None` is a throw.

## Ported

- `main` resolves module references for CSS and Less too
  (`resolveModuleReferences` is `true` for all dialects): plain CSS `@import`
  and bare module names resolve in `node_modules`.
- `~` uses module resolution only with a file system provider, and not for `~/`.
  Module resolution only for `file://` documents: the `node_modules` folders
  from the document folder up to the root folder (`resolveReference('/')`).
- `fileExists`: false for a stat that throws, and for type `Unknown` with size
  -1.
- An empty reference (`""`) from `resolveReference` is returned as the target,
  but it does not count as found.
- `importAliases` folder alias: the prefix is the target up to the first `/`
  (without a `/`, the prefix is `/`), the alias value loses its last character,
  and the rest of the target starts at the `/`.
- `pkg:`: the subpath entries use `sass`, `styles`, `default` (not `style`); the
  root uses `sass`, `style`, `default`. Only the first `.scss` is removed from
  the subpath and from a pattern, not the suffix. A `default` entry must end
  with `.scss`. An empty `package.json` gives no link; invalid JSON too.
- `toPathVariations` uses `toString(true)`: drive letters become lowercase,
  `%20` becomes a space.
- Path completion: the range arithmetic uses UTF-16 characters of the position,
  also across lines; a negative `slice` end counts from the end. Names that
  start with `.` are skipped; a file with the URI of the document is skipped.
  SCSS imports of `_x.scss`: the edit text is `x`, the label stays `_x.scss`.
  `.` and `..` make the list incomplete.
