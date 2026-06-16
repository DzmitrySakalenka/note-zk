# Installation

This guide covers the supported ways to get `zk-cli`, `zk-tui`, and `zk-server` onto a workstation or server from this repository.

## Prerequisites

Install Rust with `rustup` and let this repository choose the toolchain from `rust-toolchain.toml`. Do not hard-code a Rust version in local scripts; the checked-in toolchain file is the source of truth.

Client builds do not need a system SQLite package because the workspace uses `rusqlite` with the `bundled` feature. Server operators who use encrypted backups may also want the optional `age` command-line tool for managing recipients and identities, although the Rust server contains its own backup encryption implementation.

## Build from source

Clone the repository, run the full local gate, then build release binaries:

```bash
git clone <repo-url> note-zk
cd note-zk
make check
cargo build --release --workspace
```

The binaries are written under `target/release/`. Add that directory to `PATH` or copy the specific binaries you need into an existing `PATH` directory.

## Install with cargo

From the repository root, install each binary directly from its workspace crate:

```bash
cargo install --path zk-cli
cargo install --path zk-tui
cargo install --path zk-server
```

Build the TUI with AI screens when you want search, chat, and wiki views:

```bash
cargo install --path zk-tui --features ai
cargo install --path zk-tui --features ai-local
```

The `ai-local` feature enables local AI wiring and depends on platform support for local model runtimes. See the [LLM Execution Model](../architecture/llm-execution-model.md) for connected, offline, and hybrid behavior.

Local Candle embeddings use CPU unless the binary is built with a hardware
backend and the config selects that device. On Apple Silicon, build the binaries
that run local embeddings with the Metal feature:

```bash
cargo build --release -p zk-server --features zk-ai/provider-local-metal
cargo build --release -p zk-cli --features ai-local,zk-ai/provider-local-metal
cargo build --release -p zk-tui --features ai-local,zk-ai/provider-local-metal
```

CUDA builds use the matching `zk-ai/provider-local-cuda` feature instead.
OpenAI-compatible local servers such as Ollama do not need these feature flags;
they run inference outside note-zk and are configured as HTTP providers.

## Supported terminals

Use a Truecolor-capable terminal for the default and high-contrast themes. The `mono` theme avoids colour and works as the ANSI 16-colour fallback for limited terminals.

SSH and tmux are supported when the remote terminal advertises the right capabilities. The TUI uses the terminal alt-screen, so closing an SSH or tmux session abruptly can leave the shell in a confusing state until the terminal is reset.

On Windows, use Windows Terminal. The legacy PowerShell console is best-effort because terminal feature support varies by host version.

## Verify the install

Check that each binary is on `PATH` and prints a version:

```bash
zk-cli --version
zk-tui --version
zk-server --version
```

Last updated: 2026-05-25
