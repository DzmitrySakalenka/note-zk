#![warn(missing_docs)]
#![doc = "HTTP server for the filesystem-first knowledge system. Hosts sync endpoints and supporting services."]

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod http;
pub mod migration;
pub mod repo;
pub mod service;
pub mod state;

pub use error::{ServerError, ServerResult};

/// Temporary binary shim until TG12-019 wires the real runtime entry point.
pub fn placeholder_main() {
    // TODO: TG12-019 replace the placeholder entry point with `run()`.
}
