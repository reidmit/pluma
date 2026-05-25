#!/usr/bin/env bash
# Regenerate the parser and parse every valid Pluma program in the repo's
# tests/run/ corpus, asserting zero ERROR/MISSING nodes. These fixtures are
# known-good (they compile and run), so any parse error here is a grammar
# regression. (tests/analyze/ is skipped — it intentionally includes malformed
# sources to exercise the compiler's diagnostics.)
#
# This mirrors the snapshot test for the TextMate/Sublime grammars: the real
# syntax lives in compiler/src/tokenizer.rs + parser.rs, and this is what keeps
# the hand-written grammar from drifting away from it.
set -euo pipefail

cd "$(dirname "$0")/.."
TS=./node_modules/.bin/tree-sitter

"$TS" generate

fixtures_dir="../tests/run"
fail=0
count=0
for f in "$fixtures_dir"/*/main.pa; do
  count=$((count + 1))
  if ! "$TS" parse -q "$f" >/dev/null 2>&1; then
    fail=$((fail + 1))
    echo "PARSE ERROR: $f"
    "$TS" parse "$f" 2>&1 | grep -iE 'error|missing' | head -3 | sed 's/^/    /'
  fi
done

if [ "$fail" -ne 0 ]; then
  echo "FAILED: $fail of $count tests/run fixtures have parse errors"
  exit 1
fi
echo "OK: all $count tests/run fixtures parse cleanly"
