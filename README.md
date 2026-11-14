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

## Why

| Measurement                | Rust        | Node          |
| -------------------------- | ----------- | ------------- |
| JSON startup               | 3 to 16 ms  | 140 to 160 ms |
| JSON memory (Zed sessions) | 12 to 31 MB | 73 to 140 MB  |
| CSS startup                | 5 to 16 ms  | 170 to 185 ms |
| CSS memory (Zed sessions)  | 10 to 13 MB | 80 to 115 MB  |
| HTML startup               | 8 ms        | 363 ms        |
| HTML memory, large files   | 14 MB       | 348 MB        |

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
| [`lsp-transport`](crates/lsp-transport)               | stdio transport of the servers                                |
| [`lsp-recorder`](crates/lsp-recorder)                 | session recorder and replay tool (tests)                      |

## Documentation

[`docs/`](docs): design, tests, performance, Zed facts, upstream commits and
each difference from upstream.

## License

MIT ([`LICENSE`](LICENSE)). The upstream projects are MIT, copyright Microsoft
Corporation.
