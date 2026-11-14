# lsp-recorder

Test tools for the servers.

- `lsp-recorder --out <file or folder> -- <server command>`: runs the server,
  copies stdio and writes each message as a JSON line. Use it as the server
  path in the editor to record a session.
- `lsp-replay <session> <out.json> -- <server command>`: sends the client
  messages of a session to a server and writes the results.
  `lsp-replay --diff <a.json> <b.json>` compares two results.

Usage: [`docs/testing.md`](../../docs/testing.md).
