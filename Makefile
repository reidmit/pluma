.PHONY: check
check:
	cargo check

.PHONY: build-debug
build-debug:
	cargo build --bin cli

.PHONY: build-release
build-release:
	cargo build --release --bin cli

.PHONY: run
run:
	@ cargo run --quiet --bin cli