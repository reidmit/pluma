# run 'tokenize' on a single file path
tokenize path:
  @ cargo run --bin cli --quiet -- tokenize {{path}}

# run 'analyze' on a single file path
analyze path:
  @ cargo run --bin cli --quiet -- analyze {{path}}

# run a module (defaults to the bytecode VM; `--mode=interp` for the tree walker)
run path:
  @ cargo run --bin cli --quiet -- run {{path}}

# run the benchmark suite (VM vs interpreter on benchmarks/programs/*)
bench:
  @ cargo run --release -p bench --quiet

# build the cli in release mode; produces target/release/cli
build-release:
  @ cargo build --release --bin cli

# run the snapshot test suite (analyze + run fixtures under tests/)
test:
  @ cargo test -p pluma-tests

# regenerate snapshots for any failing tests (use `cargo insta review` for interactive)
test-write:
  @ INSTA_UPDATE=always cargo test -p pluma-tests

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

# install all deps on macos
install-depencies-macos:
  brew install zola
  brew install just