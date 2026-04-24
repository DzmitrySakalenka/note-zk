#![warn(missing_docs)]
#![doc = "HTTP server for the filesystem-first knowledge system. Hosts sync endpoints and supporting services."]

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod http;
pub mod migration;
pub mod repo;
mod schema;
pub mod service;
pub mod state;

pub use error::{ServerError, ServerResult};

/// Loads config, initializes tracing, bootstraps state, and runs the server.
///
/// # Errors
///
/// Returns [`ServerError`] when config loading, tracing setup, bootstrap, or
/// HTTP serving fails.
pub fn run() -> ServerResult<()> {
    let config = config::ServerConfig::load()?;
    initialize_tracing(&config.log_filter)?;

    let state = state::bootstrap_with_config(config)?;
    warn_if_auth_disabled(&state)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(wrap_internal_error)?;

    runtime.block_on(http::serve(state))
}

fn initialize_tracing(log_filter: &str) -> ServerResult<()> {
    let env_filter =
        tracing_subscriber::EnvFilter::try_new(log_filter).map_err(wrap_internal_error)?;

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(ServerError::Internal)?;

    Ok(())
}

fn warn_if_auth_disabled(state: &state::ServerState) -> ServerResult<()> {
    if state.config().stub_auth_token_sha256_hex.is_none() && !auth::device_tokens_exist(state)? {
        tracing::warn!("auth disabled: no stub token configured and no device_tokens rows present");
    }

    Ok(())
}

fn wrap_internal_error(error: impl std::error::Error + Send + Sync + 'static) -> ServerError {
    ServerError::Internal(Box::new(error))
}
