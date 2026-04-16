mod bootstrap;
mod error;

pub mod config;

pub use bootstrap::{
    bootstrap_effective_config, bootstrap_effective_config_from_env, detect_platform,
    resolve_default_global_config_path,
};
pub use error::{AppError, Result};
