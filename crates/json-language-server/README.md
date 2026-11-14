# json-language-server

Port of the JSON server of [`microsoft/vscode`](https://github.com/microsoft/vscode)
(`extensions/json-language-features/server`, `main`, 1448348). Binary:
`vscode-json-language-server --stdio`.

Schemas load from `file:`, HTTP and the `vscode/content` request (Zed's
`zed://schemas/...`). Push or pull diagnostics, as the client supports.

Differences from upstream: [`docs/differences/json-server.md`](../../docs/differences/json-server.md).
Zed: [`docs/zed.md`](../../docs/zed.md).
