mod bootstrap;
mod bridge;
mod error;
mod rebuild;
mod scan;
mod status;

pub mod config;

pub use bootstrap::{
    bootstrap_effective_config, bootstrap_effective_config_from_env, detect_platform,
    resolve_default_global_config_path,
};
pub use bridge::{build_asset_inputs, build_note_inputs};
pub use error::{AppError, Result};
pub use rebuild::{RebuildOptions, RebuildOutcome, rebuild_index};
pub use scan::{ScanOutcome, scan_vault};
pub use status::{StatusSummary, status_summary};
