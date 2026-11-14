#!/bin/sh
# Replays .scratch/data/perf.jsonl with --times and prints a table.
# Usage: scripts/perf.sh [label]   (label "node" runs the Node server)
cd "$(dirname "$0")/.."
REPLAY=target/replay/release/lsp-replay.exe
label=${1:-rust}
if [ "$label" = node ]; then
  cmd="node $LOCALAPPDATA/Zed/languages/json-language-server/node_modules/vscode-langservers-extracted/bin/vscode-json-language-server"
else
  cmd=target/replay/release/vscode-json-language-server.exe
fi
"$REPLAY" --times .scratch/data/perf.jsonl ".scratch/replays/perf-$label.json" -- $cmd --stdio
