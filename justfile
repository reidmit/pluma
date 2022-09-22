# run 'analyze' on a single file path
analyze path:
  @ cargo run --quiet -- analyze {{path}}

# run all tests
test:
  @ python3 scripts/run-tests.py

# run tests matching filter
test-only filter:
  @ python3 scripts/run-tests.py {{filter}}

# generate or overwrite .err and .out files for a given test
test-write $path_to_write:
  #!/usr/bin/env zsh
  if [[ ${path_to_write[-8,-1]} == "/analyze" ]]; then
    test_name=${path_to_write%/analyze}
    cargo run -q -- analyze "tests/$test_name" > "tests/$test_name/analyze.out" 2> "tests/$test_name/analyze.err" || true
    echo "wrote tests/$test_name/analyze.out and tests/$test_name/analyze.err"
  elif [[ ${path_to_write[-4,-1]} == "/run" ]]; then
    test_name=${path_to_write%/run}
    cargo run -q -- run "tests/$test_name" > "tests/$test_name/run.out" 2> "tests/$test_name/run.err" || true
    echo "wrote tests/$test_name/run.out and tests/$test_name/run.err"
  else
    echo "invalid arg; provide a path like test-name/analyze or test-name/run"
  fi

site:
  @ zola -r site serve -p 7586

# install all deps on macos
install-depencies-macos:
  brew install zola
  brew install just