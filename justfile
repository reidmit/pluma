# run on a single file path
run path:
  @ cargo run --quiet {{path}}

# run tests
test:
  @ cargo insta test --review