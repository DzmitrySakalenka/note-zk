.PHONY: check fmt lint test build clean

check: fmt lint test

fmt:
	cargo fmt --all

lint:
	cargo clippy --workspace -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --workspace

clean:
	cargo clean
