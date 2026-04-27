//! Shared application state and bootstrap helpers for HTTP handlers.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::{ServerError, ServerResult, config::ServerConfig, db::ServerDb};

/// Cheap-to-clone server state shared across HTTP handlers.
#[derive(Clone)]
pub struct ServerState {
    inner: Arc<StateInner>,
}

struct StateInner {
    config: ServerConfig,
    db: ServerDb,
    started_at: DateTime<Utc>,
}

impl ServerState {
    /// Returns the loaded runtime configuration.
    #[must_use]
    pub fn config(&self) -> &ServerConfig {
        &self.inner.config
    }

    /// Returns the shared server database handle.
    #[must_use]
    pub fn db(&self) -> &ServerDb {
        &self.inner.db
    }

    /// Returns the server startup timestamp.
    #[must_use]
    pub fn started_at(&self) -> DateTime<Utc> {
        self.inner.started_at
    }

    fn from_parts(config: ServerConfig, db: ServerDb, started_at: DateTime<Utc>) -> Self {
        Self {
            inner: Arc::new(StateInner {
                config,
                db,
                started_at,
            }),
        }
    }
}

/// Loads config, opens the database, and returns a ready server state.
///
/// # Errors
///
/// Returns [`ServerError`] when config loading or validation fails, or when
/// the database cannot be opened and migrated.
pub fn bootstrap() -> ServerResult<ServerState> {
    let config = ServerConfig::load()?;
    bootstrap_with_config(config)
}

/// Opens a ready server state from one already-prepared config.
///
/// # Errors
///
/// Returns [`ServerError`] when config validation fails, or when the database
/// cannot be opened and migrated.
pub fn bootstrap_with_config(config: ServerConfig) -> ServerResult<ServerState> {
    config.validate().map_err(ServerError::Config)?;
    let database = ServerDb::open(&config.database_path)?;

    Ok(ServerState::from_parts(config, database, Utc::now()))
}
