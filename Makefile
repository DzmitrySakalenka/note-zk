.PHONY: check fmt fmt-check lint lint-features test build tui-gate miri clean

NON_CUDA_FEATURES = zk-ai/provider-http zk-ai/provider-local zk-app/ai zk-cli/ai-local zk-cli/test-stub-ai zk-tui/ai-local zk-tui/test-stub-ai zk-tui/clipboard zk-vault/test-helpers zk-server/test-helpers

check: fmt-check build lint lint-features tui-gate test

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

lint-features:
	cargo clippy --workspace --all-targets --features "$(NON_CUDA_FEATURES)" -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --workspace

tui-gate:
	cargo build -p zk-tui
	cargo build -p zk-tui --features ai
	cargo test -p zk-tui
	cargo test -p zk-tui --features ai
	cargo build -p zk-cli-core
	cargo test -p zk-cli-core

miri:
	rustup run nightly cargo miri test -p zk-domain --test miri_smoke

clean:
	cargo clean
