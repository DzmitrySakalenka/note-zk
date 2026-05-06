//! Shared application state and bootstrap helpers for HTTP handlers.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use crate::{
    ServerError, ServerResult,
    ai::ingest::{DEFAULT_INGEST_DEBOUNCE, IngestEnqueuer},
    config::ServerConfig,
    db::ServerDb,
    service::sweep_expired_sessions,
};

/// Cheap-to-clone server state shared across HTTP handlers.
#[derive(Clone)]
pub struct ServerState {
    inner: Arc<StateInner>,
}

struct StateInner {
    config: ServerConfig,
    db: Arc<ServerDb>,
    ai_ingest: Option<Arc<IngestEnqueuer>>,
    started_at: DateTime<Utc>,
    in_flight_backup_id: Mutex<Option<String>>,
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

    /// Returns the shared server database handle for background tasks.
    #[must_use]
    pub(crate) fn db_handle(&self) -> Arc<ServerDb> {
        Arc::clone(&self.inner.db)
    }

    /// Returns the optional AI ingest enqueuer for sync-push note changes.
    #[must_use]
    pub fn ai_ingest(&self) -> Option<Arc<IngestEnqueuer>> {
        self.inner.ai_ingest.as_ref().map(Arc::clone)
    }

    /// Returns the server startup timestamp.
    #[must_use]
    pub fn started_at(&self) -> DateTime<Utc> {
        self.inner.started_at
    }

    /// Returns the shared single-flight backup marker.
    #[must_use]
    pub(crate) fn in_flight_backup_id(&self) -> &Mutex<Option<String>> {
        &self.inner.in_flight_backup_id
    }

    fn from_parts(config: ServerConfig, db: ServerDb, started_at: DateTime<Utc>) -> Self {
        let db = Arc::new(db);
        let ai_ingest = config.ai.as_ref().map(|_| {
            Arc::new(IngestEnqueuer::new(
                Arc::clone(&db),
                DEFAULT_INGEST_DEBOUNCE,
            ))
        });

        Self {
            inner: Arc::new(StateInner {
                config,
                db,
                ai_ingest,
                started_at,
                in_flight_backup_id: Mutex::new(None),
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
    let started_at = Utc::now();
    database.with_conn(|connection| sweep_expired_sessions(connection, started_at))?;

    Ok(ServerState::from_parts(config, database, started_at))
}
