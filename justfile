# run 'tokenize' on a single file path
tokenize path:
  @ cargo run --bin cli --quiet -- tokenize {{path}}

# run 'analyze' on a single file path
analyze path:
  @ cargo run --bin cli --quiet -- analyze {{path}}

# run a module via the bytecode VM
run path:
  @ cargo run --bin cli --quiet -- run {{path}}

# format one or more .pa files in place (or `-` for stdin → stdout)
format +paths:
  @ cargo run --bin cli --quiet -- format {{paths}}

# verify that .pa files in the tree are already in canonical format
format-check:
  @ cargo run --bin cli --quiet -- format --check $(find tests/run tests/analyze compiler/src/prelude.pa -name "*.pa")

# run the benchmark suite (VM on benchmarks/programs/*)
bench:
  @ cargo run --release -p bench --quiet

# build the cli in release mode; produces target/release/cli
build-release:
  @ cargo build --release --bin cli

# run the snapshot test suite (analyze + run fixtures under tests/)
test:
  @ cargo test -p tests

# regenerate snapshots for any failing tests (use `cargo insta review` for interactive)
test-write:
  @ INSTA_UPDATE=always cargo test -p tests

# run the TextMate grammar regression tests (vsix/syntaxes/pluma.tmLanguage.json)
test-grammar:
  @ cd vsix && npm test

site:
  @ zola -r site serve -p 7586

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

# build the language server + zed extension wasm; then install the dev
# extension from Zed (cmd palette -> "zed: install dev extension" -> ./zed)
zed-extension:
  cargo build --bin pluma-language-server
  cd zed && cargo build --release --target wasm32-wasip1
  @ echo "Built. In Zed run 'zed: install dev extension' -> $(pwd)/zed (see zed/README.md for settings)."

# install all deps on macos
install-depencies-macos:
  brew install zola
  brew install just