# run on a single file path
run path:
  @ cargo run --quiet {{path}}

# run all tests
test:
  @ scripts/run-tests

# run tests matching filter
test-only filter:
  @ scripts/run-tests {{filter}}

write-analyze-test-output path:
  @ cargo run -q -- analyze {{path}} > {{path}}/analyze.out 2> {{path}}/analyze.err

site:
  @ zola -r site serve -p 7586

# install all deps on macos
install-depencies-macos:
  brew install zola