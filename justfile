# run 'tokenize' on a single file path
tokenize path:
  @ cargo run --bin cli --quiet -- tokenize {{path}}

# run 'analyze' on a single file path
analyze path:
  @ cargo run --bin cli --quiet -- analyze {{path}}

# run a module on V8 (compiles to WasmGC — the deploy engine)
run path:
  @ cargo run --bin cli --quiet -- run {{path}}

# compile a module to a WasmGC deploy artifact (run it with `pluma run <out>.wasm`)
build-server path:
  @ cargo run --bin cli --quiet -- build --target server {{path}}

# format everything: Rust sources (cargo fmt) + the baked-in stdlib/prelude .pa sources
format: format-stdlib
  @ cargo fmt

# verify that .pa files in the tree are already in canonical format
format-check:
  @ cargo run --bin cli --quiet -- format --check $(find tests/run tests/analyze compiler/src/prelude.pa -name "*.pa")

# format the baked-in stdlib + prelude .pa sources in place (modules,
# their *.test.pa suites, prelude, and the stdlib package marker)
format-stdlib:
  @ cargo run --bin cli --quiet -- format $(find compiler/src/stdlib compiler/src/prelude.pa -name "*.pa")

# build the cli in release mode; produces target/release/cli
build-release:
  @ cargo build --release --bin cli

# run the snapshot test suite (analyze + run + format fixtures under tests/).
# `run` compiles each fixture to WasmGC and runs it under V8 (the deploy engine).
test:
  @ cargo test -p tests

# regenerate snapshots for any failing tests (use `cargo insta review` for interactive)
test-write:
  @ INSTA_UPDATE=always cargo test -p tests

# run the stdlib's own Pluma test suite (compiler/src/stdlib/*.test.pa)
# through `pluma test` — exercises the stdlib and the `core.test` runner under V8.
test-stdlib:
  @ cargo run --bin cli --quiet -- test compiler/src/stdlib

# run the editor-grammar regression tests: TextMate (vsix/) + Tree-sitter
# (tree-sitter/: corpus tests + parse every tests/run fixture)
test-grammar:
  @ cd vsix && npm test
  @ cd tree-sitter && ./node_modules/.bin/tree-sitter test && ./script/test.sh

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