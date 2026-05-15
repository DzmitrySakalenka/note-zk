#![deny(missing_docs)]
#![cfg_attr(test, allow(unused_crate_dependencies))]
//! Terminal UI crate for note-zk.

pub mod app;
mod editor;
pub mod event;
pub mod jobs;
pub mod keymap;
pub mod screens;
pub mod terminal;
pub mod theme;
pub mod widgets;

// TG-TUI-003 wires the dependency surface before the modules consume it.
use anyhow as _;
use clap as _;
use crossterm as _;
use futures as _;
use ratatui as _;
use serde as _;
use serde_json as _;
use tokio as _;
use toml as _;
use tracing as _;
use tracing_subscriber as _;
use url as _;
#[cfg(feature = "ai")]
use zk_ai as _;
use zk_app as _;
use zk_cli_core as _;
use zk_domain as _;
use zk_index as _;
use zk_sync as _;
use zk_vault as _;
