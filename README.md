# vscode-langservers-rs

Rust ports of the JSON, CSS and HTML language servers of VS Code, as Zed
installs them from `vscode-langservers-extracted`. Each binary replaces the Node
server through Zed's `lsp.<name>.binary.path` setting.

| Server                        | Languages                         |
| ----------------------------- | --------------------------------- |
| `vscode-json-language-server` | JSON, JSONC                       |
| `vscode-css-language-server`  | CSS, SCSS, Less                   |
| `vscode-html-language-server` | HTML, embedded CSS and JavaScript |

The ports follow the upstream `main` branches. Responses are the same as the
Node servers, except for the differences in
[`docs/differences/`](docs/differences). The JavaScript mode of the HTML server
uses [tsrs](https://github.com/oxc-project/tsrs) (TypeScript 7) in place of
TypeScript 5.

## Branches

- `main`: the exact ports. Responses are the same as the Node servers, except
  for the documented differences.
- `experimental`: changes that do not have to match the Node servers. Where an
  upstream behavior is wrong, slow or needs extra logic only for parity, the
  branch picks the correct or the faster behavior. Each change, with its reason
  and measured effect, is in [`docs/experimental.md`](docs/experimental.md). The
  main changes:
  - Result limits when the client gives none (VS Code's values: 5000 symbols and
    folding ranges, 500 colors). Zed gives none.
  - Smaller completion lists (`itemDefaults`, documentation on
    `completionItem/resolve`).
  - Pull diagnostics with `resultId`: an unchanged document gets an `unchanged`
    report. Validation runs while the server waits for the pull.
  - JSON: a disk cache for HTTP schemas (with `ETag`, works offline), lazy
    `$ref` schemas, no schema reload when Zed sends the same settings again.
  - CSS: a formatter that prints the parse tree in place of js-beautify (knows
    SCSS and Less nesting, adds the last `;` as Prettier), no exponential parse
    time on nested input, no quadratic duplicate lint.
  - HTML: embedded CSS and scripts as regions, parsed again only when their text
    changes; nested document symbols.
  - All servers: nesting limits (no stack overflow), answers where upstream
    throws, a slow request log (`VSCODE_LANGSERVERS_SLOW_MS`).

## Speed

Replays of the same sessions with `lsp-replay --times`, release builds on
Windows. Request time is the sum over the session, without `initialize`. Rust:
median of 3 runs; Node: 1 run. HTML without the JavaScript mode (the Node server
is upstream `main` without it).

| Session                                         | Measurement | Node    | `main` | `experimental` |
| ----------------------------------------------- | ----------- | ------- | ------ | -------------- |
| JSON: each feature on 26 and 33 MB files        | requests    | 12.9 s  | 6.8 s  | 1.8 s          |
|                                                 | startup     | 2437 ms | 7 ms   | 7 ms           |
|                                                 | peak memory | 1404 MB | 636 MB | 315 MB         |
|                                                 | end memory  | 841 MB  | 43 MB  | 39 MB          |
| CSS: each feature on the test files (10731 req) | requests    | 6.5 s   | 4.8 s  | 1.7 s          |
|                                                 | startup     | 962 ms  | 6 ms   | 10 ms          |
|                                                 | peak memory | 217 MB  | 25 MB  | 22 MB          |
| HTML: large, minified and style documents       | requests    | 158.2 s | 0.9 s  | 1.0 s          |
|                                                 | startup     | 324 ms  | 9 ms   | 12 ms          |
|                                                 | peak memory | 609 MB  | 119 MB | 120 MB         |
|                                                 | end memory  | 340 MB  | 14 MB  | 15 MB          |

- JSON: `experimental` sends at most 5000 document symbols when the client gives
  no limit: 30 ms in place of 2.5 s (166 MB of JSON) for each large file. The
  other requests take the same time on both branches.
- CSS: completion takes 1.7 s in place of 5.0 s on `experimental` (smaller
  lists).
- HTML: Node spends 154 s in one range format of a one-line document.

Also no Node runtime. More measurements:
[`docs/performance.md`](docs/performance.md).

## Build

Rust 1.99 (`rust-toolchain.toml`).

```sh
cargo build --release
```

The binaries are in `target/release/`. The HTML server without the JavaScript
mode (5.6 MB in place of 26 MB, faster build):

```sh
cargo build --release -p html-language-server --no-default-features
```

## Use in Zed

```json
"lsp": {
  "json-language-server": {
    "binary": { "path": "<repo>/target/release/vscode-json-language-server", "arguments": ["--stdio"] }
  },
  "vscode-css-language-server": {
    "binary": { "path": "<repo>/target/release/vscode-css-language-server", "arguments": ["--stdio"] }
  },
  "vscode-html-language-server": {
    "binary": { "path": "<repo>/target/release/vscode-html-language-server", "arguments": ["--stdio"] }
  }
}
```

On Windows, add `.exe` to each path. The servers also work in other LSP clients
that start them with `--stdio`.

## Crates

| Crate                                                 | Port of                                                       |
| ----------------------------------------------------- | ------------------------------------------------------------- |
| [`jsonc`](crates/jsonc)                               | `microsoft/node-jsonc-parser`                                 |
| [`json-languageservice`](crates/json-languageservice) | `microsoft/vscode-json-languageservice`                       |
| [`json-language-server`](crates/json-language-server) | `microsoft/vscode` `extensions/json-language-features/server` |
| [`css-languageservice`](crates/css-languageservice)   | `microsoft/vscode-css-languageservice`                        |
| [`css-language-server`](crates/css-language-server)   | `microsoft/vscode` `extensions/css-language-features/server`  |
| [`html-languageservice`](crates/html-languageservice) | `microsoft/vscode-html-languageservice`                       |
| [`html-language-server`](crates/html-language-server) | `microsoft/vscode` `extensions/html-language-features/server` |
| [`text-document`](crates/text-document)               | `vscode-languageserver-textdocument`                          |
| [`vscode-uri`](crates/vscode-uri)                     | `microsoft/vscode-uri` (parts)                                |
| [`lsp-core`](crates/lsp-core)                         | transport and main loop of the servers                        |
| [`lsp-recorder`](crates/lsp-recorder)                 | session recorder and replay tool (tests)                      |

## Documentation

[`docs/`](docs): design, tests, performance, Zed facts, upstream commits and
each difference from upstream.

## License

MIT ([`LICENSE`](LICENSE)). The upstream projects are MIT, copyright Microsoft
Corporation.
