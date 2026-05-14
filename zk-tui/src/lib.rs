#![deny(missing_docs)]
//! Terminal UI crate for note-zk.

pub mod terminal;

// TG-TUI-003 wires the dependency surface before the modules consume it.
use anyhow as _;
use crossterm as _;
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
#[cfg(feature = "ai")]
use zk_index as _;
use zk_sync as _;
#[cfg(feature = "ai")]
use zk_vault as _;
