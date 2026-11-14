#!/bin/sh
# Runs all CSS sweeps and robustness tests (release). Usage: sh scripts/css-sweeps.sh [--no-completion]
D="$(pwd -W 2>/dev/null || pwd)/.scratch/data"
T="cargo test --release --target-dir target/replay -p css-languageservice --test"
run() { echo "== $*"; "$@" 2>&1 | grep -E "^test result|FAILED|panicked|differ|error" ; }
CSS_PARSER_SWEEP="$D/css-parser-sweep.json" run $T parser upstream_sweep
CSS_PARSER_SWEEP="$D/css-nested-sweep.json" run $T parser upstream_sweep
CSS_FACTS_SWEEP="$D/css-facts-sweep.json" run $T language_facts upstream_sweep
CSS_LINT_SWEEP="$D/css-lint-sweep.json" run $T lint upstream_sweep
CSS_HOVER_SWEEP="$D/css-hover-sweep.json" run $T hover upstream_sweep
CSS_NAVIGATION_SWEEP="$D/css-navigation-sweep.json" run $T navigation upstream_sweep
CSS_FORMAT_SWEEP="$D/css-format-sweep.json" run $T formatter upstream_sweep
CSS_LINKS_SWEEP="$D/css-links-sweep.json" run $T links upstream_sweep
CSS_SWEEP_INPUTS="$D/css-sweep-inputs.json" run $T navigation robustness
CSS_SWEEP_INPUTS="$D/css-sweep-inputs.json" run $T formatter robustness
CSS_SWEEP_INPUTS="$D/css-sweep-inputs.json" run $T links robustness
[ "$1" = "--no-completion" ] || CSS_COMPLETION_SWEEP="$D/css-completion-sweep.json" run $T completion upstream_sweep
