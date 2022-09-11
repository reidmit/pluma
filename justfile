# run on a single file path
run path:
  @ cargo run --quiet {{path}}

# run all tests
test:
  @ python3 scripts/run-tests.py

# run tests matching filter
test-only filter:
  @ python3 scripts/run-tests.py {{filter}}

write-analyze-test-output path:
  @ cargo run -q -- analyze {{path}} > {{path}}/analyze.out 2> {{path}}/analyze.err || true
  @ echo "Wrote {{path}}/analyze.out and {{path}}/analyze.err"

write-run-test-output path:
  @ cargo run -q -- run {{path}} > {{path}}/run.out 2> {{path}}/run.err || true
  @ echo "Wrote {{path}}/run.out and {{path}}/run.err"

site:
  @ zola -r site serve -p 7586

# install all deps on macos
install-depencies-macos:
  brew install zola
  brew install just