# run 'tokenize' on a single file path
tokenize path:
  @ cargo run --bin pluma --quiet -- tokenize {{path}}

# run 'analyze' on a single file path
analyze path:
  @ cargo run --bin pluma --quiet -- analyze {{path}}

# run a module on V8 (compiles to WasmGC — the deploy engine)
run path:
  @ cargo run --bin pluma --quiet -- run {{path}}

# compile a module to a WasmGC deploy artifact (run it with `pluma run <out>.wasm`)
build-server path:
  @ cargo run --bin pluma --quiet -- build {{path}}

# lint a .pa file (report warnings; exits non-zero if any fire)
lint path:
  @ cargo run --bin pluma --quiet -- lint {{path}}

# format everything: Rust sources (cargo fmt) + the baked-in stdlib/prelude .pa sources
format: format-stdlib
  @ cargo fmt

# verify that .pa files in the tree are already in canonical format
format-check:
  @ cargo run --bin pluma --quiet -- format --check $(find tests/run tests/analyze compiler/src/prelude.pa -name "*.pa")

# format the baked-in stdlib + prelude .pa sources in place (modules,
# their *.test.pa suites, prelude, and the stdlib package marker)
format-stdlib:
  @ cargo run --bin pluma --quiet -- format $(find compiler/src/stdlib compiler/src/prelude.pa -name "*.pa")

# build the cli in debug mode; produces target/debug/pluma
build:
  @ cargo build --bin pluma

# build the cli in release mode; produces target/release/pluma
build-release:
  @ cargo build --release --bin pluma

# run the snapshot test suite (analyze + run + format fixtures under tests/).
# `run` compiles each fixture to WasmGC and runs it under V8 (the deploy engine).
# Uses cargo-nextest when present — it pools every fixture across all cores
# instead of running the test binaries one at a time, which is ~13x faster on
# this V8-heavy corpus (≈190s -> ≈15s). Falls back to the builtin runner with an
# install hint when nextest isn't installed. (The workspace has no doctests, so
# nextest skips nothing.)
test:
  @ if command -v cargo-nextest >/dev/null 2>&1; then cargo nextest run -p tests; else echo "tip: 'cargo install cargo-nextest' for a ~13x faster run; using the builtin runner"; cargo test -p tests; fi

# regenerate snapshots for any failing tests (use `cargo insta review` for interactive)
test-write:
  @ if command -v cargo-nextest >/dev/null 2>&1; then INSTA_UPDATE=always cargo nextest run -p tests; else INSTA_UPDATE=always cargo test -p tests; fi

# run the stdlib's own Pluma test suite (compiler/src/stdlib/*.test.pa)
# through `pluma test` — exercises the stdlib and the `std.test` runner under V8.
test-stdlib:
  @ cargo run --bin pluma --quiet -- test compiler/src/stdlib

# run the editor-grammar regression tests: TextMate (vsix/) + Tree-sitter
# (tree-sitter/: corpus tests + parse every tests/run fixture)
test-grammar:
  @ cd vsix && npm test
  @ cd tree-sitter && ./node_modules/.bin/tree-sitter test && ./script/test.sh

# build & run the vscode extension in a new window for local testing
vs-extension:
  cargo build --bin pluma
  rm -rf vsix/dist
  vsix/node_modules/.bin/esbuild vsix/src/extension.ts \
    --outdir=vsix/dist \
    --sourcemap \
    --platform=node \
    --format=cjs
  SERVER_PATH=$(pwd)/target/debug/pluma \
    code --extensionDevelopmentPath=$(pwd)/vsix ./tests

# build the cli (which hosts `pluma language-server`) + zed extension wasm; then
# install the dev extension from Zed (cmd palette -> "zed: install dev extension" -> ./zed)
zed-extension:
  cargo build --bin pluma
  cd zed && cargo build --release --target wasm32-wasip1
  @ echo "Built. In Zed run 'zed: install dev extension' -> $(pwd)/zed (see zed/README.md for settings)."

# install all deps on macos
install-depencies-macos:
  brew install just