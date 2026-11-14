# JSON server: differences from upstream

Upstream: `microsoft/vscode` `extensions/json-language-features/server`,
`jsonServer.ts`, `utils/validation.ts`. Tests:
`crates/json-language-server/tests/stdio.rs`,
`crates/json-languageservice/tests/schema_reset.rs`.

## Not ported

- `resolveExternalLink` records the dependency only on the handle of the nearest
  `$id`, so `onResourceChange` of a referenced schema does not clear a root
  schema with an `$id` (for example Zed's `package_json`): the "Loading" error
  stays. Rust: also on the root handle that is being resolved.
- `http.proxyStrictSSL` = false disables certificate checks for all HTTPS
  requests, and false is the default. Rust: only when `http.proxy` is set.
- Without `customCapabilities.rangeFormatting.editLimit` there is no edit limit:
  formatting a 25 MB file gives a 144 MB response that Zed cannot use. Rust:
  10000 (matching VS Code).

## Changed

- HTTP and `vscode/content` content stays cached until `json/schemaContent` or
  `workspace/didChangeWatchedFiles` names the URI. Upstream fetches again after
  each configuration change.
- `configure` builds a new schema service, so the graph is freed. Upstream
  `clearExternalSchemas` keeps the resolved contribution schemas.
- HTTP requests time out after 60 s. Network error texts come from `ureq`.
- Cancellation: before a request runs, the server looks ahead in the received
  messages for its `$/cancelRequest`. A running request is not cancelled.
- Parsed documents are cached for each open document. Upstream keeps at most 10,
  for 60 s.
- `json.trustedDomains` and `json.schemaDownload.*` are client settings in VS
  Code; the server does not read them.
- `SchemaRequestError.code` is `i64`: the client error codes are negative, and
  upstream adds them to `0x10000`.
- The initialize result has `serverInfo`. Formatter registrations use fixed IDs.
- `shutdown` before `initialize`: Rust follows the LSP specification (exit code
  1).
