# Session: Rust Workspace Plan

Date: 2026-04-03

## Outcome

Drafted a Rust-first workspace structure centered on:

- domain
- vault
- index
- sync
- app facade
- server
- CLI

with AI, auth, backup, and worker layers added as dedicated crates when needed.

## Core Recommendation

Avoid both extremes:

- one giant crate forever
- too many micro-crates too early
