# html-language-server

Port of the HTML server of
[`microsoft/vscode`](https://github.com/microsoft/vscode)
(`extensions/html-language-features/server`, `main`, 1448348), with the tag auto
insertion of Zed's fork. Binary: `vscode-html-language-server --stdio`.

Embedded CSS uses `css-languageservice`. The JavaScript mode (feature
`javascript`, on by default) uses [tsrs](https://github.com/oxc-project/tsrs)
(TypeScript 7.1) in place of TypeScript 5. Build without it:

```sh
cargo build --release -p html-language-server --no-default-features
```

Differences from upstream:
[`docs/differences/html-server.md`](../../docs/differences/html-server.md). Zed:
[`docs/zed.md`](../../docs/zed.md).
