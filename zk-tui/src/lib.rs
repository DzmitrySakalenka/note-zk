#![deny(missing_docs)]
#![cfg_attr(test, allow(unused_crate_dependencies))]
//! Ratatui terminal UI for the filesystem-first note-zk knowledge system.
//!
//! The `zk-tui` binary wraps the same bootstrap, sync, scan, doctor, trash,
//! auth, admin, search, chat, and wiki workflows exposed by `zk-cli` in an
//! interactive terminal interface. End-user setup documentation lives in
//! `docs/user-guide/`.

pub mod app;
mod editor;
pub mod event;
pub mod jobs;
pub mod keymap;
pub mod screens;
pub mod terminal;
pub mod theme;
pub mod widgets;

// These dependencies are consumed by the binary or feature-gated integration
// tests, while `unused_crate_dependencies` is evaluated per crate target.
use clap as _;
use futures as _;
use serde as _;
use serde_json as _;
use toml as _;
use tracing_subscriber as _;
use url as _;
