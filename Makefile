.PHONY: check fmt fmt-check lint test build clean

check: fmt-check build lint test

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --workspace

clean:
	cargo clean
