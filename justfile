# run on a single file path
run path:
  @ cargo run --quiet {{path}}

# run tests
test:
  @ cargo insta test --review

site:
  @ zola -r site serve

# install all deps on macos
install-depencies-macos:
  brew install zola