#!/bin/sh
# Replays each session to the Node and the Rust server and diffs the results.
# Sessions in a `css` or `html` folder go to the CSS or HTML servers, others to
# the JSON servers.
# The Node CSS server runs with the upstream `main` language service
# (node-main-server.cjs); CSS_NODE=zed runs the Zed install (4.10.0) as is.
# The Node HTML server is Zed's 4.10.8 with the upstream `main` services and
# without the JavaScript mode (scripts/html/node-main-server.cjs); the Rust HTML
# server is then the build without the `javascript` feature
# (target/replay-nojs). HTML_JS=1 keeps the JavaScript mode in both.
# Usage: scripts/replay-all.sh [session.jsonl ...]
cd "$(dirname "$0")/.."
REPLAY=target/replay/release/lsp-replay.exe
NODE_BIN="$LOCALAPPDATA/Zed/languages/vscode-css-language-server/node_modules/vscode-langservers-extracted/bin"
[ $# -eq 0 ] && set -- .scratch/sessions/*.jsonl .scratch/sessions/css/*.jsonl .scratch/sessions/html/*.jsonl
for s in "$@"; do
  case "$s" in
    */css/*) kind=css; name=css-$(basename "$s" .jsonl) ;;
    */html/*) kind=html; name=html-$(basename "$s" .jsonl) ;;
    *) kind=json; name=$(basename "$s" .jsonl) ;;
  esac
  RUST=target/replay/release/vscode-$kind-language-server.exe
  [ $kind = html ] && [ "$HTML_JS" != 1 ] && RUST=target/replay-nojs/release/vscode-html-language-server.exe
  echo "== $name"
  NODE_SERVER="$NODE_BIN/vscode-$kind-language-server"
  [ $kind = css ] && [ "$CSS_NODE" != zed ] && NODE_SERVER=scripts/css/node-main-server.cjs
  [ $kind = html ] && NODE_SERVER=scripts/html/node-main-server.cjs
  "$REPLAY" "$s" ".scratch/replays/$name-node.json" -- node "$NODE_SERVER" --stdio
  "$REPLAY" "$s" ".scratch/replays/$name-rust.json" -- "$RUST" --stdio
  "$REPLAY" --diff ".scratch/replays/$name-node.json" ".scratch/replays/$name-rust.json"
done
