# run 'analyze' on a single file path
analyze path:
  @ cargo run --quiet -- analyze {{path}}

# run all tests
test:
  @ python3 scripts/run-tests.py

# run tests matching filter
test-only filter:
  @ python3 scripts/run-tests.py {{filter}}

site:
  @ zola -r site serve -p 7586

# install all deps on macos
install-depencies-macos:
  brew install zola
  brew install just