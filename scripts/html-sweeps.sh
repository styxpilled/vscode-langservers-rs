#!/bin/sh
# Runs all HTML sweeps and robustness tests (release). Usage: sh scripts/html-sweeps.sh
D="$(pwd -W 2>/dev/null || pwd)/.scratch/data"
T="cargo test --release --target-dir target/replay -p html-languageservice --test"
run() { echo "== $*"; "$@" 2>&1 | grep -E "^test result|FAILED|panicked|differ|error" ; }
HTML_FORMAT_SWEEP="$D/html-format-sweep.json" run $T formatter
HTML_SWEEP="$D/html-sweep.json" run $T sweep
