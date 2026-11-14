# vscode-langservers-rs

Developer documentation. Use and build: the `README.md` of the repository.

Zed installs the JSON and CSS servers from `vscode-langservers-extracted`
4.10.0 (May 2024), and the HTML server from its fork
`@zed-industries/vscode-langservers-extracted` 4.10.8 (`zed.md`). The ports
follow upstream `main`.

## Upstream sources

Shallow clones in `upstream/` (not tracked). To update a port, diff upstream
from the commit in this table. The crate READMEs repeat the commit of their
upstream.

| Repository                                                    | Version          | Commit  | Port                          |
| ------------------------------------------------------------- | ---------------- | ------- | ----------------------------- |
| `microsoft/node-jsonc-parser`                                 | 4.0.0-next.2     | de0d2fd | `crates/jsonc`                |
| `microsoft/vscode-json-languageservice`                       | 6.0.0-next.4     | bf6a8b2 | `crates/json-languageservice` |
| `microsoft/vscode` `extensions/json-language-features/server` | main             | 1448348 | `crates/json-language-server` |
| `microsoft/vscode-css-languageservice`                        | 7.0.0-next.2     | 9f0aa50 | `crates/css-languageservice`  |
| `microsoft/vscode` `extensions/css-language-features/server`  | main             | 1448348 | `crates/css-language-server`  |
| `microsoft/vscode-uri`                                        | 3.2.0            |         | `crates/vscode-uri` (parts)   |
| `microsoft/vscode-html-languageservice`                       | 6.0.0-next.2     | 04a0299 | `crates/html-languageservice` |
| `microsoft/vscode` `extensions/html-language-features/server` | main             | 1448348 | `crates/html-language-server` |
| `zed-industries/vscode` `html-tags-as-workspace-edits`        | 4.10.8           |         | tag auto insertion            |
| `json-schema-org/JSON-Schema-Test-Suite`                      |                  | f6fd52a | test data                     |
| `oxc-project/tsrs` (crates.io)                                | 7.1.0-beta.tsrs1 |         | dependency of the HTML server |
| `zed-industries/zed` (sparse)                                 |                  | dc93abc | reference only                |

## Documents

- `design.md`: rules, crate layout, the design of each crate.
- `testing.md`: test methods, fixture generators, sweeps, replays.
- `performance.md`: measurements, benchmarks, profiling, open ideas.
- `zed.md`: what Zed sends and expects.
- `differences/`: each difference from upstream, with its decision.
- `experimental.md`: the changes of the `experimental` branch that do not
  match the JS behavior.

## Roadmap

1. CSS variables server: port of the server of Zed's `css-variables` extension.
   A separate binary: Zed starts it beside the CSS server, and one `binary.path`
   replaces one server. It indexes the workspace; the vscode services work on
   one document. First find the upstream repository and the parser that it uses.
2. Distribution: a Zed extension that downloads release binaries. Until then,
   set `binary.path` by hand.
3. Possible: localization. Upstream messages go through `l10n.t` and the vscode
   l10n bundles. The ports are English only.
