# run 'tokenize' on a single file path
tokenize path:
  @ cargo run --bin cli --quiet -- tokenize {{path}}

# run 'analyze' on a single file path
analyze path:
  @ cargo run --bin cli --quiet -- analyze {{path}}

# generate or overwrite .err and .out files for a given test
test-write $path_to_write:
  #!/usr/bin/env zsh
  if [[ ${path_to_write[-8,-1]} == "/analyze" ]]; then
    test_name=${path_to_write%/analyze}
    cargo run --bin cli --quiet -- analyze "tests/$test_name" > "tests/$test_name/analyze.out" 2> "tests/$test_name/analyze.err" || true
    echo "wrote tests/$test_name/analyze.out and tests/$test_name/analyze.err"
  elif [[ ${path_to_write[-4,-1]} == "/run" ]]; then
    test_name=${path_to_write%/run}
    cargo run --bin cli --quiet -- run "tests/$test_name" > "tests/$test_name/run.out" 2> "tests/$test_name/run.err" || true
    echo "wrote tests/$test_name/run.out and tests/$test_name/run.err"
  else
    echo "invalid arg; provide a path like test-name/analyze or test-name/run"
  fi

site:
  @ zola -r site serve -p 7586

test:
  @ python3 scripts/test.py

# build & run the vscode extension in a new window for local testing
vs-extension:
  cargo build --bin pluma-language-server
  rm -rf vsix/dist
  vsix/node_modules/.bin/esbuild vsix/src/extension.ts \
    --outdir=vsix/dist \
    --sourcemap \
    --platform=node \
    --format=cjs
  SERVER_PATH=$(pwd)/target/debug/pluma-language-server \
    code --extensionDevelopmentPath=$(pwd)/vsix ./tests

# install all deps on macos
install-depencies-macos:
  brew install zola
  brew install just