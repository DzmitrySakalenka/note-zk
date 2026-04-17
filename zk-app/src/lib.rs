mod bootstrap;
mod bridge;
mod doctor;
mod error;
mod rebuild;
mod scan;
mod status;
mod write;
mod write_policy;

pub mod config;

pub use bootstrap::{
    bootstrap_effective_config, bootstrap_effective_config_from_env, detect_platform,
    resolve_default_global_config_path,
};
pub use bridge::{build_asset_inputs, build_note_inputs};
pub use doctor::{DoctorIssue, DoctorReport, doctor_report};
pub use error::{AppError, Result};
pub use rebuild::{RebuildOptions, RebuildOutcome, rebuild_index};
pub use scan::{ScanOutcome, scan_vault};
pub use status::{StatusSummary, status_summary};
pub use write::create_note;
pub use write_policy::{ensure_writable, is_path_immutable};
